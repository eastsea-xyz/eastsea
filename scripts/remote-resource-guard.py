#!/usr/bin/env python3
"""Guard only this lane's remote processes; never manage machine-wide services."""
import argparse
import json
import os
from pathlib import Path
import pwd
import re
import signal
import shutil
import subprocess
import sys
import time

GIB = 2 ** 30
MIN_RAM = 4 * GIB
MIN_DISK = 30 * GIB
MAX_RSS = 12 * GIB
INTERVAL = 0.5


def authorized_base():
    account = pwd.getpwuid(os.geteuid())
    if sys.platform != 'darwin' or account.pw_name != 'kjaylee':
        raise RuntimeError('remote tests require the kjaylee macOS account on poc-m3')
    return Path(account.pw_dir) / 'eastsea-lab/dev-speed'


def validate_root(root):
    base = authorized_base()
    if root not in (base, base / 'source'):
        raise RuntimeError('remote work must stay under ~/eastsea-lab/dev-speed')
    for path in (base.parent, base, base / 'source', base / 'tmp', base / 'targets',
                 base / 'tmp/tools', base / 'tmp/cargo-home', base / 'tmp/sccache'):
        if path.is_symlink() or (path.exists() and not path.is_dir()):
            raise RuntimeError(f'remote lane path is not an owned directory: {path}')
    if (base / 'source/tmp').is_symlink():
        raise RuntimeError('remote source tmp must not be a symlink')
    return base


def free_ram():
    output = subprocess.check_output(['/usr/bin/vm_stat'], text=True, timeout=10)
    page_size = re.search(r'page size of (\d+) bytes', output)
    pages = dict(re.findall(r'^(Pages (?:free|speculative)):\s*(\d+)\.', output, re.M))
    if not page_size or 'Pages free' not in pages or 'Pages speculative' not in pages:
        raise RuntimeError('cannot determine free/speculative RAM from vm_stat')
    return int(page_size[1]) * sum(int(count) for count in pages.values())


def check_resources(root, rss=0, report=None):
    existing = root
    while not existing.exists():
        existing = existing.parent
    ram = free_ram()
    disk = shutil.disk_usage(existing).free
    if report is not None:
        report['min_free_ram_bytes'] = min(report['min_free_ram_bytes'], ram)
        report['min_free_disk_bytes'] = min(report['min_free_disk_bytes'], disk)
    if ram < MIN_RAM or disk < MIN_DISK or rss > MAX_RSS:
        raise RuntimeError(f'resource stop: free+speculative RAM={ram / GIB:.2f} GiB '
                           f'(minimum 4); disk={disk / GIB:.2f} GiB (minimum 30); '
                           f'owned RSS={rss / GIB:.2f} GiB (maximum 12)')
    return ram, disk


def process_table():
    output = subprocess.check_output(
        ['/bin/ps', '-axo', 'pid=,ppid=,pgid=,rss=,nice=,stat=,lstart='], text=True, timeout=10)
    rows = {}
    for line in output.splitlines():
        fields = line.split(None, 6)
        if len(fields) == 7 and not fields[5].startswith('Z'):
            pid, parent, group, rss, nice = map(int, fields[:5])
            rows[pid] = (parent, group, rss * 1024, fields[6], nice)
    return rows


