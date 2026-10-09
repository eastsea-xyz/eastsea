#!/usr/bin/env bash
# R11 listener ownership: only task-created signed helper process trees.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p tmp/swift-module-cache
root=$(pwd -P)
export TMPDIR="$root/tmp"
work=$(mktemp -d "$root/tmp/R11-listener.XXXXXX")
gate="$root/scripts/compile-gate.sh"
[ -x "$gate" ] || { echo "FAIL R11 compile gate missing: $gate" >&2; exit 1; }
phase=prepare
report_failure() {
    local fixture_status=$1
    if [ "$fixture_status" -ne 0 ]; then
        printf 'FAIL R11 listener fixture: phase=%s exit=%s work=%s\n' "$phase" "$fixture_status" "$work" >&2
    fi
}
trap 'report_failure "$?"' EXIT
cat > "$work/R11-helper.c" <<'C'
#include <arpa/inet.h>
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <unistd.h>
static volatile sig_atomic_t child = 0;
static void stop_child(void) {
    if (child > 0) { kill(child, SIGTERM); while (waitpid(child, NULL, 0) < 0 && errno == EINTR) {} }
    child = 0;
}
static void terminate(int signo) {
    (void)signo;
    stop_child();
    _exit(0);
}
static int launch_listener(const char *binary, const char *ready, const char *port) {
    sigset_t blocked, previous;
    sigemptyset(&blocked); sigaddset(&blocked, SIGTERM); sigaddset(&blocked, SIGINT);
    if (sigprocmask(SIG_BLOCK, &blocked, &previous) != 0) { perror("R11 listener signal mask"); return 0; }
    pid_t spawned = fork();
    if (spawned == 0) {
        signal(SIGTERM, SIG_DFL); signal(SIGINT, SIG_DFL);
        if (sigprocmask(SIG_SETMASK, &previous, NULL) != 0) _exit(3);
        execl(binary, binary, "listen", ready, port, (char *)NULL);
        perror("R11 listener exec"); _exit(3);
    }
    int fork_errno = errno;
    // Publish the PID before an inherited/pending termination signal can run.
    if (spawned > 0) child = spawned;
    if (sigprocmask(SIG_SETMASK, &previous, NULL) != 0) {
        perror("R11 listener signal restore"); stop_child(); return 0;
    }
    if (spawned < 0) { errno = fork_errno; perror("R11 listener fork"); return 0; }
    return 1;
}
int main(int argc, char **argv) {
    if (argc > 1 && strcmp(argv[1], "parent") == 0 && (argc == 4 || argc == 6)) {
        signal(SIGTERM, terminate); signal(SIGINT, terminate);
        if (!launch_listener(argv[2], argv[3], NULL)) return 2;
        int restarted = 0;
        for (;;) {
            int status;
            pid_t exited = waitpid(child, &status, WNOHANG);
            if (exited == child) {
                fprintf(stderr, "R11 listener child %d exited: status=%d signal=%d\n", child,
                        WIFEXITED(status) ? WEXITSTATUS(status) : -1,
                        WIFSIGNALED(status) ? WTERMSIG(status) : 0);
                child = 0;
                return 10;
            }
            if (argc == 6 && !restarted && access(argv[5], F_OK) == 0) {
                int previous_pid; unsigned port = 0;
                FILE *ready = fopen(argv[3], "r");
                if (!ready || fscanf(ready, "%d %u", &previous_pid, &port) != 2 || previous_pid != child) {
                    if (ready) fclose(ready);
                    fprintf(stderr, "R11 listener restart readiness is invalid\n");
                    stop_child(); return 7;
                }
                fclose(ready);
                stop_child();
                unlink(argv[3]);
                char fixed_port[16]; snprintf(fixed_port, sizeof(fixed_port), "%u", port);
                if (!launch_listener(argv[4], argv[3], fixed_port)) return 8;
                restarted = 1;
            }
            usleep(10000);
        }
    }
    if (argc > 1 && strcmp(argv[1], "listen") == 0 && (argc == 3 || argc == 4)) {
        int fd = socket(AF_INET, SOCK_STREAM, 0);
        struct sockaddr_in addr = {0};
        addr.sin_len = sizeof(addr); addr.sin_family = AF_INET;
        addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
        addr.sin_port = argc == 4 ? htons((unsigned short)strtoul(argv[3], NULL, 10)) : 0;
        int reusable = 1;
        setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &reusable, sizeof(reusable));
        if (fd < 0 || bind(fd, (struct sockaddr *)&addr, sizeof(addr)) != 0 || listen(fd, 8) != 0) { perror("R11 listener bind/listen"); return 4; }
        socklen_t length = sizeof(addr);
        if (getsockname(fd, (struct sockaddr *)&addr, &length) != 0) { perror("R11 listener getsockname"); return 5; }
        char pending[4096];
        if (snprintf(pending, sizeof(pending), "%s.%d", argv[2], getpid()) >= sizeof(pending)) return 6;
        FILE *ready = fopen(pending, "w");
        if (!ready) { perror("R11 listener readiness open"); return 6; }
        fprintf(ready, "%d %u\n", getpid(), ntohs(addr.sin_port));
        if (fclose(ready) != 0 || rename(pending, argv[2]) != 0) { perror("R11 listener readiness publish"); return 6; }
        for (;;) { int client = accept(fd, NULL, NULL); if (client >= 0) { write(client, R11_LABEL "\n", strlen(R11_LABEL) + 1); close(client); } }
    }
    for (;;) pause();
}
C
phase='compile listener A'
"$gate" /usr/bin/clang -DR11_LABEL='"A"' "$work/R11-helper.c" -o "$work/R11-A"
phase='compile listener B'
"$gate" /usr/bin/clang -DR11_LABEL='"B"' "$work/R11-helper.c" -o "$work/R11-B"
phase='sign helpers'
/usr/bin/codesign --force --sign - --identifier com.pipln.eastsea.R11.listener "$work/R11-A" "$work/R11-B"
phase='compile Swift checker'
printf 'R11 listener fixture: %s (work=%s)\n' "$phase" "$work" >&2
python3 scripts/swift-test-cache.py --output "$work/R11-listener-check" -- apps/wallet/Sources/NodeReleaseIdentity.swift apps/wallet/Tests/update-daemon-tree/main.swift
phase='run Swift checker'
"$work/R11-listener-check" "$work/R11-A" "$work/R11-B" "$work"
