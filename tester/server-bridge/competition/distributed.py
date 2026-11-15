#!/usr/bin/env python3
"""Deploy exact evaluated streaming binaries into a separate competition release.

runtime_base.py reuses the established D0 process controller with wired mDNS support.
Its PID/boot/UID/cwd verification, pidfd shutdown and SSH parent-loss cleanup are reused.
"""
import argparse
import ctypes
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import socket
import subprocess
import sys
import tarfile
import tempfile
import time
import runtime_base as base

HERE = Path(__file__).resolve().parent
base.ROOT = HERE
base.SSH_PASSWORD_PATH = HERE / 'private/ssh-password'
FILES = {'bin/distributed-solver', 'bin/server-bridge', 'distributed.py', 'runtime_base.py',
         'console.py', 'progress.py', 'run.sh', 'node-config.json', 'selection.json'}
BRIDGE_SHA256 = '62382f216508f7840904f133933388e9565cb76008c4980acc7fdbd874f3a0a8'
SCHEDULE = '5000,15000,25000,35000,45000,55000'
SELECTION_DURATIONS = (20000, 30000, 60000, 90000, 120000)
OPERATOR_ENTRY = r'''#!/bin/sh
# 当日の本番設定: VOIDLINUX-1でこの2行の値を書き換える。
SERVER="${SERVER:-}"
TOKEN="${TOKEN:-}"
# 任意: TOKEN_FILEまたはPROCON37_SSH_PASSWORDも環境変数で指定できる。
# release配下のrun.shは編集しない。設定はここから渡す。
set -eu
export SERVER TOKEN
BASE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec "$BASE/current/run.sh" "$@"
'''
ORIGINAL_NODE_ACTION = base.node_action
ORIGINAL_START_CLUSTER = base.start_cluster
require = base.require


def verify_node(directory, role, expected_config=None):
    root = Path(directory).resolve()
    manifest = json.loads((root / 'manifest.json').read_text())
    require(set(manifest) == FILES, 'invalid competition release manifest')
    for name, digest in manifest.items():
        path = root / name
        require(path.is_file() and not path.is_symlink() and base.sha(path) == digest,
                'release file changed: ' + name)
    cfg = base.load_config(root / 'node-config.json', allow_placeholder=True)
    require(cfg['remote_directory'].startswith('procon37-competition/releases/'),
            'competition releases must have a separate destination')
    require(cfg['bridge_sha256'] == BRIDGE_SHA256, 'bridge is not the evaluated submission implementation')
    if expected_config:
        require(base.sha(root / 'node-config.json') == expected_config, 'release configuration changed')
    for name, relative in [('solver', 'bin/distributed-solver'), ('bridge', 'bin/server-bridge')]:
        require(base.sha(root / relative) == cfg[name + '_sha256'], 'binary identity mismatch: ' + name)
        require(os.access(root / relative, os.X_OK), 'binary is not executable: ' + name)
    require(sys.version_info >= (3, 10) and hasattr(os, 'pidfd_open'), 'Linux/Python 3.10+ pidfd required')
    require(platform.machine() == 'x86_64', 'x86_64 Linux required')
    require((os.confstr('CS_GNU_LIBC_VERSION') or '').startswith('glibc '), 'glibc required')
    require(base.cpus(cfg['nodes'][role]['cpus']) <= os.sched_getaffinity(0), 'requested CPUs unavailable')
    shared_file = root / 'private/cluster.token'
    require(shared_file.is_file() and not shared_file.is_symlink()
            and shared_file.stat().st_uid == os.getuid() and shared_file.stat().st_mode & 0o077 == 0,
            'private cluster token ownership/permissions mismatch')
    shared = shared_file.read_text().strip()
    require(32 <= len(shared) <= 128 and all(33 <= ord(c) <= 126 for c in shared), 'invalid cluster token')
    selection = json.loads((root / 'selection.json').read_text())
    require(selection['solver_sha256'] == cfg['solver_sha256']
            and selection['bridge_sha256'] == cfg['bridge_sha256'], 'selection/binary identity mismatch')
    return root, cfg