class OwnedProcesses:
    def __init__(self, roots):
        self.roots = set(roots)
        self.root_births = dict.fromkeys(self.roots)
        self.children = {}
        self.known = {}
        self.groups = dict.fromkeys(self.roots)  # Private group ID -> original leader birth.
        self.discovery_error = None

    def register(self, child):
        # Popen(start_new_session=True) reserves this PID/private group until
        # the direct child is reaped, even if discovery never succeeds.
        self.roots.add(child.pid)
        self.root_births[child.pid] = None
        self.groups[child.pid] = None
        self.children[child.pid] = child

    def sample(self):
        try:
            rows = process_table()
        except Exception as error:
            self.discovery_error = error
            raise
        self.discovery_error = None
        groups = {}
        for group, birth in self.groups.items():
            leader = rows.get(group)
            if leader is not None:
                child = self.children.get(group)
                reserved = child is None or child.returncode is None
                if leader[1] == group and (birth == leader[3] or (birth is None and reserved)):
                    groups[group] = leader[3]
            elif any(row[1] == group and self.known.get(pid) == row[3]
                     for pid, row in rows.items()):
                # The leader may have exited while its original members live.
                groups[group] = birth
        owned = {pid for pid, row in rows.items()
                 if (pid in self.roots and (self.root_births[pid] == row[3]
                     or (self.root_births[pid] is None
                         and (pid not in self.children or self.children[pid].returncode is None))))
                 or self.known.get(pid) == row[3] or row[1] in groups}
        while True:
            children = {pid for pid, row in rows.items() if row[0] in owned}
            if children <= owned:
                break
            owned.update(children)
        for pid in owned:
            if pid in self.roots and self.root_births[pid] is None:
                self.root_births[pid] = rows[pid][3]
            if rows[pid][1] == pid:
                groups[pid] = rows[pid][3]
        # Discard finished groups instead of retaining IDs which can be reused
        # later by an unrelated process. Successful samples verify birth too.
        self.groups = {group: birth for group, birth in groups.items()
                       if any(rows[pid][1] == group for pid in owned)}
        self.known = {pid: rows[pid][3] for pid in owned}
        return {pid: rows[pid] for pid in owned}

    def terminate(self):
        if not self.groups and not self.children and not self.known:
            return
        def discover():
            try:
                return self.sample()
            except Exception as error:
                print(f'remote cleanup: discovery failed; using owned process groups: {error}',
                      file=sys.stderr, flush=True)
                return None

        def send(signum, rows):
            # Only private groups registered at spawn or birth-verified during
            # discovery are eligible. Never signal the guard's own group.
            for group in tuple(self.groups):
                if group <= 1 or group == os.getpgrp():
                    continue
                child = self.children.get(group)
                if child is not None and child.returncode is not None and rows is None:
                    continue  # Its PID was reaped; identity cannot be verified.
                try:
                    os.killpg(group, signum)
                except ProcessLookupError:
                    pass
                except OSError as error:
                    print(f'remote cleanup: group {group}: {error}', file=sys.stderr)
            for pid in (rows or {}):
                try:
                    os.kill(pid, signum)
                except ProcessLookupError:
                    pass
                except OSError as error:
                    print(f'remote cleanup: process {pid}: {error}', file=sys.stderr)
            # This fallback does not depend on ps, including for children whose
            # first monitoring sample failed before their group was recorded.
            for child in self.children.values():
                try:
                    signal_private_child(child, signum)
                except OSError as error:
                    print(f'remote cleanup: direct child {child.pid}: {error}', file=sys.stderr)

        # A failed monitoring probe already establishes that discovery is
        # unavailable. Do not spend another ps timeout before sending TERM.
        if self.discovery_error is not None:
            print(f'remote cleanup: discovery failed; using owned process groups: {self.discovery_error}',
                  file=sys.stderr, flush=True)
            rows = None
        else:
            rows = discover()
        send(signal.SIGTERM, rows)
        deadline = time.monotonic() + 3
        while (rows is None or rows) and time.monotonic() < deadline:
            time.sleep(0.1)
            if rows is not None:
                rows = discover()
        send(signal.SIGKILL, rows)


def signal_private_child(child, signum):
    if child.returncode is not None:
        return
    # The direct child has not been reaped, so its PID cannot have been reused.
    # Verify its private session before group signaling. Do not poll/reap here:
    # retaining the PID reserves its private group identity through the KILL
    # stage, including when descendants ignore TERM after their parent exits.
    try:
        if os.getpgid(child.pid) == child.pid:
            os.killpg(child.pid, signum)
    except ProcessLookupError:
        pass
    except OSError as error:
        # macOS may reject group signaling once only zombie members remain.
        # A group error must never prevent the independent direct-child signal.
        print(f'remote cleanup: private group {child.pid}: {error}; signaling child', file=sys.stderr)
    finally:
        try:
            os.kill(child.pid, signum)
        except ProcessLookupError:
            pass


def reap_children(children):
    errors = []
    for child in children:
        try:
            signal_private_child(child, signal.SIGTERM)
            # terminate() already allowed graceful cleanup. This independent
            # final fallback must kill the private group before reaping its
            # leader, even if discovery/termination itself raised an error.
            signal_private_child(child, signal.SIGKILL)
            try:
                child.wait(timeout=3)
            except subprocess.TimeoutExpired:
                signal_private_child(child, signal.SIGKILL)
                child.wait(timeout=3)
        except Exception as error:
            # A discovery/signaling error for one child must not skip the rest.
            try:
                signal_private_child(child, signal.SIGKILL)
                child.wait(timeout=3)
            except Exception as cleanup_error:
                errors.append(f'{child.pid}: {error}; {cleanup_error}')
    if errors:
        raise RuntimeError('could not reap owned children: ' + '; '.join(errors))


