#!/usr/bin/env python3
"""CPU distributed compilation and deployment. One operator CLI; no evaluations."""
import argparse
import contextlib
import ctypes
import datetime
import fcntl
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import platform
import pwd
import re
import shlex
import signal
import socket
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.parse

ROOT = Path(__file__).resolve().parents[2]
SPECS = {'v1': (32, 0), 'v2': (16, 31), 'v3': (32, 46)}
SSH_PASSWORD_PATH = ROOT / '.server/distributed-production/ssh-password'
SSH_ENV = None


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def cpus(value):
    result = set()
    for part in value.split(','):
        require(re.fullmatch(r'\d+(?:-\d+)?', part), 'invalid CPU list')
        a, _, b = part.partition('-')
        lo, hi = int(a), int(b or a)
        require(0 <= lo <= hi <= 4095, 'invalid CPU range')
        result.update(range(lo, hi + 1))
    return result


def config_bytes(cfg):
    return (json.dumps(cfg, indent=2) + '\n').encode()


def validate_url(value, allow_placeholder=False):
    url = urllib.parse.urlsplit(value)
    require(url.scheme == 'http' and url.hostname and (allow_placeholder or 'CHANGE_ME' not in value)
            and not url.username and not url.password and not url.query and not url.fragment,
            'server URL must be the actual HTTP competition URL without credentials/query')
    require(url.port is None or 1 <= url.port <= 65535, 'invalid HTTP port')


def load_config(path, allow_placeholder=False, allow_uncompiled=False):
    cfg = json.loads(Path(path).read_text())
    keys = {'server_url', 'remote_directory', 'worker_port', 'solver_binary', 'solver_sha256',
            'bridge_binary', 'bridge_sha256', 'nodes'}
    require(keys <= set(cfg) <= keys | {'select_ms'}, 'unknown or missing public configuration fields')
    validate_url(cfg['server_url'], allow_placeholder)
    directory = cfg['remote_directory']
    require(re.fullmatch(r'[A-Za-z0-9_-]+(?:/[A-Za-z0-9_.-]+)+', directory)
            and all(x not in ('.', '..') for x in directory.split('/')),
            'remote_directory must be a safe relative path below the SSH user home')
    require(type(cfg['worker_port']) is int and 1 <= cfg['worker_port'] <= 65535, 'invalid worker port')
    selection = cfg.get('select_ms')
    require(selection is None or type(selection) is int and 1 <= selection <= 600000,
            'select_ms must be null (official default) or a positive millisecond limit')
    require(set(cfg['nodes']) == set(SPECS), 'exactly v1/v2/v3 are required')
    for role, node in cfg['nodes'].items():
        require(set(node) == {'ssh', 'address', 'cpus'}, 'unknown or missing node configuration fields')
        ipaddress.IPv4Address(node['address'])
        require(re.fullmatch(r'[A-Za-z0-9_.-]+@[A-Za-z0-9_.-]+', node['ssh']), 'invalid SSH user@host')
        require(node['ssh'].split('@')[1] == node['address'], 'SSH must use the configured wired IPv4 address')
        require(len(cpus(node['cpus'])) == SPECS[role][0], 'CPU count must match 32/16/32')
    require(len({n['address'] for n in cfg['nodes'].values()}) == 3, 'node addresses must be distinct')
    require(len({n['ssh'] for n in cfg['nodes'].values()}) == 3, 'SSH destinations must be distinct')
    for name in ('solver', 'bridge'):
        digest = cfg[name + '_sha256']
        if allow_uncompiled and digest is None:
            continue
        require(isinstance(digest, str) and re.fullmatch(r'[0-9a-f]{64}', digest),
                'Binary SHA-256 is not registered. Run compile.sh first.')
    return cfg


def operator_config(path, action, server_url):
    path = Path(path)
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes((Path(__file__).parent / 'distributed-config.example.json').read_bytes())
    cfg = json.loads(path.read_text())
    require(not server_url or action == 'pack', '--server for deployment configuration is only supported by pack')
    if server_url:
        cfg['server_url'] = server_url
    path.write_bytes(config_bytes(cfg))
    return load_config(path, allow_placeholder=True, allow_uncompiled=action == 'compile')


def require_void1(cfg):
    require(pwd.getpwuid(os.getuid()).pw_name == cfg['nodes']['v1']['ssh'].split('@')[0],
            'Run as the configured VOID1 user, without sudo.')
    data = json.loads(run(['ip', '-j', '-4', 'address', 'show'], capture_output=True, text=True).stdout)
    addresses = {a['local'] for link in data for a in link.get('addr_info', []) if a.get('family') == 'inet'}
    require(cfg['nodes']['v1']['address'] in addresses, 'Run this operation on the PC with the configured VOID1 wired LAN address.')