def worker_ready(cfg, role, shared):
    def receive(sock, count):
        data = b''
        while len(data) < count:
            part = sock.recv(count - len(data))
            require(part, 'worker closed during authentication')
            data += part
        return data
    request = json.dumps({'Authenticate': {'version': 4, 'token': shared, 'cross_elite': True}}).encode()
    with socket.create_connection((base.node_ipv4(cfg['nodes'][role]), cfg['worker_port']), timeout=3) as sock:
        sock.sendall(len(request).to_bytes(4, 'big') + request)
        length = int.from_bytes(receive(sock, 4), 'big')
        require(1 <= length <= 4096, 'invalid worker Ready frame')
        reply = json.loads(receive(sock, length))
    total, offset = base.SPECS[role]
    require(reply == {'Ready': {'version': 4, 'total_threads': total, 'offset': offset, 'cross_elite': True}},
            role + ': worker protocol/cross-elite/thread/offset mismatch')
    print(role + ': wired TCP authentication OK (' + str(total - 1) + ' search threads)', flush=True)


def solver_arguments(cfg, role):
    total, offset = base.SPECS[role]
    if role != 'v1':
        return ['--mode', 'worker', '--threads', str(total), '--worker-offset', str(offset),
                '--cross-elite', 'on', '--listen', base.node_ipv4(cfg['nodes'][role]) + ':' + str(cfg['worker_port'])]
    peers = ','.join(base.node_ipv4(cfg['nodes'][peer]) + ':' + str(cfg['worker_port'])
                     + '/' + str(base.SPECS[peer][0]) + '/' + str(base.SPECS[peer][1]) for peer in ('v2', 'v3'))
    return ['--mode', 'coordinator', '--threads', '32', '--seed', '0', '--peers', peers,
            '--cross-elite', 'on', '--stream-responses']


def submission_schedule(selection):
    require(selection is None or selection in SELECTION_DURATIONS,
            'select_ms must match test/practice/match/match24/match32 duration')
    # Keep existing modes unchanged. The evaluated bridge clips checkpoints to
    # each phase's deadline, so 60-second action phases cannot run for 90/120s.
    # The bridge adds final opportunities 5 and 3 seconds before each deadline.
    if selection is not None and selection > 60000:
        return ','.join(str(ms) for ms in range(5000, selection, 10000))
    return SCHEDULE


def bridge_arguments(root, cfg, settings):
    selection = settings['select_ms']
    schedule = submission_schedule(selection)
    return [str(root / 'bin/server-bridge'), '--server', settings['server_url'], '--token', settings['token'],
            '--solver', str(root / 'bin/distributed-solver'), '--cwd', str(root),
            '--record-dir', str(root / 'records'), '--submit-at-ms', schedule, '--stream-responses',
            *([] if selection is None else ['--select-ms', str(selection)]), '--',
            *solver_arguments(cfg, 'v1')]


