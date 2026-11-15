#!/usr/bin/env python3
"""Finite GPU-lab/D0 CPU coexistence microbenchmark; never connects to a match.

Invoke as root on VOID1 or VOID3. Each child runs as the normal experiment user;
the existing reviewed controller confines GPU lab descendants to CPU31. D0 uses
CPU0-30 with 30 search workers plus its control thread. Fresh labels are required.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import pwd
import re
import signal
import subprocess
import time


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def snapshot(pid):
    proc = Path('/proc') / str(pid)
    try:
        stat = proc.joinpath('stat').read_text().rsplit(')', 1)[1].split()
        statuses = []
        for task in proc.joinpath('task').iterdir():
            try:
                fields = dict(line.split(':', 1) for line in task.joinpath('status').read_text().splitlines() if ':' in line)
                statuses.append({'tid': int(task.name), 'cpu_allowed': fields['Cpus_allowed_list'].strip(), 'state': fields['State'].strip()})
            except (FileNotFoundError, ProcessLookupError):
                pass
        return {'at_monotonic': time.monotonic(), 'ticks': int(stat[11]) + int(stat[12]), 'threads': statuses}
    except (FileNotFoundError, ProcessLookupError):
        return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--host', choices=['v1', 'v3'], required=True)
    parser.add_argument('--label', required=True)
    args = parser.parse_args()
    if os.geteuid() != 0 or not re.fullmatch(args.host + r'-r3-resume20261009-coload-r[0-9]+', args.label):
        parser.error('root and a fresh matching coexistence label are required')
    account = pwd.getpwnam('voidlinux-' + args.host[1:])
    root = Path(account.pw_dir) / 'procon37-distributed'
    output = root / 'gpu-lab-compact-r3' / 'results' / (args.label + '-coload')
    if list(output.parent.glob(output.name + '.*')):
        parser.error('existing result label')
    binary = root / 'bin/distributed-solver'
    assert digest(binary) == 'e80474cc5f65a30b18d8143eec656e3abe093c927d75080aed2bd9dce72a5de0'
    source = root / ('benchmark-request.json' if args.host == 'v1' else 'resume20261009-benchmark-request.json')
    request = json.loads(source.read_text())
    request['timeLimitMs'] = 60000
    payload = (json.dumps(request, separators=(',', ':')) + '\n').encode()
    output.with_suffix('.request.json').write_bytes(payload)

    def as_user():
        os.sched_setaffinity(0, set(range(31)))
        os.initgroups(account.pw_name, account.pw_gid)
        os.setgid(account.pw_gid)
        os.setuid(account.pw_uid)

    environment = dict(os.environ, HOME=account.pw_dir, USER=account.pw_name, LOGNAME=account.pw_name)
    environment.update(PROCON37_G007_FUTURE_WEIGHT='0', PROCON37_G007_RESERVE_WEIGHT='15')
    command = [str(binary), '--mode', 'parallel', '--threads', '31', '--seed', '0']
    gpu_command = ['python3', str(root / 'run_cpuset_profile.py'), '--label', args.label]
    if args.host == 'v1':
        gpu_command += ['--host', 'v1', '--profiles', 'both']
    record = {
        'kind': 'resource-coexistence microbenchmark, not a match or adoption evaluation',
        'host': args.host, 'label': args.label, 'started_unix': time.time(),
        'cpu_load_command': command, 'cpu_load_sha256': digest(binary),
        'request_sha256': hashlib.sha256(payload).hexdigest(), 'request_limit_ms': 60000,
        'cpu_load_affinity': '0-30', 'cpu_load_search_threads': 30, 'cpu_load_total_threads': 31,
        'gpu_affinity': '31', 'gpu_command': gpu_command,
        'controller_sha256': digest(root / 'run_cpuset_profile.py'),
        'probe_sha256': digest(Path(__file__)), 'clock_ticks_per_second': os.sysconf('SC_CLK_TCK'),
        'gpu_sampled_cpu_load': [], 'forced_cleanup': [],
    }
    load = gpu = None
    with output.with_suffix('.stdout').open('wb') as out, output.with_suffix('.stderr').open('wb') as err, output.with_suffix('.gpu-launch.log').open('wb') as gpu_log:
        try:
            load = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=out, stderr=err,
                                    env=environment, start_new_session=True, preexec_fn=as_user)
            record['cpu_load_pid'] = load.pid
            load.stdin.write(payload)
            load.stdin.close()
            time.sleep(2)
            assert load.poll() is None, 'CPU load ended before GPU measurement'
            record['load_before_gpu'] = snapshot(load.pid)
            gpu = subprocess.Popen(gpu_command, stdout=gpu_log, stderr=subprocess.STDOUT,
                                   stdin=subprocess.DEVNULL, env=environment)
            record['gpu_controller_pid'] = gpu.pid
            deadline = time.monotonic() + 90
            while gpu.poll() is None:
                current = snapshot(load.pid)
                if current:
                    record['gpu_sampled_cpu_load'].append(current)
                if time.monotonic() > deadline:
                    raise TimeoutError('GPU profile exceeded 90 seconds')
                time.sleep(0.2)
            record['gpu_exit_code'] = gpu.wait()
            record['cpu_load_alive_after_gpu'] = load.poll() is None
            record['load_after_gpu'] = snapshot(load.pid)
            record['cpu_load_exit_code'] = load.wait(timeout=70)
        finally:
            # Only direct children owned by this finite probe can be signalled.
            for name, process in [('gpu_controller', gpu), ('cpu_load', load)]:
                if process is not None and process.poll() is None:
                    process.terminate()
                    record['forced_cleanup'].append({'name': name, 'pid': process.pid, 'signal': 'SIGTERM'})
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        record['forced_cleanup'].append({'name': name, 'pid': process.pid, 'signal': 'SIGKILL'})
                        process.wait()
            record['ended_unix'] = time.time()
            output.with_suffix('.json').write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps({k: v for k, v in record.items() if k != 'gpu_sampled_cpu_load'}))
    if record.get('gpu_exit_code') or record.get('cpu_load_exit_code') or not record.get('cpu_load_alive_after_gpu') or record['forced_cleanup']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