@contextlib.contextmanager
def ssh_auth():
    """Use the configured shared password with standard OpenSSH (no sshpass dependency)."""
    global SSH_ENV
    password = os.environ.get('PROCON37_SSH_PASSWORD')
    if password is None and SSH_PASSWORD_PATH.exists():
        password = SSH_PASSWORD_PATH.read_text().strip()
    with tempfile.TemporaryDirectory(prefix='procon37-ssh-') as td:
        env = os.environ.copy()
        if password:
            helper = Path(td) / 'askpass'
            helper.write_text("#!/bin/sh\nprintf '%s\\n' " + shlex.quote(password) + '\n')
            helper.chmod(0o700)
            env.update(SSH_ASKPASS=str(helper), SSH_ASKPASS_REQUIRE='force', DISPLAY=env.get('DISPLAY') or ':0')
        SSH_ENV = env
        try:
            yield
        finally:
            SSH_ENV = None


def run(command, **kwargs):
    return subprocess.run(command, check=True, **kwargs)


def package(cfg, output):
    """Archive explicit deployment files; runtime team/cluster tokens are separate."""
    output = Path(output).resolve()
    require(not output.exists(), 'package already exists; use another output name')
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='procon37-distributed-pack-') as td:
        base = Path(td)
        (base / 'bin').mkdir()
        for name, target in [('solver', 'distributed-solver'), ('bridge', 'server-bridge')]:
            origin = cfg[name + '_binary']
            require(not origin.startswith('bundled:'),
                    'Bundled binaries cannot be deployed. Run compile.sh first.')
            source = Path(origin)
            if not source.is_absolute():
                source = ROOT / source
            require(source.is_file() and not source.is_symlink(),
                    'binary is missing or symlink: ' + name + '. Run compile.sh first.')
            source_bytes = source.read_bytes()
            require(hashlib.sha256(source_bytes).hexdigest() == cfg[name + '_sha256'], 'binary SHA256 mismatch: ' + name)
            require(source_bytes[:6] == b'\x7fELF\x02\x01', 'binary must be a 64-bit little-endian ELF: ' + name)
            dest = base / 'bin' / target
            dest.write_bytes(source_bytes)
            dest.chmod(0o755)
        (base / 'distributed.py').write_bytes(Path(__file__).read_bytes())
        (base / 'node-config.json').write_bytes(config_bytes(cfg))
        paths = ['bin/distributed-solver', 'bin/server-bridge', 'distributed.py', 'node-config.json']
        manifest = {p: sha(base / p) for p in paths}
        (base / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        with tarfile.open(output, 'x:gz') as archive:
            for p in paths + ['manifest.json']:
                archive.add(base / p, arcname=p, recursive=False)
    return sha(output)


def compile_release(cfg, config_path):
    """Build on VOID1 with its installed Rust/cache; select only after both builds succeed."""
    binaries = {'solver': 'solver/distributed/target/release/distributed-solver',
                'bridge': 'target/release/server-bridge'}
    run(['taskset', '-c', '4-11', 'cargo', 'build', '--release', '--locked', '--offline', '-j', '2',
         '--manifest-path', 'solver/distributed/Cargo.toml'], cwd=ROOT)
    run(['taskset', '-c', '4-11', 'cargo', 'build', '--release', '--locked', '--offline', '-j', '2',
         '-p', 'server-bridge'], cwd=ROOT)
    selected = cfg.copy()
    for name, path in binaries.items():
        require((ROOT / path).is_file(), 'compiled binary is missing: ' + name)
        selected[name + '_binary'] = path
        selected[name + '_sha256'] = sha(ROOT / path)
    # Original configuration survives a failed build; active deployed releases are never overwritten.
    temporary = Path(config_path).with_suffix('.building.json')
    temporary.write_bytes(config_bytes(selected))
    load_config(temporary, allow_placeholder=True)
    temporary.replace(config_path)
    print('[compile] Build complete. Registered both VOID1 binaries and their SHA-256 hashes.')
    print('[compile] Next: deploy.sh. For updates, stop the old deployment and choose an unused destination.')


def remote(cfg, role, command, **kwargs):
    if role == 'v1':
        return run(['sh', '-c', command], **kwargs)
    return run(['ssh', '-o', 'StrictHostKeyChecking=accept-new', '-o', 'ConnectTimeout=10',
                cfg['nodes'][role]['ssh'], command], env=SSH_ENV, stdin=subprocess.DEVNULL, **kwargs)


def upload(cfg, role, source, target):
    if role == 'v1':
        dest = Path.home() / target
        dest.write_bytes(Path(source).read_bytes())
        dest.chmod(Path(source).stat().st_mode & 0o777)
    else:
        run(['scp', '-p', '-o', 'StrictHostKeyChecking=accept-new', '-o', 'ConnectTimeout=10',
             str(source), cfg['nodes'][role]['ssh'] + ':' + target], env=SSH_ENV, stdin=subprocess.DEVNULL)


def remote_node(cfg, role, action, **kwargs):
    directory = cfg['remote_directory']
    expected = hashlib.sha256(config_bytes(cfg)).hexdigest() if action in ('check', 'start') else ''
    return remote(cfg, role, 'python3 "$HOME/' + directory + '/distributed.py" _node '
                  + action + ' "$HOME/' + directory + '" ' + role + (' ' + expected if expected else ''), **kwargs)


def spawn_node(cfg, role):
    """Keep stdout/stderr connected to the operator, including over SSH."""
    directory = cfg['remote_directory']
    expected = hashlib.sha256(config_bytes(cfg)).hexdigest()
    command = 'exec python3 -u "$HOME/' + directory + '/distributed.py" _node start "$HOME/' + directory + '" ' + role + ' ' + expected
    if role == 'v1':
        args = ['sh', '-c', command]
    else:
        args = ['ssh', '-T', '-o', 'StrictHostKeyChecking=accept-new', '-o', 'ConnectTimeout=10',
                cfg['nodes'][role]['ssh'], command]
    return subprocess.Popen(args, env=SSH_ENV, stdin=subprocess.DEVNULL, start_new_session=True)


def wait_node_started(cfg, role, child):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        require(child.poll() is None, role + ': runner exited; see terminal output')
        status = json.loads(remote_node(cfg, role, 'status', capture_output=True, text=True).stdout)
        require(not status['identity_mismatch'], role + ': runner identity changed')
        if status['running']:
            return
        time.sleep(.1)
    raise RuntimeError(role + ': runner startup timed out')


def stop_runners(cfg, runners):
    failures = []
    for role in ('v1', 'v3', 'v2'):
        if role not in runners:
            continue
        try:
            remote_node(cfg, role, 'stop')
        except Exception:
            failures.append(role)
            print(role + ': shutdown could not be confirmed. Check the connection and run run.sh stop.', file=sys.stderr)
        child = runners[role]
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            child.terminate()
            try:
                child.wait(timeout=3)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
    return failures


@contextlib.contextmanager
def terminal_signals():
    """Terminal closure and termination follow the same cleanup path as Ctrl+C."""
    def interrupted(signum, frame):
        raise KeyboardInterrupt
    previous = {sig: signal.signal(sig, interrupted) for sig in (signal.SIGTERM, signal.SIGHUP)}
    try:
        yield
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)


