#!/usr/bin/env python3
"""同じbridgeとsolverで試合待ちを継続し、指定された提出時刻を渡す。"""
import argparse
import os
import signal
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bridge', required=True)
    parser.add_argument('--solver', required=True)
    parser.add_argument('--server', required=True)
    parser.add_argument('--team', type=int, required=True)
    parser.add_argument('--cpus', required=True)
    parser.add_argument('--submit-at-ms')
    parser.add_argument('--stream-responses', action='store_true')
    args = parser.parse_args()
    if args.stream_responses and not args.submit_at_ms:
        parser.error('--stream-responses requires --submit-at-ms')
    cpus = set()
    for part in args.cpus.split(','):
        bounds = list(map(int, part.split('-')))
        cpus.update(range(bounds[0], bounds[-1] + 1))
    os.sched_setaffinity(0, cpus)
    stopping = False
    child = None

    def stop(_signal, _frame):
        nonlocal stopping
        stopping = True
        if child is not None and child.poll() is None:
            child.terminate()

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    environment = {'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8'}
    command = [args.bridge, '--server', args.server, '--token', f'token-p{args.team}', '--solver', args.solver]
    if args.submit_at_ms:
        command += ['--submit-at-ms', args.submit_at_ms]
    if args.stream_responses:
        command += ['--stream-responses']
    while not stopping:
        child = subprocess.Popen(command, env=environment)
        result = child.wait()
        print(f'[team-runner] bridge exited {result}', flush=True)
        if result == 2:
            raise SystemExit(2)
        if not stopping:
            time.sleep(1)


if __name__ == '__main__':
    main()
