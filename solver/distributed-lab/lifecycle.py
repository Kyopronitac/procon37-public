#!/usr/bin/env python3
"""登録した専用チームを起動・照合・停止する。solver実装や配置は変更しない。"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'solver/distributed/tools'))
from ssh import execute

# 同じ処理をローカルとSSH先で実行する。秘密情報を受け取らず、起動環境を限定する。
PROGRAM = r'''
import hashlib,json,os,pathlib,signal,subprocess,time
spec=SPECIFICATION
action=ACTION
directory=pathlib.Path(spec['directory'])
record_path=directory/'resume-team-process.json'
def identity(pid):
    try:
        p=pathlib.Path('/proc')/str(pid)
        stat=(p/'stat').read_text().rsplit(')',1)[1].split()
        return {'pid':pid,'startTicks':stat[19],'state':stat[0],
                'bootId':pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
                'session':int(stat[3]),'uid':p.stat().st_uid}
    except (FileNotFoundError,ProcessLookupError):return None
def active(record):
    actual=identity(record['pid']) if record else None
    return bool(actual and actual['state']!='Z' and all(actual[k]==record.get(k) for k in ('pid','startTicks','bootId','session','uid')))
old=json.loads(record_path.read_text()) if record_path.exists() else None
if action=='start':
    if active(old):raise SystemExit('dedicated process already active')
    for name,expected in spec['files'].items():
        if hashlib.sha256((directory/name).read_bytes()).hexdigest()!=expected:
            raise SystemExit('file hash mismatch: '+name)
    for p in pathlib.Path('/proc').iterdir():
        if not p.name.isdigit():continue
        try:
            argv=(p/'cmdline').read_bytes().decode(errors='replace').split(chr(0))
            if any(x['matchArg'] in argv for x in spec['processes']):
                raise SystemExit('untracked required process already active: '+p.name)
        except (FileNotFoundError,ProcessLookupError,PermissionError):pass
    cpus=set()
    for part in spec['cpus'].split(','):
        bounds=list(map(int,part.split('-')));cpus.update(range(bounds[0],bounds[-1]+1))
    os.sched_setaffinity(0,cpus)
    with (directory/'resume-team.log').open('ab') as log:
        child=subprocess.Popen(spec['launch'],cwd=directory,env={'PATH':'/usr/bin:/bin','LANG':'C.UTF-8'},
                               stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    record=identity(child.pid)
    if record is None:raise SystemExit('process exited immediately')
    record.update(command=spec['launch'],startedUnix=time.time(),log=str(directory/'resume-team.log'))
    record_path.write_text(json.dumps(record,indent=2)+'\n')
    time.sleep(0.3)
    if child.poll() is not None:raise SystemExit('process exited at startup; inspect log')
    print('__RESULT__'+json.dumps(record))
elif action=='stop':
    if active(old):
        if old['session']!=old['pid'] or old['uid']!=os.getuid():raise SystemExit('unsafe process identity')
        os.killpg(old['pid'],signal.SIGTERM)
        deadline=time.monotonic()+5
        while active(old) and time.monotonic()<deadline:time.sleep(0.05)
        if active(old):os.killpg(old['pid'],signal.SIGKILL)
    print('__RESULT__'+json.dumps({'record':old,'alive':active(old)}))
else:print('__RESULT__'+json.dumps({'record':old,'alive':active(old)}))
'''


def run_spec(host, spec, action):
    code = PROGRAM.replace('SPECIFICATION', repr(spec), 1).replace('ACTION', repr(action), 1)
    if host == 'local':
        result = subprocess.run([sys.executable, '-c', code], capture_output=True, text=True, check=True).stdout
    else:
        result = execute(host, 'python3 -c ' + shlex.quote(code))
    return json.loads(result.split('__RESULT__')[-1].strip())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['start', 'stop', 'status'])
    parser.add_argument('--config', default='.server/distributed/resume-config.json')
    parser.add_argument('--distributed-only', action='store_true')
    args = parser.parse_args()
    config = json.loads((ROOT / args.config).read_text())
    state_path = ROOT / '.server/distributed-state.json'
    state = json.loads(state_path.read_text()) if state_path.exists() else {}
    if args.action != 'status' and state.get('evaluation', {}).get('status') in ('starting', 'running'):
        raise SystemExit('評価実行中。まず評価を終了・確定してください。')
    order = ['v2', 'v3', 'v1'] + ([] if args.distributed_only else ['nomlab', 'local2', 'local3'])
    if args.action == 'stop':
        order.reverse()
    result = {}
    started = []
    try:
        for host in order:
            spec = config['localTeams'][host] if host.startswith('local') else config['nodes'][host]
            result[host] = run_spec('local' if host.startswith('local') else host, spec, args.action)
            if args.action == 'start':
                started.append((host, spec))
    except Exception:
        for host, spec in reversed(started):
            try:
                run_spec('local' if host.startswith('local') else host, spec, 'stop')
            except Exception as error:
                print(f'cleanup failed {host}: {error}', file=sys.stderr)
        raise
    record = ROOT / '.server/distributed' / ('lifecycle-' + args.action + '.json')
    record.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
