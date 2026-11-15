#!/usr/bin/env python3
"""専用領域へ初期版を配置。旧poolや他のユーザープロセスには触れない。"""
import hashlib
import json
import os
from pathlib import Path
import secrets
import shlex
import subprocess
from ssh import execute, HOSTS

ROOT = Path(__file__).resolve().parents[3]
RUNTIME = ROOT / '.server/distributed'
DOCS = ROOT / 'solver/docs/distributed'

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def write_launcher(path, executable, cpus, arguments, token=None, start_environment=True):
    lines = ['#!/bin/sh']
    if start_environment:
        lines += ['export PROCON37_G007_FUTURE_WEIGHT=0', 'export PROCON37_G007_RESERVE_WEIGHT=15']
    if token:
        lines.append('export PROCON37_DISTRIBUTED_TOKEN_FILE=' + shlex.quote(token))
    lines.append('exec taskset -c ' + shlex.quote(cpus) + ' ' + shlex.quote(executable) +
                 ' ' + shlex.join(arguments) + ' "$@"')
    Path(path).write_text('\n'.join(lines) + '\n')
    Path(path).chmod(0o755)

def start_remote(host, directory, command, pidfile, logfile):
    program = """import json,os,pathlib,subprocess
d=pathlib.Path(DIRECTORY)
pidfile=d/PIDFILE
if pidfile.exists():
    pid=int(pidfile.read_text())
    if pathlib.Path('/proc/'+str(pid)).exists():
        raise SystemExit('既存PIDを確認してから再配置する: '+str(pid))
with (d/LOGFILE).open('ab') as log:
    p=subprocess.Popen(COMMAND,cwd=d,stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
pidfile.write_text(str(p.pid)+'\\n')
print('__RESULT__'+json.dumps({'pid':p.pid,'directory':str(d),'log':str(d/LOGFILE)}))
"""
    for key, value in [('DIRECTORY', directory), ('PIDFILE', pidfile), ('LOGFILE', logfile), ('COMMAND', command)]:
        program = program.replace(key, repr(value))
    result = execute(host, 'python3 - <<\'PY\'\n' + program + '\nPY')
    return json.loads(result.split('__RESULT__')[-1])

