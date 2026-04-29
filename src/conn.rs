//! Network monitor: listen on a local port that the SSH child
//! forwards back to us, periodically open a connection through
//! the tunnel and verify a round-trip payload. Returns 1 on
//! success, 0 on any failure — ssh_watch uses 0 as the "tunnel
//! is dead, restart" trigger.
//!
//! This is the HAVE_ADDRINFO branch only; the legacy
//! gethostbyname() variant in autossh.c was Solaris-era plumbing
//! that doesn't ship on any current Linux glibc.

#![allow(unused_assignments)]

use libc::{
    c_char, c_int, c_long, c_void, accept, addrinfo, bind,
    close, connect, freeaddrinfo, getaddrinfo, gai_strerror, getpid, listen,
    nfds_t, pollfd, read, setsockopt, shutdown, size_t, sockaddr, socket,
    socklen_t, ssize_t, strerror, write, AF_INET, AI_PASSIVE, EAGAIN, EINTR,
    IPPROTO_TCP, POLLIN, POLLOUT, SHUT_RDWR, SOCK_STREAM, SOL_SOCKET,
    SO_REUSEADDR,
};
use std::ptr;

/// Magic flag for echo-mode (no read socket).
const NO_RD_SOCK: c_int = -2;
/// Maximum number of conn-test attempts before giving up.
const MAX_CONN_TRIES: c_int = 3;
/// Bytes appended to the test payload (max 64 + …).
const MAX_MESSAGE: usize = 64;

extern "C" {
    static mut net_timeout: c_int;
    static mut echo_message: *mut c_char;
    static __progname: *const c_char;

    fn errlog(level: c_int, fmt: *const c_char, ...);
    fn xerrlog(level: c_int, fmt: *const c_char, ...);

    /// Required for the test payload's random ID; declared in
    /// stdlib.h but not exposed by libc 0.2 on Linux.
    fn random() -> c_long;

    /// uname() — fills utsname struct. Used in payload's nodename.
    fn uname(buf: *mut libc::utsname) -> c_int;
}

/// Convert host/port to addrinfo.
unsafe fn conn_addr(host: *const c_char, port: *const c_char) -> *mut addrinfo {
    let mut hints: addrinfo = std::mem::zeroed();
    hints.ai_family = AF_INET;
    hints.ai_socktype = SOCK_STREAM;
    hints.ai_protocol = IPPROTO_TCP;
    hints.ai_flags |= AI_PASSIVE;

    let mut res: *mut addrinfo = ptr::null_mut();
    let err = getaddrinfo(host, port, &hints, &mut res);
    if err != 0 {
        xerrlog(libc::LOG_ERR,
            c"%s".as_ptr(),
            gai_strerror(err));
    }
    res
}

#[no_mangle]
pub unsafe extern "C" fn conn_remote(
    host: *const c_char,
    port: *const c_char,
) -> c_int {
    // Cache the address info — matches C's `static struct addrinfo *res`.
    static mut RES: *mut addrinfo = ptr::null_mut();
    if RES.is_null() {
        RES = conn_addr(host, port);
    }
    let res = &*RES;

    let sock = socket(res.ai_family, res.ai_socktype, res.ai_protocol);
    if sock == -1 {
        xerrlog(libc::LOG_ERR,
            c"socket: %s".as_ptr(),
            strerror(*libc::__errno_location()));
    }

    if connect(sock, res.ai_addr, res.ai_addrlen) == -1 {
        errlog(libc::LOG_INFO,
            c"%s:%s: %s".as_ptr(),
            host, port, strerror(*libc::__errno_location()));
        close(sock);
        return -1;
    }
    sock
}

