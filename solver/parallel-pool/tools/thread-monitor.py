#!/usr/bin/env python3
"""実行中の対象バイナリについて内部スレッド数とCPU集合を実測する。"""
import argparse
import json
import os
import time
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--root', required=True)
parser.add_argument('--seconds', type=float, default=240)
parser.add_argument('--output', required=True)
args = parser.parse_args()
root = Path(args.root).resolve()
records = {}
start = time.time()
while time.time() - start < args.seconds:
    for proc in Path('/proc').iterdir():
        if not proc.name.isdigit():
            continue
        try:
            exe = Path(os.readlink(proc / 'exe'))
            if not exe.is_relative_to(root / 'teams') or exe.name != 'solver':
                continue
            status = dict(line.split(':', 1) for line in (proc / 'status').read_text().splitlines() if ':' in line)
            slot = exe.parent.name
            row = records.setdefault(slot, {'maxThreads': 0, 'maxRssKiB': 0, 'pids': [], 'cpuSets': []})
            row['maxThreads'] = max(row['maxThreads'], int(status['Threads']))
            row['maxRssKiB'] = max(row['maxRssKiB'], int(status.get('VmRSS', '0 kB').split()[0]))
            pid = int(proc.name)
            if pid not in row['pids']:
                row['pids'].append(pid)
            cpu = status['Cpus_allowed_list'].strip()
            if cpu not in row['cpuSets']:
                row['cpuSets'].append(cpu)
        except (OSError, ValueError, KeyError):
            continue
    Path(args.output).write_text(json.dumps({'startedAt': start, 'observedAt': time.time(), 'teams': records}, indent=2) + '\n')
    time.sleep(0.1)