def node_action(action, directory, role, expected_config=None):
    if action != 'run':
        return ORIGINAL_NODE_ACTION(action, directory, role, expected_config)
    root, cfg = verify_node(directory, role)
    os.chdir(root)
    os.sched_setaffinity(0, base.cpus(cfg['nodes'][role]['cpus']))
    environment = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'),
                   'PROCON37_G007_FUTURE_WEIGHT': '0', 'PROCON37_G007_RESERVE_WEIGHT': '15',
                   'PROCON37_DISTRIBUTED_TOKEN_FILE': str(root / 'private/cluster.token')}
    if role != 'v1':
        os.execve(root / 'bin/distributed-solver', [str(root / 'bin/distributed-solver'),
                                                  *solver_arguments(cfg, role)], environment)
    settings = base.load_runtime(root)
    def interrupted(_signum, _frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    while True:
        child = subprocess.Popen(bridge_arguments(root, cfg, settings), cwd=root, env=environment)
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
        require(code != 2, 'competition token rejected; runner stopped')
        if code != 0:
            print('[competition] bridge_restart exit_code=' + str(code), flush=True)
            time.sleep(1)


def start_cluster(cfg, settings=None):
    # The inherited cleanup owns newly started runners only. Never adopt a partial
    # cluster, whose pre-existing worker would otherwise survive this operator's exit.
    statuses = [json.loads(base.remote_node(cfg, role, 'status', capture_output=True, text=True).stdout)
                for role in base.SPECS]
    running = sum(bool(status['running']) for status in statuses)
    if running not in (0, 3):
        print('[competition] start_refused reason=partial_cluster', file=sys.stderr)
        raise RuntimeError('Cluster is partially running. Run run.sh stop before starting all three nodes.')
    return ORIGINAL_START_CLUSTER(cfg, settings)


base.verify_node = verify_node
base.worker_ready = worker_ready
base.node_action = node_action
base.start_cluster = start_cluster


def package(config, output, evidence, candidate):
    cfg = base.load_config(config, allow_placeholder=True)
    require(cfg['bridge_sha256'] == BRIDGE_SHA256, 'unexpected bridge binary identity')
    require(cfg['remote_directory'].startswith('procon37-competition/releases/'), 'invalid competition release path')
    require(cfg['worker_port'] != 39370, 'competition must not use the evaluation worker port')
    require(re.fullmatch(r'[A-Za-z0-9_.-]+', candidate), 'invalid candidate identifier')
    evidence = Path(evidence).resolve(strict=True)
    evidence_bytes = evidence.read_bytes()
    evidence_json = json.loads(evidence_bytes)
    # The supplied frozen preflight/deployment/result record must identify this exact binary.
    def values(value):
        if isinstance(value, dict):
            for item in value.values():
                yield from values(item)
        elif isinstance(value, list):
            for item in value:
                yield from values(item)
        else:
            yield value
    require(cfg['solver_sha256'] in set(str(value) for value in values(evidence_json)),
            'the evaluation evidence does not identify the selected solver SHA256')
    require(cfg['bridge_sha256'] in set(str(value) for value in values(evidence_json)),
            'the evaluation evidence does not identify the selected bridge SHA256')
    output = Path(output).resolve()
    require(not output.exists(), 'package exists; use a new name')
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='competition-package-') as temp:
        root = Path(temp)
        (root / 'bin').mkdir()
        for key, name in [('solver', 'distributed-solver'), ('bridge', 'server-bridge')]:
            source = Path(cfg[key + '_binary']).resolve(strict=True)
            require(source.is_file() and not Path(cfg[key + '_binary']).is_symlink(), 'invalid binary input')
            require(base.sha(source) == cfg[key + '_sha256'], 'input binary identity mismatch: ' + key)
            require(source.read_bytes()[:6] == b'\x7fELF\x02\x01', '64-bit ELF required')
            shutil.copyfile(source, root / 'bin' / name)
            (root / 'bin' / name).chmod(0o755)
        for name in ('distributed.py', 'runtime_base.py', 'console.py', 'progress.py', 'run.sh'):
            shutil.copyfile(HERE / name, root / name)
        (root / 'run.sh').chmod(0o755)
        (root / 'node-config.json').write_bytes(base.config_bytes(cfg))
        selection = {'candidate': candidate, 'solver_sha256': cfg['solver_sha256'],
                     'bridge_sha256': cfg['bridge_sha256'],
                     'evidence_file': evidence.name, 'evidence_sha256': hashlib.sha256(evidence_bytes).hexdigest(),
                     'evidence': evidence_json,
                     'built_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                     'submit_at_ms': SCHEDULE, 'wire_version': 4, 'cross_elite': True,
                     'submit_at_ms_by_select_ms': {str(ms): submission_schedule(ms) for ms in SELECTION_DURATIONS},
                     'final_checkpoint_margin_ms': 5000, 'backup_checkpoint_margin_ms': 3000, 'submission_margin_ms': 2000,
                     'threads': [32, 16, 32], 'note': 'Packaging preserves the evaluated executables; no compilation.'}
        (root / 'selection.json').write_text(json.dumps(selection, indent=2, ensure_ascii=False) + '\n')
        (root / 'manifest.json').write_text(json.dumps({name: base.sha(root / name) for name in sorted(FILES)}, indent=2) + '\n')
        with tarfile.open(output, 'x:gz') as archive:
            for name in sorted(FILES | {'manifest.json'}):
                archive.add(root / name, arcname=name, recursive=False)
    print(json.dumps({'archive': str(output), 'sha256': base.sha(output), 'candidate': candidate}))


