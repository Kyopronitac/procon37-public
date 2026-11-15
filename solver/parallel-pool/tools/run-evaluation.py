#!/usr/bin/env python3
"""切断後も評価プロセスを継続し、終了コードを別ファイルに残す。"""
import argparse
import json
import os
import subprocess
import time
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--label', required=True)
parser.add_argument('--count', type=int, default=12)
parser.add_argument('--time-limit-ms', type=int)
parser.add_argument('--output-dir')
parser.add_argument('--record', required=True)
args = parser.parse_args()
root = Path(__file__).resolve().parents[3]
cmd = ['/home/jikky/.cargo/bin/cargo', 'run', '--release', '-p', 'server', '--',
       'run', str(args.count), '--players', '6', '--label', args.label]
if args.time_limit_ms is not None:
    assert args.output_dir, '短縮評価は一時保存先を使う'
    cmd += ['--time-limit-ms', str(args.time_limit_ms)]
if args.output_dir:
    cmd += ['--output-dir', args.output_dir]
env = os.environ.copy()
env['PATH'] = '/home/jikky/.cargo/bin:' + env['PATH']
record = Path(args.record)
record.parent.mkdir(parents=True, exist_ok=True)
proc = subprocess.Popen(cmd, cwd=root, env=env)
status = {'command': cmd, 'cwd': str(root), 'label': args.label,
          'pid': proc.pid, 'wrapperPid': os.getpid(), 'startedAt': time.time(), 'exitCode': None}
record.write_text(json.dumps(status, ensure_ascii=False, indent=2) + '\n')
status['exitCode'] = proc.wait()
status['finishedAt'] = time.time()
record.write_text(json.dumps(status, ensure_ascii=False, indent=2) + '\n')
raise SystemExit(status['exitCode'])