def deploy():
    state_file = ROOT / '.server/distributed-state.json'
    current = json.loads(state_file.read_text())
    assert not current.get('deployment'), '初期配置済み。採用済み相手を再配置しない'
    binary = ROOT / 'solver/distributed/target/release/distributed-solver'
    assert binary.is_file()
    token = RUNTIME / 'token'
    if not token.exists():
        token.write_text(secrets.token_hex(32) + '\n')
        token.chmod(0o600)
    record = {'binarySha256': sha(binary), 'nodes': {}, 'teams': {}, 'launchers': {}}
    def checkpoint():
        (RUNTIME / 'deployment-progress.json').write_text(json.dumps(record, ensure_ascii=False, indent=2) + '\n')
    launchers = ROOT / 'solver/distributed/runtime'
    launchers.mkdir(exist_ok=True)
    settings = {'v1': ('0-31', ['--mode', 'coordinator', '--threads', '32', '--seed', '0',
                             '--peers', '192.168.0.22:39370/16/31,192.168.0.33:39370/32/46']),
                'v2': ('0-15', ['--mode', 'worker', '--threads', '16', '--worker-offset', '31', '--listen', '192.168.0.22:39370']),
                'v3': ('0-31', ['--mode', 'worker', '--threads', '32', '--worker-offset', '46', '--listen', '192.168.0.33:39370']),
                'nomlab': ('0-15', ['--mode', 'parallel', '--threads', '16', '--seed', '0'])}
    for host, (cpus, arguments) in settings.items():
        home = execute(host, 'printf "__HOME__%s\\n" "$HOME"').split('__HOME__')[-1].strip()
        directory = home + '/procon37-distributed'
        execute(host, 'mkdir -p ' + shlex.quote(directory + '/bin') + ' ' + shlex.quote(directory + '/logs'))
        execute(host, upload=(str(binary), directory + '/bin/distributed-solver'))
        execute(host, upload=(str(ROOT / 'tester/server-bridge/run.sh'), directory + '/run.sh'))
        execute(host, upload=(str(token), directory + '/token'))
        path = launchers / (host + '.sh')
        write_launcher(path, directory + '/bin/distributed-solver', cpus, arguments,
                       None if host == 'nomlab' else directory + '/token')
        execute(host, upload=(str(path), directory + '/launcher'))
        execute(host, 'chmod 755 ' + shlex.quote(directory + '/launcher') + ' ' + shlex.quote(directory + '/run.sh') +
                '; chmod 600 ' + shlex.quote(directory + '/token'))
        record['nodes'][host] = dict(directory=directory, cpus=cpus, arguments=arguments,
                                     binarySha256=sha(binary), launcherSha256=sha(path))
        record['launchers'][str(path.relative_to(ROOT))] = sha(path)
        checkpoint()
    for host in ('v2', 'v3'):
        directory = record['nodes'][host]['directory']
        record['nodes'][host]['process'] = start_remote(host, directory, [directory + '/launcher'], 'worker.pid', 'logs/worker.log')
        checkpoint()
    for host, team in [('v1', 0), ('nomlab', 1)]:
        directory = record['nodes'][host]['directory']
        command = ['env', '-u', 'SELECT_MS', '-u', 'TOKEN', 'SERVER=http://192.168.0.81:11451', 'TEAM=' + str(team),
                   'SOLVER=' + directory + '/launcher', 'BRIDGE=' + directory + '/bin/server-bridge', directory + '/run.sh']
        record['teams'][str(team)] = start_remote(host, directory, command, 'run.pid', 'logs/bridge.log')
        checkpoint()
    for generation, team, cpus in [(6, 2, '0'), (8, 3, '2')]:
        team_directory = RUNTIME / f'team{team}'
        team_directory.mkdir(exist_ok=True)
        path = launchers / ('generation' + str(generation) + '.sh')
        write_launcher(path, str(RUNTIME / f'bin/pool-solver-{generation:02d}'), cpus, [], start_environment=False)
        record['launchers'][str(path.relative_to(ROOT))] = sha(path)
        pidfile = RUNTIME / f'team{team}.pid'
        assert not pidfile.exists(), 'ローカル既存PIDを確認してから再配置する'
        environment = os.environ.copy()
        for key in list(environment):
            if key.startswith('PROCON37_'):
                environment.pop(key)
        environment.update(SERVER='http://192.168.0.81:11451', TEAM=str(team), SOLVER=str(path),
                           BRIDGE=str(RUNTIME / 'bin/server-bridge'))
        for key in ('SELECT_MS', 'TOKEN'):
            environment.pop(key, None)
        logfile = RUNTIME / f'team{team}-bridge.log'
        with logfile.open('ab') as log:
            process = subprocess.Popen([str(ROOT / 'tester/server-bridge/run.sh')], cwd=team_directory,
                                       env=environment, stdin=subprocess.DEVNULL, stdout=log,
                                       stderr=subprocess.STDOUT, start_new_session=True)
        pidfile.write_text(str(process.pid) + '\n')
        record['teams'][str(team)] = dict(pid=process.pid, directory=str(team_directory), log=str(logfile), cpus=cpus)
        checkpoint()
    (DOCS / 'initial-deployment.json').write_text(json.dumps(record, ensure_ascii=False, indent=2) + '\n')
    state_path = ROOT / '.server/distributed-state.json'
    state = json.loads(state_path.read_text())
    state.update(stage='deployed_before_smoke', deployment=record)
    state_path.write_text(json.dumps(state, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(record, ensure_ascii=False, indent=2))

if __name__ == '__main__':
    deploy()