def configure_operator_auth(cfg):
    # Runtime-only credential, excluded from the package/manifest/evaluation records.
    password = os.environ.get('PROCON37_SSH_PASSWORD')
    if password:
        require('\n' not in password and '\r' not in password, 'invalid SSH password format')
        directory = Path.home() / cfg['remote_directory'] / 'private'
        require(directory.is_dir() and not directory.is_symlink(), 'private credential directory missing')
        path = directory / 'ssh-password'
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, 'w') as output:
            output.write(password + '\n')


def activate(cfg):
    # Stable operator entry is owned by this tool; never replace the existing D0 run.sh.
    for role in base.SPECS:
        base.remote_node(cfg, role, 'check')
    release = cfg['remote_directory'].removeprefix('procon37-competition/')
    script = OPERATOR_ENTRY
    for role in base.SPECS:
        code = ('from pathlib import Path\nimport os\n'
                'root=Path.home()/"procon37-competition"\n'
                'entry=root/"run.sh"\n'
                f'content={script!r}\n'
                'assert not entry.exists() or entry.read_text()==content,"refusing to replace unknown entrypoint"\n'
                'current=root/"current"\n'
                f'target={release!r}\n'
                'assert not current.exists() or current.is_symlink(),"current is not a symlink"\n'
                'assert not current.is_symlink() or os.readlink(current)==target,"a different release is already selected"\n'
                'if not current.is_symlink(): current.symlink_to(target,target_is_directory=True)\n'
                'entry.write_text(content)\nentry.chmod(0o700)\n'
                'print(str(entry))\n')
        base.remote(cfg, role, 'python3 -c ' + base.shlex.quote(code))