def run(root, command, sccache=False, owner_file=None, report_file=None):
    validate_root(root)
    if report_file and (report_file.parent != root / 'tmp' or report_file.is_symlink()):
        raise RuntimeError('resource report must be inside the owned source tmp directory')
    ram, disk = check_resources(root)
    started = time.monotonic()
    report = dict(schema_version=1, sample_interval_seconds=INTERVAL, samples=0,
                  min_free_ram_bytes=ram, min_free_disk_bytes=disk, peak_owned_rss_bytes=0,
                  nice_min=None, nice_max=None, cleanup_complete=False,
                  limits=dict(min_free_ram_bytes=MIN_RAM, min_free_disk_bytes=MIN_DISK,
                              max_owned_rss_bytes=MAX_RSS, nice=15))
    print(f'remote resources: RAM={ram / GIB:.2f} GiB; disk={disk / GIB:.2f} GiB; '
          'RSS limit=12 GiB; nice=15; checks every 0.5s', file=sys.stderr, flush=True)
    environment = dict(os.environ, AETHER_REMOTE_TEST_ROOT=str(root),
                       AETHER_REMOTE_GUARD_ACTIVE='1', TMPDIR=str(root / 'tmp'))
    environment.pop('AETHER_COMPILE_GATE_HELD', None)
    children = []
    owned = OwnedProcesses([])
    previous = {}
    socket = None
    owner_written = False
    parent = os.getppid()
    def sample_resources():
        rows = owned.sample()
        rss = sum(row[2] for row in rows.values())
        report['peak_owned_rss_bytes'] = max(report['peak_owned_rss_bytes'], rss)
        nice = [row[4] for row in rows.values()]
        if nice:
            report['nice_min'] = min(nice + ([report['nice_min']] if report['nice_min'] is not None else []))
            report['nice_max'] = max(nice + ([report['nice_max']] if report['nice_max'] is not None else []))
        report['samples'] += 1
        check_resources(root, rss, report)
    try:
        def stop(signum, _frame):
            raise SystemExit(128 + signum)
        for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            previous[sig] = signal.signal(sig, stop)
        if owner_file:
            if owner_file.parent != root / 'tmp' or owner_file.is_symlink():
                raise RuntimeError('owner record must be inside the owned source tmp directory')
            owner_file.write_text(json.dumps({'pid': os.getpid(),
                                              'start': process_table()[os.getpid()][3]}))
            owner_written = True
        if sccache:
            # A private foreground server stays in this process tree, including
            # in the RSS budget. No shared/default sccache server is stopped.
            # The outer snapshot lease serializes this lane. A stable socket
            # path also keeps SCCACHE_* out of cache invalidation on every run.
            socket = root / 'tmp/remote-sccache.sock'
            if socket.exists():
                socket = None
                raise RuntimeError('previous owned sccache socket still exists; check its guarded run')
            environment['SCCACHE_SERVER_UDS'] = str(socket)
            server_env = dict(environment, SCCACHE_START_SERVER='1', SCCACHE_NO_DAEMON='1')
            server = subprocess.Popen(['/usr/bin/nice', '-n', '15', environment['RUSTC_WRAPPER']],
                                      cwd=root, env=server_env, start_new_session=True)
            children.append(server)
            owned.register(server)
            deadline = time.monotonic() + 10
            while not socket.exists():
                if server.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError('private sccache server did not start')
                sample_resources()
                time.sleep(INTERVAL)
        child = subprocess.Popen(['/usr/bin/nice', '-n', '15', *command],
                                 cwd=root, env=environment, start_new_session=True)
        children.append(child)
        owned.register(child)
        while True:
            sample_resources()
            if os.getppid() != parent:
                raise RuntimeError('SSH command owner exited; stopping remote processes')
            code = child.poll()
            if code is not None:
                report['exit_code'] = code if code >= 0 else 128 - code
                return report['exit_code']
            if sccache and children[0].poll() is not None:
                raise RuntimeError('private sccache server exited during tests')
            time.sleep(INTERVAL)
    except BaseException as error:
        report['error'] = str(error)
        report['exit_code'] = error.code if isinstance(error, SystemExit) else 75
        raise
    finally:
        try:
            try:
                try:
                    # Cleanup waits run outside the signal handler. Ignore further
                    # signals so they cannot interrupt a Popen waitpid lock.
                    for sig in previous:
                        signal.signal(sig, signal.SIG_IGN)
                    owned.terminate()
                finally:
                    # Discovery must never be a prerequisite for direct cleanup.
                    reap_children(children)
                report['cleanup_complete'] = True
            finally:
                try:
                    if socket:
                        socket.unlink(missing_ok=True)
                finally:
                    if owner_written:
                        owner_file.unlink(missing_ok=True)
        except BaseException as error:
            report.update(cleanup_complete=False, error=str(error), exit_code=75)
            raise
        finally:
            for sig, handler in previous.items():
                signal.signal(sig, handler)
            if report_file:
                report['wall_seconds'] = time.monotonic() - started
                report_file.write_text(json.dumps(report) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--preflight', action='store_true')
    parser.add_argument('--root', type=Path)
    parser.add_argument('--sccache', action='store_true')
    parser.add_argument('--owner-file', type=Path)
    parser.add_argument('--report-file', type=Path)
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    root = args.root or authorized_base()
    if args.preflight:
        validate_root(root)
        ram, disk = check_resources(root)
        print(f'poc-m3 preflight: RAM={ram / GIB:.2f} GiB; disk={disk / GIB:.2f} GiB')
        return 0
    if args.command[:1] == ['--']:
        args.command.pop(0)
    if not args.command:
        parser.error('a guarded command is required')
    return run(root, args.command, args.sccache, args.owner_file, args.report_file)


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (RuntimeError, OSError, subprocess.SubprocessError) as error:
        print(f'remote-resource-guard: {error}', file=sys.stderr)
        sys.exit(75)
