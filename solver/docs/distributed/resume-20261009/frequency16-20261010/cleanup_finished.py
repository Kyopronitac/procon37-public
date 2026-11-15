#!/usr/bin/env python3
"""Explicit, finite cleanup after the verified frequency matrix; never launch a match."""
import argparse
import concurrent.futures
import csv
import io
import json
import os
from pathlib import Path
import select
import shlex
import signal
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
ROOT = next(p for p in HERE.parents if (p / 'solver/distributed-lab/lifecycle.py').is_file())
sys.path.insert(0, str(ROOT / 'solver/distributed-lab'))
import lifecycle
import manage

STAGE = 'frequency16-matrix-20261010-01'
EXPECTED_CLOCK = {'pid': 75930, 'startTicks': '2767228',
                  'bootId': '61fafedf-4b69-4c18-81a9-6af42f0f3422', 'uid': 1000}

SCAN = r'''
import json,os,pathlib
registered=set(REGISTERED_PATHS)
found=[]
for p in pathlib.Path('/proc').iterdir():
    if not p.name.isdigit():continue
    try:
        args=(p/'cmdline').read_bytes().decode(errors='replace').split(chr(0))
        matches=sorted(registered.intersection(args))
        try:
            executable=os.readlink(p/'exe')
        except (FileNotFoundError,ProcessLookupError,PermissionError):executable=None
        if executable in registered:matches=sorted(set(matches+[executable]))
        if matches:
            stat=(p/'stat').read_text().rsplit(')',1)[1].split()
            found.append({'pid':int(p.name),'uid':p.stat().st_uid,'state':stat[0],
                          'startTicks':stat[19],'session':int(stat[3]),'matchedPaths':matches})
    except (FileNotFoundError,ProcessLookupError,PermissionError):pass
print('__RESULT__'+json.dumps(found))
'''

CHRONY = r'''
import csv,hashlib,io,json,os,pathlib,subprocess
expected=EXPECTED_FILES
assert os.geteuid()==0,'chrony cleanup requires sudo'
def configuration():
    result=[]
    for item in expected:
        actual=hashlib.sha256(pathlib.Path(item['path']).read_bytes()).hexdigest()
        assert actual==item['expected'],'persistent chrony configuration changed: '+item['path']
        result.append({'path':item['path'],'sha256':actual,'expected':item['expected']})
    return result
def command(args):
    child=subprocess.run(['chronyc',*args],capture_output=True,text=True,timeout=10)
    return {'args':args,'exitCode':child.returncode,'stdout':child.stdout,'stderr':child.stderr}
def addresses(result):
    assert result['exitCode']==0,'chronyc sources failed'
    rows=list(csv.reader(io.StringIO(result['stdout'])))
    assert all(len(row)>=3 for row in rows),'unexpected chronyc CSV'
    return sorted(row[2] for row in rows)
record={'persistentBefore':configuration(),'before':command(['-c','-n','sources'])}
prior=addresses(record['before'])
record['delete']=None
if '192.168.0.81' in prior:
    record['delete']=command(['delete','192.168.0.81'])
    assert record['delete']['exitCode']==0,'chronyc delete failed'
record['after']=command(['-c','-n','sources'])
after=addresses(record['after'])
assert '192.168.0.81' not in after,'temporary runtime source remains'
assert [x for x in prior if x!='192.168.0.81']==after,'other chrony source set changed'
record['persistentAfter']=configuration()
record['tracking']=command(['-n','tracking'])
record['temporarySourceRemoved']=True
print('__RESULT__'+json.dumps(record))
'''


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def clock_identity():
    p = Path('/proc') / str(EXPECTED_CLOCK['pid'])
    try:
        stat = (p / 'stat').read_text().rsplit(')', 1)[1].split()
        return {'pid': EXPECTED_CLOCK['pid'], 'startTicks': stat[19], 'state': stat[0],
                'bootId': Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
                'uid': p.stat().st_uid,
                'command': (p / 'cmdline').read_bytes().decode().rstrip('\0').split('\0')}
    except (FileNotFoundError, ProcessLookupError):
        return None


def expected_clock_command():
    return ['/usr/bin/python3', str(ROOT / 'solver/distributed-lab/clock_sync.py'),
            'serve', '--record-dir', str(ROOT / '.server/distributed/clock')]


def check_clock_identity(value):
    require(value and all(value.get(k) == v for k, v in EXPECTED_CLOCK.items()),
            'clock PID/start ticks/boot/UID mismatch; no signal sent')
    require(value['command'] == expected_clock_command(), 'clock command mismatch; no signal sent')


def stop_clock():
    stored = manage.read(ROOT / '.server/distributed/clock/server-process.json')
    require(all(stored.get(k) == EXPECTED_CLOCK[k] for k in ('pid', 'startTicks', 'bootId')),
            'clock process record changed')
    require(stored['source'] == str(ROOT / 'solver/distributed-lab/clock_sync.py'), 'clock source mismatch')
    before = clock_identity()
    check_clock_identity(before)
    descriptor = os.pidfd_open(EXPECTED_CLOCK['pid'])
    try:
        check_clock_identity(clock_identity())
        signal.pidfd_send_signal(descriptor, signal.SIGTERM)
        poller = select.poll()
        poller.register(descriptor, select.POLLIN)
        require(bool(poller.poll(5000)), 'clock did not exit after SIGTERM; inspect owned PID')
    finally:
        os.close(descriptor)
    after = clock_identity()
    require(after is None or after.get('state') == 'Z', 'clock remains active')
    return {'before': before, 'after': after, 'signal': 'SIGTERM via pidfd', 'stopped': True}