def probe(cfg):
    """No HTTP and no Solve: authenticate idle workers, then stop every owned process."""
    root = Path.home() / cfg['remote_directory']
    runners = {}
    try:
        for role in base.SPECS:
            base.remote_node(cfg, role, 'check')
            status = json.loads(base.remote_node(cfg, role, 'status', capture_output=True, text=True).stdout)
            require(not status['running'] and not status['identity_mismatch'] and not status['orphans'],
                    role + ': probe requires an idle release')
        for role in ('v2', 'v3'):
            runners[role] = base.spawn_node(cfg, role)
            base.wait_node_started(cfg, role, runners[role])
        shared = (root / 'private/cluster.token').read_text().strip()
        for role in ('v2', 'v3'):
            worker_ready(cfg, role, shared)
        environment = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'),
                       'PROCON37_G007_FUTURE_WEIGHT': '0', 'PROCON37_G007_RESERVE_WEIGHT': '15',
                       'PROCON37_DISTRIBUTED_TOKEN_FILE': str(root / 'private/cluster.token')}
        base.run(['taskset', '-c', cfg['nodes']['v1']['cpus'], str(root / 'bin/distributed-solver'),
                  *solver_arguments(cfg, 'v1')], cwd=root, env=environment,
                 stdin=subprocess.DEVNULL, timeout=10)
        print('[competition] probe_ok wire=4 cross_elite=on threads=32,16,32 requests=0 http=0')
    finally:
        failures = base.stop_runners(cfg, runners)
        require(not failures, 'probe cleanup requires attention: ' + ','.join(failures))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['package', 'deploy', 'activate', 'probe', 'start', 'status', 'stop', '_node'])
    parser.add_argument('--config', type=Path, default=HERE / 'node-config.json')
    parser.add_argument('--archive', type=Path)
    parser.add_argument('--evidence', type=Path)
    parser.add_argument('--candidate')
    parser.add_argument('node_args', nargs='*', help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.action == '_node':
        require(len(args.node_args) in (3, 4) and args.node_args[2] in base.SPECS, 'invalid node arguments')
        if args.node_args[0] == 'start':
            with base.terminal_signals():
                parent = os.getppid()
                require(ctypes.CDLL(None).prctl(1, signal.SIGHUP, 0, 0, 0) == 0, 'SSH parent monitor unavailable')
                require(os.getppid() == parent, 'SSH/operator parent already exited')
                node_action(*args.node_args)
        else:
            node_action(*args.node_args)
        return
    if args.action == 'package':
        require(args.archive and args.evidence and args.candidate, 'package needs --archive --evidence --candidate')
        package(args.config, args.archive, args.evidence, args.candidate)
        return
    cfg = base.load_config(args.config, allow_placeholder=True)
    base.require_void1(cfg)
    if args.action == 'start':
        if not (os.environ.get('SERVER') and (os.environ.get('TOKEN') or os.environ.get('TOKEN_FILE'))):
            print('[competition] start_refused reason=endpoint_unset', file=sys.stderr)
            raise RuntimeError('Set the actual competition SERVER and TOKEN (or TOKEN_FILE); no default server is contacted.')
        token = Path(os.environ['TOKEN_FILE']).read_text().strip() if os.environ.get('TOKEN_FILE') else os.environ['TOKEN']
        selection = os.environ.get('SELECT_MS', '')
        require(selection == '' or selection in tuple(str(ms) for ms in SELECTION_DURATIONS),
                'Invalid SELECT_MS: use 20000, 30000, 60000, 90000, or 120000')
        settings = base.runtime_settings(os.environ['SERVER'], token, int(selection) if selection else None,
                                         Path.home() / cfg['remote_directory'] / 'records')
        with base.terminal_signals(), base.ssh_auth():
            base.run_cluster(cfg, settings)
    else:
        with base.operator_lock(), base.ssh_auth():
            if args.action == 'deploy':
                require(args.archive and args.archive.is_file(), 'deploy needs an existing --archive')
                # Verify the archive configuration before copying anything to another PC.
                with tarfile.open(args.archive) as archive:
                    members = archive.getmembers()
                    require(len(members) == len(FILES) + 1 and all(member.isfile() for member in members)
                            and set(archive.getnames()) == FILES | {'manifest.json'}, 'unexpected archive contents')
                    manifest = json.load(archive.extractfile('manifest.json'))
                    require(set(manifest) == FILES, 'archive manifest fields mismatch')
                    for name, digest in manifest.items():
                        require(hashlib.sha256(archive.extractfile(name).read()).hexdigest() == digest,
                                'archive manifest mismatch: ' + name)
                    require(archive.extractfile('node-config.json').read() == base.config_bytes(cfg), 'archive configuration mismatch')
                base.deploy(cfg, args.archive)
                configure_operator_auth(cfg)
            elif args.action == 'activate':
                activate(cfg)
            elif args.action == 'probe':
                probe(cfg)
            else:
                failures = []
                for role in base.SPECS:
                    try:
                        base.remote_node(cfg, role, args.action)
                    except Exception:
                        failures.append(role)
                require(not failures, 'nodes requiring attention: ' + ','.join(failures))


if __name__ == '__main__':
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
    except (RuntimeError, ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        print('Operation failed (exit ' + str(error.returncode) + ')' if isinstance(error, subprocess.CalledProcessError)
              else str(error), file=sys.stderr)
        sys.exit(1)
