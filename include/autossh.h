#ifndef LIBAUTOSSH_H
#define LIBAUTOSSH_H

#include <stddef.h>
#include <stdint.h>
#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct autossh_session autossh_session;

enum {
    AUTOSSH_SESSION_MONITOR_DISABLED = 1u << 0
};

enum {
    AUTOSSH_STATE_INIT = 0,
    AUTOSSH_STATE_STARTING = 1,
    AUTOSSH_STATE_RUNNING = 2,
    AUTOSSH_STATE_EXITED = 3,
    AUTOSSH_STATE_STOPPED = 4,
    AUTOSSH_STATE_ERROR = 5
};

typedef struct autossh_session_config {
    size_t struct_size;

    /* Optional path to ssh. NULL keeps autossh's default/AUTOSSH_PATH behavior. */
    const char *ssh_path;
    /* OpenSSH destination for the supervised process, e.g. "bastion" or "user@host". */
    const char *ssh_destination;
    /* TCP endpoint reached from ssh_destination by the local forward. */
    const char *target_host;
    uint16_t target_port;

    /* Local listener returned to the embedding client. NULL defaults to 127.0.0.1. */
    const char *bind_host;
    /* 0 asks libautossh to choose an ephemeral local port. */
    uint16_t local_port;
    /* 0 asks libautossh to choose monitor ports unless monitoring is disabled. */
    uint16_t monitor_port;
    uint32_t flags;

    /* Optional autossh tuning. Zero values keep autossh defaults except max_start. */
    int poll_time;
    int first_poll_time;
    double gate_time;
    int max_start;
    double max_session;
    int start_timeout_ms;

    const char *const *extra_ssh_args;
    size_t extra_ssh_args_len;
} autossh_session_config;

void autossh_session_config_init(autossh_session_config *config);
autossh_session *autossh_session_new(const autossh_session_config *config);
int autossh_session_start(autossh_session *session);
int autossh_session_stop(autossh_session *session);
void autossh_session_free(autossh_session *session);

int autossh_session_state(autossh_session *session);
pid_t autossh_session_pid(autossh_session *session);
uint16_t autossh_session_local_port(autossh_session *session);
const char *autossh_session_bind_host(autossh_session *session);
const char *autossh_session_last_error(autossh_session *session);

#ifdef __cplusplus
}
#endif

#endif /* LIBAUTOSSH_H */
