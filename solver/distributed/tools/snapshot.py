#!/usr/bin/env python3
"""評価前後の実ファイルを新しく照合し、実行中の配置・スレッドを記録する。"""
import argparse
import concurrent.futures
import hashlib
import json
from pathlib import Path
from ssh import execute

ROOT = Path(__file__).resolve().parents[3]

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def snapshot():
    deployment = json.loads((ROOT / 'solver/docs/distributed/initial-deployment.json').read_text())
    remote_program = '''import hashlib,json,pathlib,os
d=pathlib.Path(DIRECTORY)
names=['bin/distributed-solver','bin/server-bridge','launcher','run.sh']
r={'files':{n:hashlib.sha256((d/n).read_bytes()).hexdigest() for n in names},'processes':[]}
for p in pathlib.Path('/proc').iterdir():
    if not p.name.isdigit():continue
    try:
        argv=(p/'cmdline').read_bytes().replace(bytes([0]),b' ').decode(errors='replace')
        if str(d) not in argv or argv.startswith('python3 -'):continue
        status=(p/'status').read_text()
        wanted=[x for x in status.splitlines() if x.startswith(('Name:','Threads:','Cpus_allowed_list:'))]
        r['processes'].append({'pid':int(p.name),'command':argv,'status':wanted})
    except (FileNotFoundError,PermissionError,ProcessLookupError):pass
print('__RESULT__'+json.dumps(r))
'''
    def read_remote(host):
        directory = deployment['nodes'][host]['directory']
        program = remote_program.replace('DIRECTORY', repr(directory))
        output = execute(host, "python3 - <<'PY'\n" + program + '\nPY')
        payload = next(line.removeprefix('__RESULT__') for line in output.splitlines()
                       if line.startswith('__RESULT__'))
        return host, json.loads(payload)
    result = {'remote': dict(concurrent.futures.ThreadPoolExecutor(4).map(read_remote, deployment['nodes']))}
    local_names = ['target/release/server', 'target/release/server-bridge', 'tester/server/procon-server',
                   'tester/server-bridge/run.sh', '.server/distributed/bin/pool-solver-06',
                   '.server/distributed/bin/pool-solver-08', '.server/distributed/bin/server-bridge']
    local_names += list(deployment['launchers'])
    result['localFiles'] = {name: digest(ROOT / name) for name in local_names}
    source = ROOT / 'solver/distributed'
    files = {str(p.relative_to(source)): digest(p) for p in sorted(source.rglob('*'))
             if p.is_file() and not {'target','__pycache__'}.intersection(p.relative_to(source).parts)}
    result['sourceFiles'] = files
    result['sourceHash'] = hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest()
    bridge_hash = result['localFiles']['target/release/server-bridge']
    run_hash = result['localFiles']['tester/server-bridge/run.sh']
    for host, values in result['remote'].items():
        expected = deployment['nodes'][host]
        assert values['files']['bin/distributed-solver'] == expected['binarySha256'], host + ': binary変更'
        assert values['files']['launcher'] == expected['launcherSha256'], host + ': launcher変更'
        assert values['files']['bin/server-bridge'] == bridge_hash, host + ': bridge相違'
        assert values['files']['run.sh'] == run_hash, host + ': run.sh相違'
    return result

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--save', required=True)
    parser.add_argument('--before')
    args = parser.parse_args()
    result = snapshot()
    if args.before:
        before = json.loads(Path(args.before).read_text())
        for key in ('localFiles', 'sourceFiles', 'sourceHash'):
            assert before[key] == result[key], '評価中の変更: ' + key
        for host in result['remote']:
            assert before['remote'][host]['files'] == result['remote'][host]['files'], host + ': 評価中の変更'
        result['verifiedUnchanged'] = True
    Path(args.save).write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({'sourceHash': result['sourceHash'], 'verifiedUnchanged': result.get('verifiedUnchanged'),
                      'remoteHosts': list(result['remote']), 'saved': args.save}, ensure_ascii=False))