def registered_paths(cfg):
    result = {}
    for host, spec in cfg['nodes'].items():
        result[host] = sorted({str(Path(spec['directory']) / name) for name in spec['files']}
                              | {p['matchArg'] for p in spec['processes']})
    result['local'] = sorted({str(ROOT / name) for name in cfg['fixedLocalFiles']}
                             | {p['matchArg'] for p in cfg['localProcesses']})
    return result


def scan(host, paths):
    code = SCAN.replace('REGISTERED_PATHS', repr(paths), 1)
    if host == 'local':
        output = subprocess.run([sys.executable, '-c', code], check=True, capture_output=True, text=True).stdout
    else:
        output = lifecycle.execute(host, 'python3 -c ' + shlex.quote(code))
    return json.loads(output.split('__RESULT__')[-1].strip())


def verified_matrix():
    report = manage.read(manage.stage_directory(STAGE) / 'results.json')
    require(report['stage'] == STAGE and report['status'] == 'verified' and report['all32AC'] is True,
            'matrix is not verified/all32AC; refuse cleanup')
    require(len(report['runs']) == 8 and len(report['variants']) == 2, 'matrix case count mismatch')
    for version in ('C014', 'C015'):
        rows = [row for row in report['runs'] if row['version'] == version]
        require(sorted(row['seed'] for row in rows) == [0, 3, 6, 9], 'matrix seed coverage mismatch')
        require(all(row['all4AC'] is True and len(row['case']['players']) == 4
                    and all(p['status'] == 'AC' for p in row['case']['players']) for row in rows),
                'matrix includes a non-AC player')
    evaluation = manage.state()['evaluation']
    require(evaluation['stage'] == STAGE and evaluation['status'] == 'verified', 'active evaluation changed')
    require(not manage.alive(evaluation), 'matrix parent is still alive; wait for it to finish')
    manage.require_no_server()
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--execute', action='store_true', help='Run only after the root agent explicitly authorizes cleanup.')
    args = parser.parse_args()
    require(args.execute, 'No changes made. Explicit --execute is required after the root agent authorizes cleanup.')
    os.sched_setaffinity(0, {1, 3, 5, 7})
    os.chdir(ROOT)
    record_path, clock_path = HERE / 'cleanup.json', HERE / 'clock-cleanup.json'
    require(not record_path.exists() and not clock_path.exists(), 'cleanup record already exists; inspect it instead of rerunning')
    with manage.locked():
        matrix = verified_matrix()
        cfg = manage.read(ROOT / '.server/distributed/resume-config.json')
        lab = manage.read(ROOT / '.server/distributed/lab-config.json')
        require(cfg['nodes'] == lab['nodes'] and cfg['localTeams'] == lab['localTeams'],
                'lifecycle and evaluation deployment registration differ')
        setup = manage.read(HERE / 'clock-restored.json')
        check_clock_identity(clock_identity())
        record = {'stage': STAGE, 'status': 'running', 'startedUnix': time.time(),
                  'matrixAll32AC': matrix['all32AC'], 'resultsSha256': manage.sha(manage.stage_directory(STAGE) / 'results.json'),
                  'resumeConfigSha256': manage.sha(ROOT / '.server/distributed/resume-config.json'),
                  'scriptSha256': manage.sha(__file__)}
        manage.write(record_path, record)
        try:
            child = subprocess.run([sys.executable, str(ROOT / 'solver/distributed-lab/lifecycle.py'),
                                    'stop', '--config', '.server/distributed/resume-config.json'],
                                   text=True, capture_output=True, timeout=120)
            record['lifecycle'] = {'exitCode': child.returncode, 'stdout': child.stdout, 'stderr': child.stderr}
            manage.write(record_path, record)
            require(child.returncode == 0, 'registered team shutdown failed; inspect cleanup record')
            paths = registered_paths(lab)
            with concurrent.futures.ThreadPoolExecutor(max_workers=5) as pool:
                jobs = {host: pool.submit(scan, host, names) for host, names in paths.items()}
                record['remainingProcesses'] = {host: job.result() for host, job in jobs.items()}
            manage.write(record_path, record)
            require(not any(record['remainingProcesses'].values()), 'registered processes remain; no broad kill attempted')
            clock_record = {'startedUnix': time.time(), 'setupRecord': 'clock-restored.json',
                            'setupSha256': manage.sha(HERE / 'clock-restored.json'), 'nodes': {}}
            manage.write(clock_path, clock_record)
            for host in ('v1', 'v2', 'v3', 'nomlab'):
                code = CHRONY.replace('EXPECTED_FILES', repr(setup[host]['persistentConfiguration']), 1)
                output = lifecycle.execute(host, "sudo -S -p 'Password:' python3 -c " + shlex.quote(code), timeout=45)
                clock_record['nodes'][host] = json.loads(output.split('__RESULT__')[-1].strip())
                manage.write(clock_path, clock_record)
            clock_record['clock'] = stop_clock()
            clock_record.update(status='completed', finishedUnix=time.time())
            manage.write(clock_path, clock_record)
            record.update(status='completed', finishedUnix=time.time(), productionProbePermitted=True,
                          clockCleanup=str(clock_path.relative_to(ROOT)))
            manage.write(record_path, record)
            print(json.dumps({'status': record['status'], 'record': str(record_path),
                              'allRegisteredTeamsStopped': True, 'clockStopped': True}))
        except BaseException as error:
            record.update(status='failed', error=str(error), finishedUnix=time.time(), productionProbePermitted=False)
            manage.write(record_path, record)
            raise


if __name__ == '__main__':
    main()