def deploy(cfg, archive):
    # Preflight every destination before any installation; existing releases are preserved.
    for role in SPECS:
        remote(cfg, role, 'set -eu; command -v python3 >/dev/null; command -v tar >/dev/null; '
               'test ! -e "$HOME/' + cfg['remote_directory'] + '"')
    with tempfile.TemporaryDirectory(prefix='procon37-deploy-private-') as td:
        private = Path(td)
        private.chmod(0o700)
        shared = private / 'cluster.token'
        import secrets
        shared.write_text(secrets.token_hex(32) + '\n')
        shared.chmod(0o600)
        for role in SPECS:
            directory = cfg['remote_directory']
            parent = directory.rsplit('/', 1)[0]
            remote(cfg, role, 'set -eu; umask 077; mkdir -p "$HOME/' + parent + '"; '
                   'mkdir "$HOME/' + directory + '"; mkdir "$HOME/' + directory + '/private"')
            target = directory + '/'
            upload(cfg, role, archive, target + 'package.tar.gz')
            upload(cfg, role, shared, target + 'private/cluster.token')
            checksum = 'import hashlib; assert hashlib.sha256(open("package.tar.gz","rb").read()).hexdigest() == ' + repr(sha(archive))
            remote(cfg, role, 'set -eu; cd "$HOME/' + directory + '"; '
                   'python3 -c ' + shlex.quote(checksum) + '; '
                   'tar -xzf package.tar.gz; chmod 700 private; chmod 600 private/*; '
                   'python3 distributed.py _node check "$PWD" ' + role)
    print('[deploy] All three nodes deployed and verified. Processes have not been started.')


