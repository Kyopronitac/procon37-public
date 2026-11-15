#!/usr/bin/env python3
"""ユーザー指定の55秒提出を新しい評価条件として配置する（起動は別）。"""
import argparse
import copy
import json
from pathlib import Path
import shlex
import shutil
import time

import lifecycle
import manage
import switch


def replace_remote(host, node, source, name, expected, backup_name):
    temporary = node['directory'] + '/' + name + '.upload-' + expected[:16]
    lifecycle.execute(host, upload=(str(source), temporary))
    code = '''import pathlib,hashlib,os,shutil,json
root=pathlib.Path(DIRECTORY)
target=root/NAME
temporary=pathlib.Path(TEMPORARY)
assert hashlib.sha256(temporary.read_bytes()).hexdigest()==EXPECTED
backup=root/BACKUP/NAME
assert not backup.exists()
backup.parent.mkdir(parents=True,exist_ok=True)
shutil.copy2(target,backup)
temporary.chmod(0o755)
os.replace(temporary,target)
print('__RESULT__'+json.dumps({'path':str(target),'backup':str(backup),'sha256':EXPECTED}))
'''
    for key, value in [('DIRECTORY', node['directory']), ('NAME', name), ('TEMPORARY', temporary),
                       ('EXPECTED', expected), ('BACKUP', backup_name)]:
        code = code.replace(key, repr(value))
    output = lifecycle.execute(host, 'python3 -c ' + shlex.quote(code))
    return json.loads(output.split('__RESULT__')[-1])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--candidate', default='c013-scheduled-submission')
    parser.add_argument('--bridge', default='target/release/server-bridge')
    parser.add_argument('--server-release', default='.server/build-submit55/release/server')
    args = parser.parse_args()
    version, binary, binary_sha, source_files = switch.candidate(args.candidate)
    bridge = manage.local_path(args.bridge)
    manage.require(manage.sha(binary) == binary_sha, 'candidate binary differs from validation')
    bridge_sha = manage.sha(bridge)
    server = manage.local_path(args.server_release)
    server_sha = manage.sha(server)
    runner = manage.ROOT / 'solver/distributed-lab/team_runner.py'
    runner_sha = manage.sha(runner)
    with manage.locked():
        manage.require_idle()
        manage.require_no_server()
        old = manage.read(manage.CONFIG)
        manage.require(old['version'] == 'C012-isolated-cooperation-on', 'expected paused C012 deployment')
        for name, expected in old['fixedLocalFiles'].items():
            manage.require(manage.sha(manage.local_path(name)) == expected, 'fixed local file changed: ' + name)
        stopped = {}
        for host in ('v1', 'v2', 'v3', 'nomlab', 'local2', 'local3'):
            spec = old['localTeams'][host] if host.startswith('local') else old['nodes'][host]
            stopped[host] = lifecycle.run_spec('local' if host.startswith('local') else host, spec, 'status')
            manage.require(not stopped[host]['alive'], 'team still active: ' + host)
        remote_before = {h: manage.ssh_probe(node) for h, node in old['nodes'].items()}
        for host, probe in remote_before.items():
            manage.require(probe['files'] == old['nodes'][host]['files'] and not probe['processes'],
                           'remote files changed or unexpected process: ' + host)
        stage = manage.WORK / 'deployments' / (time.strftime('%Y%m%dT%H%M%S') + '-submit55-C013')
        stage.mkdir(parents=True, exist_ok=False)
        backup_name = 'backups/' + stage.name
        manage.write(stage / 'before-config.json', old)
        manage.write(stage / 'before-remote.json', remote_before)
        manage.write(stage / 'before-processes.json', stopped)
        updated = copy.deepcopy(old)
        updated['version'] = version + '-on-submit55'
        updated['environmentIdentity']['submissionPolicy'] = {
            'revision': '20261010-user-submit55', 'selectionAndDay': True,
            'distributedMs': [5000, 15000, 25000, 35000, 45000, 55000],
            'fixedMs': [55000], 'serverTimeLimitOverride': None,
            'authorization': 'User explicitly requested all solvers submit at 55sec and distributed at 5,15,...55sec.'}
        generated = switch.launchers(old, 'on')
        generated['v1'] = generated['v1'][:-6] + ' --stream-responses "$@"\n'
        updated['localFiles'] = {p: manage.sha(manage.local_path(p)) for p in old['localFiles']
                                 if not p.startswith('solver/distributed-candidates/')}
        updated['localFiles'].update(source_files)
        for p in [Path(__file__).resolve(), manage.ROOT / 'solver/distributed-lab/timed_regression.py']:
            updated['localFiles'][str(p.relative_to(manage.ROOT))] = manage.sha(p)
        build_inputs = [manage.ROOT / 'Cargo.lock', manage.ROOT / 'Cargo.toml']
        for part in ('common', 'tester/server-bridge', 'tester/server'):
            base = manage.ROOT / part
            build_inputs += [base / 'Cargo.toml', *sorted((base / 'src').rglob('*.rs'))]
        for p in build_inputs:
            updated['localFiles'][str(p.relative_to(manage.ROOT))] = manage.sha(p)
        for relative in ('tester/server-bridge/run.sh', 'tester/server-bridge/distributed/run.sh',
                         'tester/server-bridge/console.py'):
            updated['localFiles'][relative] = manage.sha(manage.local_path(relative))
        modifications = []
        for host in ('v1', 'v2', 'v3'):
            node = updated['nodes'][host]
            launcher = stage / (host + '-launcher')
            launcher.write_text(generated[host])
            for name, source, expected in [('distributed-solver', binary, binary_sha),
                                            ('server-bridge', bridge, bridge_sha),
                                            ('launcher', launcher, manage.sha(launcher))]:
                node['files'][name] = expected
                modifications.append((host, source, name, expected))
        for host in ('v1', 'nomlab'):
            node = updated['nodes'][host]
            node['files']['resume-team-runner.py'] = runner_sha
            modifications.append((host, runner, 'resume-team-runner.py', runner_sha))
            node['launch'] += ['--submit-at-ms', '5000,15000,25000,35000,45000,55000' if host == 'v1' else '55000']
            if host == 'v1':
                node['launch'] += ['--stream-responses']
        updated['nodes']['nomlab']['files']['bin/server-bridge'] = bridge_sha
        modifications.append(('nomlab', bridge, 'bin/server-bridge', bridge_sha))
        updated['fixedLocalFiles'].pop(updated['server'])
        updated['server'] = '.server/distributed/submit55/server'
        local_changes = [('.server/distributed/bin/server-bridge', bridge, bridge_sha),
                         (updated['server'], server, server_sha)]
        for host in ('local2', 'local3'):
            spec = updated['localTeams'][host]
            spec['files']['resume-team-runner.py'] = runner_sha
            spec['launch'] += ['--submit-at-ms', '55000']
            path = str((Path(spec['directory']) / 'resume-team-runner.py').relative_to(manage.ROOT))
            local_changes.append((path, runner, runner_sha))
        for name, source, expected in local_changes:
            updated['fixedLocalFiles'][name] = expected
        manage.validate_config(updated)
        manage.write(stage / 'planned-config.json', updated)
        record = {'authorization': updated['environmentIdentity']['submissionPolicy']['authorization'],
                  'status': 'installing', 'directory': str(stage), 'remote': [], 'local': []}
        manage.write(stage / 'deployment.json', record)
        for host, source, name, expected in modifications:
            result = replace_remote(host, updated['nodes'][host], source, name, expected, backup_name)
            record['remote'].append({'host': host, **result})
            manage.write(stage / 'deployment.json', record)
        for name, source, expected in local_changes:
            target = manage.local_path(name)
            target.parent.mkdir(parents=True, exist_ok=True)
            backup = stage / 'local-backup' / name
            backup.parent.mkdir(parents=True, exist_ok=True)
            if target.exists():
                shutil.copy2(target, backup)
            temporary = target.with_name(target.name + '.upload-' + expected[:16])
            shutil.copy2(source, temporary)
            manage.require(manage.sha(temporary) == expected, 'local copy differs')
            temporary.chmod(0o755)
            temporary.replace(target)
            record['local'].append({'path': name, 'backup': str(backup) if backup.exists() else None, 'sha256': expected})
            manage.write(stage / 'deployment.json', record)
        after = {h: manage.ssh_probe(node) for h, node in updated['nodes'].items()}
        for host, probe in after.items():
            manage.require(probe['files'] == updated['nodes'][host]['files'] and not probe['processes'],
                           'post-install remote mismatch: ' + host)
        for name, expected in {**updated['localFiles'], **updated['fixedLocalFiles']}.items():
            manage.require(manage.sha(manage.local_path(name)) == expected, 'post-install local mismatch: ' + name)
        # A new, explicitly requested experiment policy; ordinary configure/switch guards remain unchanged.
        manage.write(manage.CONFIG, updated)
        manage.write(manage.WORK / 'resume-config.json', updated)
        manage.write(stage / 'after-remote.json', after)
        record['status'] = 'deployed-not-started'
        manage.write(stage / 'deployment.json', record)
        current = manage.state()
        current.update(stage='submission_policy_deployed', activeEvaluation=False,
                       submissionPolicyDeployment=str(stage), currentCandidate=updated['version'])
        manage.write(manage.STATE, current)
        print(json.dumps({'version': updated['version'], 'record': str(stage / 'deployment.json'),
                          'bridgeSha256': bridge_sha, 'serverSha256': server_sha, 'candidateSha256': binary_sha}))


if __name__ == '__main__':
    main()
