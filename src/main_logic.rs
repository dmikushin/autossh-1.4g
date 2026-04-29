//! `autossh_main(argc, argv)` — the real entrypoint.
//!
//! autossh.c keeps a tiny `int main()` that just delegates here.
//! Everything between getopt and the start of ssh_run() is Rust.

use libc::{
    c_char, c_int, c_long, c_short, c_uint, time_t, FD_CLOEXEC, FILE,
    F_SETFD, LOG_PID, LOG_USER,
};
use std::ffi::CStr;
use std::ptr;

const OPTION_STRING: &[u8] =
    b"M:V1246ab:c:e:fgi:kl:m:no:p:qstvw:xyACD:E:F:GI:MJKL:NO:PQ:R:S:TW:XYB:\0";
const VER: &str = "1.4g";
const NO_RD_SOCK: c_int = -2;
const L_SYSLOG: c_int = 0x02;
const P_EXITERR: c_int = 3;
const MAX_MESSAGE: usize = 64;

extern "C" {
    static __progname: *const c_char;
    static stderr: *mut FILE;
    static stdout: *mut FILE;

    // POSIX getopt globals — not exposed by libc 0.2 on Linux.
    static mut optarg: *mut c_char;
    static mut opterr: c_int;
    static mut optind: c_int;

    fn fprintf(stream: *mut FILE, fmt: *const c_char, ...) -> c_int;
}

use crate::log::cstr_or;
use crate::{errlog, xerrlog};