def verify_node(directory, role, expected_config=None):
    base = Path(directory).resolve()
    manifest = json.loads((base / 'manifest.json').read_text())
    expected = {'bin/distributed-solver', 'bin/server-bridge', 'distributed.py', 'node-config.json'}
    require(set(manifest) == expected, 'invalid deployment manifest')
    for name, digest in manifest.items():
        path = base / name
        require(not path.is_symlink() and sha(path) == digest, 'deployed file changed: ' + name)
    cfg = load_config(base / 'node-config.json', allow_placeholder=True)
    if expected_config:
        require(sha(base / 'node-config.json') == expected_config,
                'Configuration differs from the deployed release. Stop it and deploy to another remote_directory.')
    require(sys.version_info >= (3, 10) and hasattr(os, 'pidfd_open'), 'Python 3.10+ and Linux pidfd are required')
    require(platform.machine() == 'x86_64', 'deployment requires x86_64 Linux/glibc')
    libc = os.confstr('CS_GNU_LIBC_VERSION') or ''
    require(libc.startswith('glibc '), 'glibc Linux is required')
    version = tuple(map(int, libc.split()[1].split('.')[:2]))
    require(version >= ((2, 39) if role == 'v1' else (2, 34)), 'installed glibc is too old for the deployed binary')
    for library in ('libgcc_s.so.1', 'libm.so.6'):
        ctypes.CDLL(library)
    require(Path('/lib64/ld-linux-x86-64.so.2').is_file(), 'ELF dynamic loader is missing')
    require(sha(base / 'bin/distributed-solver') == cfg['solver_sha256'], 'solver identity mismatch')
    require(sha(base / 'bin/server-bridge') == cfg['bridge_sha256'], 'bridge identity mismatch')
    require(os.access(base / 'bin/distributed-solver', os.X_OK)
            and os.access(base / 'bin/server-bridge', os.X_OK), 'binary execute permission is missing')
    require(cpus(cfg['nodes'][role]['cpus']) <= os.sched_getaffinity(0), 'requested CPUs are unavailable')
    for name in ['cluster.token']:
        p = base / 'private' / name
        require(p.is_file() and not p.is_symlink() and p.stat().st_mode & 0o077 == 0
                and p.stat().st_uid == os.getuid(), 'private token permissions/owner')
    shared = (base / 'private/cluster.token').read_text().strip()
    require(32 <= len(shared) <= 128 and all(33 <= ord(c) <= 126 for c in shared), 'invalid shared token')
    return base, cfg


def runtime_settings(server, token, select_ms, record_directory=None):
    validate_url(server)
    require(token and '\n' not in token and '\r' not in token, 'TOKEN must be one nonempty line')
    require(select_ms is None or type(select_ms) is int and 1 <= select_ms <= 600000,
            'SELECT_MS must be empty or a positive millisecond limit')
    require(record_directory is None or isinstance(record_directory, (str, Path)) and str(record_directory),
            'RECORD_DIR must not be empty')
    directory = Path(record_directory if record_directory is not None else ROOT / '.server/submissions').resolve()
    return {'server_url': server, 'token': token, 'select_ms': select_ms, 'record_directory': str(directory)}


def write_runtime(base, settings):
    """Connection settings belong to VOID1 startup, separate from the immutable deployment."""
    values = {'private/competition.token': settings['token'] + '\n',
              'runtime.json': json.dumps({k: settings[k] for k in ('server_url', 'select_ms', 'record_directory')}, indent=2) + '\n'}
    for name, value in values.items():
        destination = base / name
        temporary = destination.with_name(destination.name + '.starting')
        with temporary.open('w') as f:
            os.fchmod(f.fileno(), 0o600)
            f.write(value)
        temporary.replace(destination)


def load_runtime(base):
    data = json.loads((base / 'runtime.json').read_text())
    token_file = base / 'private/competition.token'
    require(token_file.is_file() and not token_file.is_symlink()
            and token_file.stat().st_mode & 0o077 == 0 and token_file.stat().st_uid == os.getuid(),
            'competition token permissions/owner')
    return runtime_settings(data['server_url'], token_file.read_text().strip(), data.get('select_ms'),
                            data.get('record_directory', str(base / 'records')))


def process(pid):
    try:
        p = Path('/proc') / str(pid)
        fields = (p / 'stat').read_text().split(') ', 1)[1].split()
        return {'pid': pid, 'start': int(fields[19]), 'state': fields[0],
                'sid': int(fields[3]), 'uid': p.stat().st_uid, 'cwd': os.readlink(p / 'cwd'),
                'boot': Path('/proc/sys/kernel/random/boot_id').read_text().strip()}
    except (FileNotFoundError, ProcessLookupError, PermissionError):
        return None


