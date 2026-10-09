#!/usr/bin/env bash
# R11 listener ownership: only task-created signed helper process trees.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p tmp/swift-module-cache
root=$(pwd -P)
export TMPDIR="$root/tmp"
work=$(mktemp -d "$root/tmp/R11-listener.XXXXXX")
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
static void terminate(int signo) {
    if (child > 0) { kill(child, SIGTERM); while (waitpid(child, NULL, 0) < 0 && errno == EINTR) {} }
    _exit(0);
}
int main(int argc, char **argv) {
    if (argc > 1 && strcmp(argv[1], "parent") == 0 && (argc == 4 || argc == 6)) {
        signal(SIGTERM, terminate); signal(SIGINT, terminate);
        child = fork();
        if (child < 0) return 2;
        if (child == 0) { execl(argv[2], argv[2], "listen", argv[3], (char *)NULL); _exit(3); }
        int restarted = 0;
        for (;;) {
            if (argc == 6 && !restarted && access(argv[5], F_OK) == 0) {
                int previous_pid; unsigned port = 0;
                FILE *ready = fopen(argv[3], "r");
                if (!ready || fscanf(ready, "%d %u", &previous_pid, &port) != 2) return 7;
                fclose(ready);
                kill(child, SIGTERM); while (waitpid(child, NULL, 0) < 0 && errno == EINTR) {}
                char fixed_port[16]; snprintf(fixed_port, sizeof(fixed_port), "%u", port);
                child = fork();
                if (child < 0) return 8;
                if (child == 0) { execl(argv[4], argv[4], "listen", argv[3], fixed_port, (char *)NULL); _exit(9); }
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
        if (fd < 0 || bind(fd, (struct sockaddr *)&addr, sizeof(addr)) != 0 || listen(fd, 8) != 0) return 4;
        socklen_t length = sizeof(addr);
        if (getsockname(fd, (struct sockaddr *)&addr, &length) != 0) return 5;
        FILE *ready = fopen(argv[2], "w");
        if (!ready) return 6;
        fprintf(ready, "%d %u\n", getpid(), ntohs(addr.sin_port)); fclose(ready);
        for (;;) { int client = accept(fd, NULL, NULL); if (client >= 0) { write(client, R11_LABEL "\n", strlen(R11_LABEL) + 1); close(client); } }
    }
    for (;;) pause();
}
C
/usr/bin/clang -DR11_LABEL='"A"' "$work/R11-helper.c" -o "$work/R11-A"
/usr/bin/clang -DR11_LABEL='"B"' "$work/R11-helper.c" -o "$work/R11-B"
/usr/bin/codesign --force --sign - --identifier com.pipln.eastsea.R11.listener "$work/R11-A" "$work/R11-B"
python3 scripts/swift-test-cache.py --output "$work/R11-listener-check" -- apps/wallet/Sources/NodeReleaseIdentity.swift apps/wallet/Tests/update-daemon-tree/main.swift
"$work/R11-listener-check" "$work/R11-A" "$work/R11-B" "$work"