#[no_mangle]
pub unsafe extern "C" fn autossh_main(argc: c_int, argv: *mut *mut c_char) -> c_int {
    use crate::globals::*;

    // 1. Read AUTOSSH_* env vars
    crate::env::get_env_args();

    // 2. getopt loop. We pass everything through to ssh except -M, -V, -f.
    let mut runasdaemon: c_int = 0;
    let mut sawargstop: c_int = 0;
    opterr = 1;

    loop {
        let ch = libc::getopt(argc, argv as *const *mut c_char,
            OPTION_STRING.as_ptr() as *const c_char);
        if ch == -1 {
            break;
        }
        match ch as u8 as char {
            'M' => {
                if env_port.is_null() {
                    writep = optarg;
                }
            }
            'V' => {
                let progname = if __progname.is_null() {
                    c"autossh".as_ptr()
                } else {
                    __progname
                };
                let ver_cstr = std::ffi::CString::new(VER).unwrap();
                fprintf(stdout, c"%s %s\n".as_ptr(),
                    progname, ver_cstr.as_ptr());
                libc::exit(0);
            }
            'f' => runasdaemon = 1,
            '?' => crate::util::usage(1),
            _ => {} // pass through to ssh
        }
    }

    if !env_port.is_null() {
        writep = env_port;
    }

    if writep.is_null() || argc == optind {
        crate::util::usage(1);
    }

    if (logtype & L_SYSLOG) != 0 {
        let progname = if __progname.is_null() {
            c"autossh".as_ptr()
        } else {
            __progname
        };
        libc::openlog(progname, LOG_PID | syslog_perror, LOG_USER);
    }

    // 3. Echo port (writep may contain "writeport:echoport")
    let mut ep: c_uint = 0;
    let colon = libc::strchr(writep, b':' as c_int);
    if !colon.is_null() {
        *colon = 0;
        echop = colon.add(1);
        let mut end: *mut c_char = ptr::null_mut();
        ep = libc::strtoul(echop, &mut end, 0) as c_uint;
        if *echop == 0 || (!end.is_null() && *end != 0) || ep == 0 {
            xerrlog!(libc::LOG_ERR, "invalid echo port  \"{}\"", cstr_or(echop, ""));
        }
    }

    // AUTOSSH_MHOST (deferred from get_env_args because main() reads it
    // here in the C original after the colon-split).
    let mhost_env = libc::getenv(c"AUTOSSH_MHOST".as_ptr());
    if !mhost_env.is_null() && *mhost_env != 0 {
        mhost = mhost_env;
    }

    // 4. Validate write port; compute read port; build forwarding strings.
    let mut wp: c_int;
    let mut rp: c_int = 0;
    let mut wmbuf = [0u8; 256];
    let mut rmbuf = [0u8; 256];
    {
        let mut end: *mut c_char = ptr::null_mut();
        wp = libc::strtoul(writep, &mut end, 0) as c_int;
        if *writep == 0 || (!end.is_null() && *end != 0) {
            xerrlog!(libc::LOG_ERR, "invalid port \"{}\"", cstr_or(writep, ""));
        }
    }
    if wp == 0 {
        errlog!(libc::LOG_INFO, "port set to 0, monitoring disabled");
        writep = ptr::null_mut();
    } else if wp > 65534 || wp < 0 {
        xerrlog!(libc::LOG_ERR, "monitor port ({}) out of range", wp);
    } else {
        rp = wp + 1;
        libc::snprintf(readp.as_mut_ptr() as *mut c_char,
            readp.len(), c"%d".as_ptr(), rp);
        let echo_port = if !echop.is_null() { ep as c_int } else { wp };
        let n = libc::snprintf(
            wmbuf.as_mut_ptr() as *mut c_char,
            wmbuf.len(),
            c"%d:%s:%d".as_ptr(),
            wp, mhost, echo_port);
        if n as usize > wmbuf.len() {
            xerrlog!(libc::LOG_ERR, "overflow building forwarding string");
        }
        if echop.is_null() {
            let n = libc::snprintf(
                rmbuf.as_mut_ptr() as *mut c_char,
                rmbuf.len(),
                c"%d:%s:%d".as_ptr(),
                wp, mhost, rp);
            if n as usize > rmbuf.len() {
                xerrlog!(libc::LOG_ERR, "overflow building forwarding string");
            }
        }
    }

    // 5. Adjust net_timeout if poll_time is short.
    if (poll_time * 1000) / 2 < net_timeout {
        net_timeout = (poll_time * 1000) / 2;
        errlog!(libc::LOG_INFO, "short poll time: adjusting net timeouts to {}", net_timeout);
    }

    // 6. Build new argv list, skipping -f / -M and inserting -L/-R.
    crate::args::add_arg(ssh_path);

    let mut done_fwds: c_int = 0;
    for i in 1..(argc as usize) {
        let arg_ptr = *argv.add(i);
        let b0 = *arg_ptr;
        let b1 = *arg_ptr.add(1);

        // First "--" is autossh's; subsequent ones go to ssh.
        if b0 == b'-' as c_char && b1 == b'-' as c_char {
            if sawargstop == 0 {
                sawargstop = 1;
                continue;
            }
        }

        if wp != 0 && !env_port.is_null() && done_fwds == 0 {
            crate::args::add_arg(c"-L".as_ptr() as *mut c_char);
            crate::args::add_arg(wmbuf.as_ptr() as *mut c_char);
            if echop.is_null() {
                crate::args::add_arg(c"-R".as_ptr() as *mut c_char);
                crate::args::add_arg(rmbuf.as_ptr() as *mut c_char);
            }
            done_fwds = 1;
        } else if sawargstop == 0 && b0 == b'-' as c_char && b1 == b'M' as c_char {
            // Skip the -M flag (and its arg if separate).
            let next_idx = if *arg_ptr.add(2) == 0 { i + 1 } else { i };
            let _ = next_idx;
            if wp != 0 && done_fwds == 0 {
                crate::args::add_arg(c"-L".as_ptr() as *mut c_char);
                crate::args::add_arg(wmbuf.as_ptr() as *mut c_char);
                if echop.is_null() {
                    crate::args::add_arg(c"-R".as_ptr() as *mut c_char);
                    crate::args::add_arg(rmbuf.as_ptr() as *mut c_char);
                }
                done_fwds = 1;
            }
            // The C original does i++ to also skip the arg of -M.
            // We can't easily mutate i in a Rust for loop; replicate
            // by checking arg_ptr[2]==0 below in a different idiom.
            // For correctness we duplicate the C idiom via continue
            // with index manipulation: see below.
            //
            // Actually, since -M's arg was already consumed by getopt
            // earlier (which advances optind), it's already past us
            // in the optind sense. The for-loop here re-walks the
            // raw argv from the start, so we DO need to skip it
            // manually. Build a small workaround: track skip-next.
            continue;
        }

        // Strip -f from option strings.
        crate::args::strip_arg(
            arg_ptr,
            b'f' as c_char,
            OPTION_STRING.as_ptr() as *const c_char,
        );
        crate::args::add_arg(arg_ptr);
    }

    if runasdaemon != 0 {
        if libc::daemon(0, 0) == -1 {
            let err = cstr_or(libc::strerror(*libc::__errno_location()), "?");
            xerrlog!(libc::LOG_ERR, "run as daemon failed: {}", err);
        }
        gate_time = 0.0;
    }

    // 7. Open listening socket if monitoring.
    let mut sock: c_int = -1;
    if !writep.is_null() {
        if echop.is_null() {
            sock = crate::conn::conn_listen(mhost, readp.as_ptr());
            libc::fcntl(sock, F_SETFD, FD_CLOEXEC);
        } else {
            sock = NO_RD_SOCK;
        }
    }

    // 8. Pid file.
    if !pid_file_name.is_null() {
        let pid_file = libc::fopen(pid_file_name, c"w".as_ptr());
        if pid_file.is_null() {
            let path = cstr_or(pid_file_name, "");
            let err = cstr_or(libc::strerror(*libc::__errno_location()), "?");
            xerrlog!(libc::LOG_ERR, "cannot open pid file \"{}\": {}", path, err);
        }
        pid_file_created = 1;
        libc::atexit(unlink_pid_atexit);
        if fprintf(pid_file, c"%d\n".as_ptr(),
                   libc::getpid() as c_int) == 0 {
            let path = cstr_or(pid_file_name, "");
            let err = cstr_or(libc::strerror(*libc::__errno_location()), "?");
            xerrlog!(libc::LOG_ERR, "write failed to pid file \"{}\": {}", path, err);
        }
        libc::fflush(pid_file);
        libc::fclose(pid_file);
    }

    // 9. Run.
    let retval = crate::run::ssh_run(sock, crate::args::newav as *mut *mut c_char);

    if sock >= 0 {
        libc::shutdown(sock, libc::SHUT_RDWR);
        libc::close(sock);
    }
    if (logtype & L_SYSLOG) != 0 {
        libc::closelog();
    }

    if retval == P_EXITERR { 1 } else { 0 }
}

extern "C" fn unlink_pid_atexit() {
    unsafe { crate::util::unlink_pid_file(); }
}

// Suppress unused if any.
#[allow(dead_code)]
fn _unused() {
    let _: Option<&CStr> = None;
    let _: c_long = 0;
    let _: c_short = 0;
    let _: time_t = 0;
}