def matches(saved, current):
    return saved and current and all(saved[k] == current[k] for k in ('pid', 'start', 'sid', 'uid', 'cwd', 'boot'))


def live_handles(handles):
    alive = []
    for fd, saved in handles:
        current = process(saved['pid'])
        if matches(saved, current) and current['state'] != 'Z':
            alive.append((fd, saved))
    return alive


def worker_ready(cfg, role, shared):
    """Authenticate without a Solve request. Call only before the coordinator starts."""
    def receive(sock, count):
        data = b''
        while len(data) < count:
            part = sock.recv(count - len(data))
            require(part, 'worker closed during authentication')
            data += part
        return data
    message = json.dumps({'Authenticate': {'version': 1, 'token': shared}}).encode()
    with socket.create_connection((cfg['nodes'][role]['address'], cfg['worker_port']), timeout=3) as sock:
        sock.sendall(len(message).to_bytes(4, 'big') + message)
        length = int.from_bytes(receive(sock, 4), 'big')
        require(1 <= length <= 4096, 'invalid worker Ready frame')
        reply = json.loads(receive(sock, length))
    total, offset = SPECS[role]
    require(reply == {'Ready': {'version': 1, 'total_threads': total, 'offset': offset}},
            role + ': worker protocol/thread/offset mismatch')
    print(role + ': wired TCP authentication OK (' + str(total - 1) + ' search threads)')


def start_cluster(cfg, settings=None):
    prior = {}
    for role in SPECS:
        remote_node(cfg, role, 'check')
        status = json.loads(remote_node(cfg, role, 'status', capture_output=True, text=True).stdout)
        require(not status['identity_mismatch'] and not status['orphans'], role + ': inspect/stop the old session first')
        prior[role] = status['running']
    base = Path.home() / cfg['remote_directory']
    if prior['v1'] and settings is not None:
        require(load_runtime(base) == settings,
                'Connection settings differ from the running configuration. Run run.sh stop between matches before restarting.')
    if all(prior.values()):
        print('[run.sh] All three nodes are already running. No additional processes were started.')
        return {}
    require(not prior['v1'], 'VOID1 is running without all workers. Stop between matches before starting again.')
    if settings is not None:
        write_runtime(base, settings)
    runners = {}
    try:
        for role in ('v2', 'v3'):
            if not prior[role]:
                runners[role] = spawn_node(cfg, role)
                wait_node_started(cfg, role, runners[role])
        shared = (Path.home() / cfg['remote_directory'] / 'private/cluster.token').read_text().strip()
        for role in ('v2', 'v3'):
            worker_ready(cfg, role, shared)
        runners['v1'] = spawn_node(cfg, 'v1')
        wait_node_started(cfg, 'v1', runners['v1'])
    except BaseException:
        stop_runners(cfg, runners)
        raise
    return runners


def run_cluster(cfg, settings):
    runners = {}
    try:
        # Release the operator lock after startup so another terminal can status/stop.
        with operator_lock():
            runners = start_cluster(cfg, settings)
        if not runners:
            return
        print('[run.sh] Streaming all three nodes to this terminal. Press Ctrl+C to stop.', flush=True)
        while True:
            code = runners['v1'].poll()
            if code is not None:
                require(code == 0, 'VOID1 exited unsuccessfully. Check the terminal output above.')
                return
            for role in ('v2', 'v3'):
                if role in runners:
                    require(runners[role].poll() is None,
                            role + ': worker/SSHが終了しました。上のログを確認してください。')
            time.sleep(.1)
    finally:
        failures = stop_runners(cfg, runners)
        require(not failures, 'Nodes with unconfirmed shutdown: ' + ','.join(failures))