#[no_mangle]
pub unsafe extern "C" fn conn_listen(
    host: *const c_char,
    port: *const c_char,
) -> c_int {
    let res_ptr = conn_addr(host, port);
    let res = &*res_ptr;

    let sock = socket(res.ai_family, res.ai_socktype, res.ai_protocol);
    if sock == -1 {
        xerrlog(libc::LOG_ERR,
            c"socket: %s".as_ptr(),
            strerror(*libc::__errno_location()));
    }

    let on: c_int = 1;
    if setsockopt(
        sock, SOL_SOCKET, SO_REUSEADDR,
        &on as *const _ as *const c_void,
        std::mem::size_of::<c_int>() as socklen_t,
    ) != 0 {
        xerrlog(libc::LOG_ERR,
            c"setsockopt: %s".as_ptr(),
            strerror(*libc::__errno_location()));
    }

    if bind(sock, res.ai_addr, res.ai_addrlen) == -1 {
        xerrlog(libc::LOG_ERR,
            c"bind on %s:%s: %s".as_ptr(),
            host, port, strerror(*libc::__errno_location()));
    }

    if listen(sock, 1) < 0 {
        xerrlog(libc::LOG_ERR,
            c"listen: %s".as_ptr(),
            strerror(*libc::__errno_location()));
    }

    freeaddrinfo(res_ptr);
    sock
}

#[no_mangle]
pub unsafe extern "C" fn conn_poll_for_accept(
    sock: c_int,
    pfd: *mut pollfd,
) -> c_int {
    let pfds = std::slice::from_raw_parts_mut(pfd, 1);
    pfds[0].fd = sock;
    pfds[0].events = POLLIN;
    let timeo_polla = net_timeout;

    loop {
        match libc::poll(pfd, 1, timeo_polla) {
            0 => {
                errlog(libc::LOG_INFO,
                    c"timeout polling to accept read connection".as_ptr());
                return -1;
            }
            -1 => {
                errlog(libc::LOG_ERR,
                    c"error polling to accept read connection: %s".as_ptr(),
                    strerror(*libc::__errno_location()));
                return -1;
            }
            _ => {}
        }
        if (pfds[0].revents & POLLIN) != 0 {
            let mut cliaddr: sockaddr = std::mem::zeroed();
            let mut len: socklen_t = std::mem::size_of::<sockaddr>() as socklen_t;
            let rd = accept(sock, &mut cliaddr, &mut len);
            if rd == -1 {
                errlog(libc::LOG_ERR,
                    c"error accepting read connection: %s".as_ptr(),
                    strerror(*libc::__errno_location()));
                return -1;
            }
            return rd;
        }
        return 0;  // matches C: falls through to break in inner switch
    }
}

#[no_mangle]
pub unsafe extern "C" fn conn_send_and_receive(
    rp: *mut c_char,
    wp: *const c_char,
    len: size_t,
    pfd: *mut pollfd,
    ntopoll: c_int,
) -> c_int {
    let timeo_pollio = net_timeout;
    let mut rleft = len;
    let mut wleft = len;
    let mut rp_cur = rp;
    let mut wp_cur = wp;

    let (ird, iwr): (usize, usize) = if ntopoll == 2 {
        (0, 1)
    } else {
        (0, 0)
    };

    let pfds = std::slice::from_raw_parts_mut(pfd, ntopoll as usize);
    let mut ntopoll = ntopoll;
    let mut loops = 0;

    while rleft > 0 {
        match libc::poll(pfd, ntopoll as nfds_t, timeo_pollio) {
            0 => return 1,
            -1 => return -1,
            _ => {}
        }

        if wleft > 0 && (pfds[iwr].revents & POLLOUT) != 0 {
            while wleft > 0 {
                let nwrite: ssize_t =
                    write(pfds[iwr].fd, wp_cur as *const c_void, wleft);
                if nwrite == 0 {
                    wleft = 0;
                    break;
                } else if nwrite == -1 {
                    let e = *libc::__errno_location();
                    if e == EINTR || e == EAGAIN {
                        break;
                    }
                    return -1;
                }
                wleft -= nwrite as size_t;
                wp_cur = wp_cur.add(nwrite as usize);
            }
            if wleft == 0 {
                ntopoll = 1;
                if iwr == ird {
                    pfds[ird].events = POLLIN;
                }
            }
        }

        if (pfds[ird].revents & POLLIN) != 0
            || (pfds[ird].revents & libc::POLLHUP) != 0
        {
            while rleft > 0 {
                let nread: ssize_t =
                    read(pfds[ird].fd, rp_cur as *mut c_void, rleft);
                if nread == 0 {
                    rleft = 0;
                    break;
                } else if nread == -1 {
                    let e = *libc::__errno_location();
                    if e == EINTR || e == EAGAIN {
                        break;
                    }
                    return -1;
                }
                rleft -= nread as size_t;
                rp_cur = rp_cur.add(nread as usize);
            }
        }

        loops += 1;
        if loops > 5 {
            libc::sleep(1);
            if loops > 10 {
                errlog(libc::LOG_INFO,
                    c"too many loops without data".as_ptr());
                return -1;
            }
        }
    }
    0
}

