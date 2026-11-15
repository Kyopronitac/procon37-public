#!/usr/bin/env python3
"""既定.server/runsへ4チーム12件を分離起動し、再開情報を残す。"""
import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage')
    parser.add_argument('--label', required=True)
    parser.add_argument('--short-ms', type=int)
    args = parser.parse_args()
    directory = ROOT / '.server/distributed'
    directory.mkdir(parents=True, exist_ok=True)
    state_path = ROOT / '.server/distributed-state.json'
    state = json.loads(state_path.read_text())
    old = state.get('evaluation', {})
    if old.get('pid'):
        try:
            cmdline = Path(f"/proc/{old['pid']}/cmdline").read_bytes()
        except FileNotFoundError:
            cmdline = b''
        assert not (b'server' in cmdline and b'run' in cmdline), '前の評価が進行中'
    config = {'count': 12, 'players': 4, 'format': None, 'arenaMaps': True,
              'timeLimitMs': args.short_ms, 'label': args.label,
              'teamNames': ['分散80スレッド', '固定並列16スレッド', '固定第6世代', '固定第8世代']}
    config_path = directory / f'{args.stage}-config.json'
    config_path.write_text(json.dumps(config, ensure_ascii=False, indent=2) + '\n')
    command = [str(ROOT / 'target/release/server'), 'run', '12', '--players', '4',
               '--config', str(config_path), '--label', args.label]
    if args.short_ms is not None:
        command += ['--time-limit-ms', str(args.short_ms)]
    log_path = directory / f'{args.stage}.log'
    assert not log_path.exists(), '同じstageの記録を上書きしない'
    preflight_path = ROOT / 'solver/docs/distributed' / f'{args.stage}-preflight.json'
    subprocess.run([sys.executable, str(Path(__file__).with_name('snapshot.py')),
                    '--save', str(preflight_path)], cwd=ROOT, check=True)
    with log_path.open('wb') as log:
        process = subprocess.Popen(command, cwd=ROOT, stdin=subprocess.DEVNULL,
                                   stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    evaluation = dict(stage=args.stage, pid=process.pid, startUnix=time.time(),
                      command=command, config=str(config_path), log=str(log_path),
                      history=str(ROOT / '.server/runs'), preflight=str(preflight_path), expectedAllAC=48)
    (directory / f'{args.stage}-process.json').write_text(json.dumps(evaluation, ensure_ascii=False, indent=2) + '\n')
    state.update(stage='evaluating', evaluation=evaluation)
    state_path.write_text(json.dumps(state, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(evaluation, ensure_ascii=False, indent=2))