def stop_owned(saved, base):
    """PID reuse never redirects a signal; only the verified runner's descendants are targeted."""
    current = process(saved['pid'])
    require(saved['sid'] == saved['pid'] and saved['uid'] == os.getuid() and saved['cwd'] == str(base),
            'invalid owned session record')
    if saved['boot'] != Path('/proc/sys/kernel/random/boot_id').read_text().strip():
        return []  # All members of the recorded boot disappeared on reboot.
    if current:
        require(matches(saved, current), 'runner identity changed; refusing to stop another process')
    # Include orphaned members of this dedicated session after an earlier runner exit.
    queue, targets = ([saved['pid']] if current else []), []
    for p in Path('/proc').iterdir():
        if p.name.isdigit():
            r = process(int(p.name))
            if (r and r['sid'] == saved['sid'] and r['uid'] == saved['uid']
                    and Path(r['cwd']).is_relative_to(base)):
                queue.append(r['pid'])
    while queue:
        pid = queue.pop()
        if pid in targets:
            continue
        targets.append(pid)
        try:
            for task in (Path('/proc') / str(pid) / 'task').iterdir():
                queue.extend(map(int, (task / 'children').read_text().split()))
        except (FileNotFoundError, ProcessLookupError):
            pass
    handles = []
    observations = {pid: (saved if pid == saved['pid'] else process(pid)) for pid in targets}
    try:
        # Terminate the supervisor first so it cannot continually replace children.
        targets.sort(key=lambda pid: pid != saved['pid'])
        for pid in targets:
            try:
                fd = os.pidfd_open(pid)
                record = process(pid)
                if record is None:
                    os.close(fd)
                    continue
                if not matches(observations[pid], record):
                    os.close(fd)
                    continue
                if not (record['uid'] == os.getuid() and record['sid'] == saved['sid']
                        and Path(record['cwd']).is_relative_to(base)):
                    os.close(fd)
                    raise RuntimeError('unexpected descendant ownership')
                handles.append((fd, record))
            except ProcessLookupError:
                pass
        for fd, _ in handles:
            try:
                signal.pidfd_send_signal(fd, signal.SIGTERM)
            except ProcessLookupError:
                pass
        # The supervisor may have spawned a child immediately before termination.
        known = {r['pid'] for _, r in handles}
        for p in Path('/proc').iterdir():
            if not p.name.isdigit() or int(p.name) in known:
                continue
            r = process(int(p.name))
            if (r and r['sid'] == saved['sid'] and r['uid'] == saved['uid']
                    and Path(r['cwd']).is_relative_to(base)):
                try:
                    fd = os.pidfd_open(r['pid'])
                    if not matches(r, process(r['pid'])):
                        os.close(fd)
                        continue
                    handles.append((fd, r))
                    targets.append(r['pid'])
                    signal.pidfd_send_signal(fd, signal.SIGTERM)
                except ProcessLookupError:
                    pass
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            alive = live_handles(handles)
            if not alive:
                return targets
            time.sleep(.05)
        for fd, _ in alive:
            try:
                signal.pidfd_send_signal(fd, signal.SIGKILL)
            except ProcessLookupError:
                pass
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            if not live_handles(handles):
                return targets
            time.sleep(.05)
        raise RuntimeError('owned processes did not exit')
    finally:
        for fd, _ in handles:
            os.close(fd)