/// Test the connection. Returns 1 on success, 0 on failure.
#[no_mangle]
pub unsafe extern "C" fn conn_test(
    sock: c_int,
    host: *const c_char,
    write_port: *const c_char,
) -> c_int {
    let mut rval: c_int = 0;
    let mut tries: c_int = 0;
    let mut wd: c_int = -1;
    let mut rd: c_int = -1;

    let mut uts: libc::utsname = std::mem::zeroed();
    uname(&mut uts);
    let id: c_long = random();

    // Open write socket.
    wd = conn_remote(host, write_port);
    if wd == -1 {
        return 0;
    }

    let mut pfd: [pollfd; 2] = [
        pollfd { fd: -1, events: 0, revents: 0 },
        pollfd { fd: wd, events: POLLOUT, revents: 0 },
    ];

    // Buffer sized for nodename + format + echo_message margin.
    const BUF_SZ: usize = 64 + 65 + MAX_MESSAGE;
    let mut wbuf = [0u8; BUF_SZ];
    let mut rbuf = [0u8; BUF_SZ];

    while tries < MAX_CONN_TRIES {
        if tries + 1 >= MAX_CONN_TRIES {
            errlog(libc::LOG_DEBUG,
                c"tried connection %d times and failed".as_ptr(),
                tries + 1);
            tries += 1;
            break;
        }

        // Close stale read socket from previous iteration.
        if sock != NO_RD_SOCK && rd != -1 {
            shutdown(rd, SHUT_RDWR);
            close(rd);
            rd = -1;
        }

        // Build payload.
        let nodename_ptr = uts.nodename.as_ptr();
        let progname = if __progname.is_null() {
            c"autossh".as_ptr()
        } else {
            __progname
        };
        let n = libc::snprintf(
            wbuf.as_mut_ptr() as *mut c_char,
            BUF_SZ,
            c"%s %s %d %ld %s\r\n".as_ptr(),
            nodename_ptr, progname,
            getpid() as c_int, id, echo_message);
        if n as usize >= BUF_SZ {
            xerrlog(libc::LOG_ERR,
                c"conn_test: buffer overflow".as_ptr());
        }
        rbuf.fill(0);

        let ntopoll: c_int;
        if sock != NO_RD_SOCK {
            // Loopback mode: poll for accept then use both fds.
            rd = conn_poll_for_accept(sock, pfd.as_mut_ptr());
            if rd < 0 {
                tries += 1;
                break;
            }
            pfd[0].fd = rd;
            pfd[0].events = POLLIN;
            ntopoll = 2;
        } else {
            // Echo service mode.
            pfd[0].fd = wd;
            pfd[0].events = POLLIN | POLLOUT;
            ntopoll = 1;
        }

        let send_error = conn_send_and_receive(
            rbuf.as_mut_ptr() as *mut c_char,
            wbuf.as_ptr() as *const c_char,
            libc::strlen(wbuf.as_ptr() as *const c_char),
            pfd.as_mut_ptr(),
            ntopoll,
        );

        if send_error == 0 {
            if libc::strcmp(rbuf.as_ptr() as *const c_char,
                            wbuf.as_ptr() as *const c_char) == 0 {
                errlog(libc::LOG_DEBUG, c"connection ok".as_ptr());
                rval = 1;
                tries += 1;
                break;
            } else {
                errlog(libc::LOG_DEBUG,
                    c"not what I sent: \"%s\" : \"%s\"".as_ptr(),
                    wbuf.as_ptr(), rbuf.as_ptr());
            }
        } else if send_error == 1 {
            errlog(libc::LOG_DEBUG,
                c"timeout on io poll, looping to accept again".as_ptr());
        } else {
            errlog(libc::LOG_DEBUG,
                c"error on poll: %s".as_ptr(),
                strerror(*libc::__errno_location()));
            tries += 1;
            break;
        }
        tries += 1;
    }

    shutdown(wd, SHUT_RDWR);
    close(wd);
    if sock != NO_RD_SOCK && rd != -1 {
        shutdown(rd, SHUT_RDWR);
        close(rd);
    }

    rval
}
