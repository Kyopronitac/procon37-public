#!/usr/bin/env python3
"""Read-only participation, clock and bounded log capture for one evaluation."""
import argparse
import concurrent.futures
import json
from pathlib import Path
import shlex
import subprocess
import sys

import lifecycle
import manage

PROGRAM = r'''
import pathlib,json,time,os,hashlib,subprocess,datetime
node=NODE
phase=PHASE
marker=MARKER
clock_namespace={'__name__':'embedded_clock'}
exec(CLOCK_SOURCE,clock_namespace)
root=pathlib.Path(node['directory'])
def processes():
    found=[]
    for p in pathlib.Path('/proc').iterdir():
        if not p.name.isdigit() or int(p.name)==os.getpid():continue
        try:
            args=(p/'cmdline').read_bytes().decode(errors='replace').split(chr(0))
            if not any(a.startswith(str(root)+'/') for a in args):continue
            for i,a in enumerate(args[:-1]):
                if a in ('--token','--auth-token'):args[i+1]='[redacted]'
            stat=(p/'stat').read_text().rsplit(')',1)[1].split()
            status=dict(line.split(':',1) for line in (p/'status').read_text().splitlines() if ':' in line)
            threads=[]
            for task in (p/'task').iterdir():
                try:
                    ts=(task/'stat').read_text().rsplit(')',1)[1].split()
                    st=dict(line.split(':',1) for line in (task/'status').read_text().splitlines() if ':' in line)
                    threads.append({'tid':int(task.name),'name':(task/'comm').read_text().strip(),
                                    'ticks':int(ts[11])+int(ts[12]),'cpus':st['Cpus_allowed_list'].strip()})
                except (FileNotFoundError,ProcessLookupError):pass
            found.append({'pid':int(p.name),'startTicks':stat[19],'state':stat[0],'argv':args,
                          'cpus':status['Cpus_allowed_list'].strip(),'threads':threads})
        except (FileNotFoundError,ProcessLookupError,PermissionError):pass
    return found
first=processes()
if phase=='start':time.sleep(.25)
second=processes() if phase=='start' else first
ticks={(p['pid'],p['startTicks'],t['tid']):t['ticks'] for p in first for t in p['threads']}
for p in second:
    for t in p['threads']:
        old=ticks.get((p['pid'],p['startTicks'],t['tid']))
        t['deltaTicks']=None if old is None else t['ticks']-old
def capture(path,start,previous=None):
    with path.open('rb') as f:
        st=os.fstat(f.fileno())
        if st.st_size<start:raise RuntimeError('log truncated: '+str(path))
        if previous:
            data=f.read(start)
            if hashlib.sha256(data).hexdigest()!=previous['prefixSha256']:raise RuntimeError('log prefix changed')
        else:f.seek(start)
        data=f.read(st.st_size-start)
    lines=data.decode(errors='replace').splitlines()
    return {'path':str(path),'startByte':start,'endByte':st.st_size,'sha256':hashlib.sha256(data).hexdigest(),
            'lineCount':len(lines),'diagnosticLines':[l for l in lines if l.startswith(('dist ','cross_elite ','[team-runner]','[server-bridge]','[bridge]'))],
            'errorLines':[l for l in lines if any(k in l.lower() for k in ('panic','timed out','timeout','fallback'))]}
logs={}
path=root/'resume-team.log'
if path.exists():
    if phase=='mark':
        data=path.read_bytes();logs['resume-team.log']={'endByte':len(data),'prefixSha256':hashlib.sha256(data).hexdigest()}
    elif phase=='end':logs['resume-team.log']=capture(path,marker['logs']['resume-team.log']['endByte'],marker['logs']['resume-team.log'])
    else:logs['resume-team.log']=capture(path,max(marker['logs']['resume-team.log']['endByte'],path.stat().st_size-100000))
if phase in ('start','end') and (root/'stderr.log').exists():
    logs['stderr.log']=capture(root/'stderr.log',0)
settings={key:{str(p):p.read_text().strip() for p in pathlib.Path('/sys/devices/system/cpu/cpufreq').glob('policy*/'+key)}
          for key in ('scaling_governor','energy_performance_preference')}
def command(args):
    r=subprocess.run(args,capture_output=True,text=True,timeout=5)
    return {'exitCode':r.returncode,'stdout':r.stdout,'stderr':r.stderr}
out={'capturedAtUtc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'phase':phase,
     'bootId':pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),'processes':second,
     'settings':settings,'clockProbe':clock_namespace['probe']('192.168.0.81',32123,8),
     'chronyTracking':command(['chronyc','-n','tracking']),'logs':logs,
     'files':{name:hashlib.sha256((root/name).read_bytes()).hexdigest() for name in node.get('files',{})}}
print('__RESULT__'+json.dumps(out))
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage')
    parser.add_argument('phase', choices=('mark', 'start', 'end'))
    args = parser.parse_args()
    manage.stage_directory(args.stage)
    cfg = manage.read(manage.CONFIG)
    state = manage.state()
    if args.phase == 'mark':
        manage.require_idle()
        manage.require_no_server()
    else:
        manage.require(state['evaluation']['stage'] == args.stage, 'different active stage')
        if args.phase == 'end':
            manage.require(not manage.alive(state['evaluation']), 'server still running')
    output = manage.WORK / 'observations' / args.stage
    output.mkdir(parents=True, exist_ok=True)
    target = output / (args.phase + '.json')
    manage.require(not target.exists(), 'observation already exists')
    previous = manage.read(output / 'mark.json') if args.phase != 'mark' else {}
    clock = (manage.ROOT / 'solver/distributed-lab/clock_sync.py').read_text()
    def collect(host, node):
        program = PROGRAM.replace('NODE', repr(node), 1).replace('PHASE', repr(args.phase), 1)
        program = program.replace('MARKER', repr(previous.get('nodes', {}).get(host)), 1)
        program = program.replace('CLOCK_SOURCE', repr(clock), 1)
        if host == 'local':
            result = subprocess.run([sys.executable, '-c', program], capture_output=True, text=True, timeout=30, check=True).stdout
        else:
            result = lifecycle.execute(host, 'python3 -c ' + shlex.quote(program), timeout=40)
        return json.loads(result.split('__RESULT__')[-1].strip())
    nodes = dict(cfg['nodes'])
    nodes['local'] = {'directory': str(manage.ROOT)}
    with concurrent.futures.ThreadPoolExecutor(max_workers=5) as pool:
        futures = {host: pool.submit(collect, host, node) for host, node in nodes.items()}
        results = {host: future.result() for host, future in futures.items()}
    for host in cfg['nodes']:
        manage.require(results[host]['files'] == cfg['nodes'][host]['files'], host + ': files changed')
    record = {'stage': args.stage, 'phase': args.phase, 'observerSha256': manage.sha(Path(__file__)),
              'configurationSha256': manage.sha(manage.CONFIG), 'nodes': results,
              'scope': 'One start sample only. Marked resume-team.log range covers this stage including bridge checkpoints. Legacy stderr.log may be stale and must not be used as evidence for this stage.'}
    manage.write(target, record)
    print(json.dumps({'saved': str(target), 'nodes': {h: {'processes': len(v['processes']),
          'clockOffsetMs': v['clockProbe']['medianBestHalfOffsetMs']} for h, v in results.items()}}))


if __name__ == '__main__':
    main()