def node_action(action, directory, role, expected_config=None):
    base = Path(directory).resolve()
    require(base.is_dir() and not Path(directory).is_symlink(), 'invalid node directory')
    child = saved = None
    with (base / '.control.lock').open('a') as lock:
        if action != 'run':
            fcntl.flock(lock, fcntl.LOCK_EX)
        record_file = base / 'runner.json'
        if action == 'check':
            verify_node(base, role, expected_config)
            print(role + ': package, token permissions and CPU allocation OK')
        elif action == 'start':
            verify_node(base, role, expected_config)
            if role == 'v1':
                load_runtime(base)
            # Avoid two deployments submitting for the same distributed team.
            for p in Path('/proc').iterdir():
                if not p.name.isdigit():
                    continue
                try:
                    argv = (p / 'cmdline').read_bytes().split(b'\0')[:-1]
                    if len(argv) > 1 and argv[1] == b'-u':
                        argv = [argv[0]] + argv[2:]
                    if (len(argv) >= 6 and Path(os.fsdecode(argv[1])).name == 'distributed.py'
                            and argv[2:4] == [b'_node', b'run'] and argv[5] == role.encode()
                            and os.fsdecode(argv[4]) != str(base) and p.stat().st_uid == os.getuid()):
                        raise RuntimeError('another ' + role + ' deployment is running; stop it first')
                    if argv and p.stat().st_uid == os.getuid():
                        executable = Path(os.fsdecode(argv[0])).name
                        old_bridge = role == 'v1' and executable == 'server-bridge'
                        old_worker = role != 'v1' and executable == 'distributed-solver' and b'worker' in argv
                        if old_bridge or old_worker:
                            running = process(int(p.name))
                            require(not running or Path(running['cwd']).is_relative_to(base),
                                    'A bridge/worker from another deployment is running. Stop it between matches.')
                except (FileNotFoundError, ProcessLookupError):
                    pass
            if record_file.exists():
                saved = json.loads(record_file.read_text())
                current = process(saved['pid'])
                if current and current['state'] != 'Z' and current['boot'] == saved['boot']:
                    require(matches(saved, current), 'saved PID was reused; inspect runner.json before continuing')
                    print(role + ': already running')
                    return
                require(saved['boot'] != Path('/proc/sys/kernel/random/boot_id').read_text().strip()
                        or not any(r and r['sid'] == saved['sid'] and r['uid'] == saved['uid'] and r['state'] != 'Z'
                                and Path(r['cwd']).is_relative_to(base)
                                for r in (process(int(p.name)) for p in Path('/proc').iterdir() if p.name.isdigit())),
                        'old session still has processes; stop it before restarting')
            child = subprocess.Popen(['python3', '-u', str(base / 'distributed.py'), '_node', 'run', str(base), role],
                                     cwd=base, stdin=subprocess.DEVNULL, start_new_session=True)
            try:
                pid = child.pid
                saved = process(pid)
                require(saved and saved['cwd'] == str(base), 'runner did not start')
                record_file.write_text(json.dumps(saved, indent=2) + '\n')
                time.sleep(.3)
                current = process(pid)
                require(matches(saved, current) and current['state'] != 'Z', 'runner exited; see terminal output')
                print(role + ': started PID ' + str(pid), flush=True)
            except BaseException:
                if saved:
                    stop_owned(saved, base)
                else:
                    child.terminate()
                child.wait(timeout=5)
                raise
        elif action == 'status':
            saved = json.loads(record_file.read_text()) if record_file.exists() else None
            current = process(saved['pid']) if saved else None
            same_boot = saved and saved['boot'] == Path('/proc/sys/kernel/random/boot_id').read_text().strip()
            orphans = [r['pid'] for r in (process(int(p.name)) for p in Path('/proc').iterdir() if p.name.isdigit())
                       if same_boot and r and r['pid'] != saved['pid'] and r['sid'] == saved['sid']
                       and r['uid'] == saved['uid'] and r['state'] != 'Z' and Path(r['cwd']).is_relative_to(base)]
            print(json.dumps({'role': role, 'running': bool(saved and matches(saved, current)
                              and current['state'] != 'Z'),
                              'pid': saved['pid'] if saved else None,
                              'identity_mismatch': bool(same_boot and current and current['state'] != 'Z'
                                                        and not matches(saved, current)),
                              'orphans': orphans if not matches(saved, current) else []}))
        elif action == 'stop':
            if not record_file.exists():
                print(role + ': not running')
                return
            saved = json.loads(record_file.read_text())
            stopped = stop_owned(saved, base)
            print(role + ': stopped owned PIDs ' + ','.join(map(str, stopped)))
        elif action == 'run':
            base, cfg = verify_node(base, role)
            os.chdir(base)
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'),
                   'PROCON37_G007_FUTURE_WEIGHT': '0', 'PROCON37_G007_RESERVE_WEIGHT': '15',
                   'PROCON37_DISTRIBUTED_TOKEN_FILE': str(base / 'private/cluster.token')}
            total, offset = SPECS[role]
            binary = str(base / 'bin/distributed-solver')
            allocation = cpus(cfg['nodes'][role]['cpus'])
            os.sched_setaffinity(0, allocation)
            if role != 'v1':
                arguments = [binary, '--mode', 'worker', '--threads', str(total),
                             '--worker-offset', str(offset), '--listen',
                             cfg['nodes'][role]['address'] + ':' + str(cfg['worker_port'])]
                os.execve(binary, arguments, env)
            else:
                connection = load_runtime(base)
                def interrupted(signum, frame):
                    raise KeyboardInterrupt
                signal.signal(signal.SIGTERM, interrupted)
                signal.signal(signal.SIGINT, interrupted)
                # One bridge per match, with bridge/coordinator diagnostics on the terminal.
                peers = ','.join(cfg['nodes'][r]['address'] + ':' + str(cfg['worker_port'])
                                 + '/' + str(SPECS[r][0]) + '/' + str(SPECS[r][1]) for r in ('v2', 'v3'))
                while True:
                    token = connection['token']
                    arguments = [str(base / 'bin/server-bridge'), '--server', connection['server_url'],
                                 '--token', token, '--solver', binary, '--cwd', str(base),
                                 '--record-dir', connection['record_directory'], '--',
                                 '--mode', 'coordinator', '--threads', '32', '--seed', '0', '--peers', peers]
                    if connection['select_ms'] is not None:
                        arguments[1:1] = ['--select-ms', str(connection['select_ms'])]
                    child = subprocess.Popen(arguments, cwd=base, env=env)
                    try:
                        code = child.wait()
                    finally:
                        if child.poll() is None:
                            child.terminate()
                            try:
                                child.wait(timeout=3)
                            except subprocess.TimeoutExpired:
                                child.kill()
                                child.wait(timeout=2)
                    if code == 2:
                        raise RuntimeError('competition token rejected; runner stopped')
                    if code != 0:
                        print('[run.sh] server-bridge exited unexpectedly. Restarting in one second.', flush=True)
                        time.sleep(1)
                    else:
                        print('[run.sh] Waiting for the next match.', flush=True)
        else:
            raise RuntimeError('unknown node action')
    if action == 'start' and child is not None:
        # Status/stop must remain usable while this foreground supervisor waits.
        try:
            code = child.wait()
        finally:
            stop_owned(saved, base)
            child.wait(timeout=5)
        require(code in (0, 130, -signal.SIGTERM), role + ': runner failed; see terminal output')


@contextlib.contextmanager
def operator_lock():
    directory = ROOT / '.server/distributed-production'
    directory.mkdir(parents=True, exist_ok=True)
    with (directory / 'operator.lock').open('a') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise RuntimeError('Another deployment/start/stop operation is in progress.') from None
        yield


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['compile', 'pack', 'deploy', 'start', 'status', 'stop', '_node'])
    parser.add_argument('--config', default='.server/distributed-production/config.json')
    parser.add_argument('--output', help='package output; default uses a fresh timestamp')
    parser.add_argument('--server', help='startup HTTP URL; normally SERVER in distributed/run.sh')
    parser.add_argument('--token', help='startup team token; normally TOKEN in distributed/run.sh')
    parser.add_argument('--select-ms', type=int, help='startup selection limit; normally SELECT_MS in distributed/run.sh')
    parser.add_argument('--team-token-file', help='optional startup team token file')
    parser.add_argument('node_args', nargs='*', help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.action == '_node':
        require(len(args.node_args) in (3, 4) and args.node_args[2] in SPECS, 'invalid internal node arguments')
        if args.node_args[0] == 'start':
            with terminal_signals():
                # SSH disconnection/operator death must not leave a silent worker behind.
                parent = os.getppid()
                require(ctypes.CDLL(None).prctl(1, signal.SIGHUP, 0, 0, 0) == 0,
                        'could not monitor the SSH/operator parent')
                require(os.getppid() == parent, 'SSH/operator parent exited during startup')
                node_action(*args.node_args)
        else:
            node_action(*args.node_args)
        return
    require(not (args.server or args.token or args.select_ms is not None or args.team_token_file)
            or args.action in ('start', 'pack'), 'connection options are used by run.sh/start')
    cfg = operator_config(args.config, args.action, args.server if args.action == 'pack' else None)
    if args.action != 'pack':
        require_void1(cfg)
    if args.action == 'start':
        operate(args, cfg)
    else:
        with operator_lock():
            operate(args, cfg)


def operate(args, cfg):
    if args.action == 'compile':
        compile_release(cfg, args.config)
    elif args.action in ('pack', 'deploy'):
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
        output = Path(args.output or ('.server/distributed-production/D0-' + stamp + '.tar.gz')).resolve()
        digest = package(cfg, output)
        print('package: ' + str(output) + '\nSHA256: ' + digest)
        if args.action == 'deploy':
            with ssh_auth():
                deploy(cfg, output)
    elif args.action == 'start':
        server = args.server or os.environ.get('SERVER') or cfg['server_url']
        token = (Path(args.team_token_file).read_text().strip() if args.team_token_file
                 else args.token or os.environ.get('TOKEN', ''))
        selection = args.select_ms
        if selection is None:
            selection = (int(os.environ['SELECT_MS']) if os.environ.get('SELECT_MS')
                         else None if 'SELECT_MS' in os.environ else cfg.get('select_ms'))
        settings = runtime_settings(server, token, selection, os.environ.get('RECORD_DIR'))
        with terminal_signals(), ssh_auth():
            run_cluster(cfg, settings)
    else:
        failures = []
        with ssh_auth():
            for role in ('v1', 'v2', 'v3'):
                try:
                    remote_node(cfg, role, args.action)
                except Exception:
                    failures.append(role)
                    print(role + ': ' + args.action + ' failed', file=sys.stderr)
        require(not failures, 'nodes needing attention: ' + ','.join(failures))


if __name__ == '__main__':
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
    except (RuntimeError, ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        # Never render bridge argv or token values in operator error output.
        if isinstance(error, subprocess.CalledProcessError):
            print('Operation failed (exit code ' + str(error.returncode) + ').', file=sys.stderr)
        else:
            print(str(error), file=sys.stderr)
        sys.exit(1)
