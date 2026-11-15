#!/usr/bin/env python3
"""停止済みの55秒提出配置へ、ユーザー指定の同一内容再提出省略を反映する。"""
import copy
import json
from pathlib import Path
import shutil
import time

import lifecycle
import manage
from deploy_submission_policy import replace_remote


def main():
    with manage.locked():
        manage.require_idle()
        manage.require_no_server()
        old = manage.read(manage.CONFIG)
        manage.require(old['version'] == 'C013-scheduled-submission-on-submit55', 'expected paused submission deployment')
        old_bridge = '923d9d7a58b7797c9dd375b286cc12c0e1481d3e368c3b94708ef62af0e37b94'
        manage.require(old['fixedLocalFiles']['.server/distributed/bin/server-bridge'] == old_bridge,
                       'unexpected original bridge')
        allowed_changes = {'tester/server-bridge/src/main.rs', 'tester/server-bridge/src/schedule.rs',
                           'solver/distributed-lab/timed_regression.py', 'solver/distributed-lab/observe.py'}
        changed = {}
        for name, expected in old['localFiles'].items():
            actual = manage.sha(manage.local_path(name))
            if actual != expected:
                manage.require(name in allowed_changes, 'unrelated source changed: ' + name)
                changed[name] = {'before': expected, 'after': actual}
        for name, expected in old['fixedLocalFiles'].items():
            manage.require(manage.sha(manage.local_path(name)) == expected, 'fixed local changed: ' + name)
        for host in ('v1', 'v2', 'v3', 'nomlab', 'local2', 'local3'):
            spec = old['localTeams'][host] if host.startswith('local') else old['nodes'][host]
            process = lifecycle.run_spec('local' if host.startswith('local') else host, spec, 'status')
            manage.require(not process['alive'], 'team still running: ' + host)
        before = {host: manage.ssh_probe(node) for host, node in old['nodes'].items()}
        for host, record in before.items():
            manage.require(record['files'] == old['nodes'][host]['files'] and not record['processes'],
                           'remote state changed: ' + host)
        bridge = manage.ROOT / 'target/release/server-bridge'
        expected = manage.sha(bridge)
        manage.require(expected != old_bridge, 'new bridge not built')
        stage = manage.WORK / 'deployments' / (time.strftime('%Y%m%dT%H%M%S') + '-submit55-dedup-C013')
        stage.mkdir(parents=True, exist_ok=False)
        updated = copy.deepcopy(old)
        updated['version'] += '-dedup'
        policy = updated['environmentIdentity']['submissionPolicy']
        policy.update(skipUnchangedPayload=True, equality='JSON answer value versus last actually attempted payload within the same phase',
                      duplicatePolicyAuthorization='User requested skipping unchanged second/later submissions and restarting the interrupted test.')
        updated['localFiles'].update({name: record['after'] for name, record in changed.items()})
        for name in ('solver/distributed-lab/deploy_submission_dedup.py',
                     'tester/server-bridge/tests/check_scheduled_submissions.py',
                     'tester/server-bridge/tests/test_console.py'):
            updated['localFiles'][name] = manage.sha(manage.local_path(name))
        for host in updated['nodes']:
            name = 'bin/server-bridge' if host == 'nomlab' else 'server-bridge'
            manage.require(updated['nodes'][host]['files'][name] == old_bridge, 'remote bridge version differs')
            updated['nodes'][host]['files'][name] = expected
        updated['fixedLocalFiles']['.server/distributed/bin/server-bridge'] = expected
        manage.validate_config(updated)
        manage.write(stage / 'before-config.json', old)
        manage.write(stage / 'before-remote.json', before)
        manage.write(stage / 'planned-config.json', updated)
        record = {'status': 'installing', 'bridgeSha256': expected, 'sourceChanges': changed,
                  'authorization': policy['duplicatePolicyAuthorization'], 'remote': []}
        manage.write(stage / 'deployment.json', record)
        for host in updated['nodes']:
            name = 'bin/server-bridge' if host == 'nomlab' else 'server-bridge'
            result = replace_remote(host, updated['nodes'][host], bridge, name, expected, 'backups/' + stage.name)
            record['remote'].append({'host': host, **result})
            manage.write(stage / 'deployment.json', record)
        local = manage.ROOT / '.server/distributed/bin/server-bridge'
        shutil.copy2(local, stage / 'previous-server-bridge')
        temporary = local.with_name(local.name + '.upload-' + expected[:16])
        shutil.copy2(bridge, temporary)
        manage.require(manage.sha(temporary) == expected, 'local upload hash mismatch')
        temporary.chmod(0o755)
        temporary.replace(local)
        after = {host: manage.ssh_probe(node) for host, node in updated['nodes'].items()}
        for host, result in after.items():
            manage.require(result['files'] == updated['nodes'][host]['files'] and not result['processes'],
                           'post-install remote mismatch: ' + host)
        for name, expected_hash in {**updated['localFiles'], **updated['fixedLocalFiles']}.items():
            manage.require(manage.sha(manage.local_path(name)) == expected_hash, 'post-install local mismatch: ' + name)
        manage.write(manage.CONFIG, updated)
        manage.write(manage.WORK / 'resume-config.json', updated)
        manage.write(stage / 'after-remote.json', after)
        record.update(status='deployed-not-started', version=updated['version'])
        manage.write(stage / 'deployment.json', record)
        current = manage.state()
        current.update(stage='submission_dedup_deployed', activeEvaluation=False,
                       submissionPolicyDeployment=str(stage), currentCandidate=updated['version'])
        manage.write(manage.STATE, current)
        print(json.dumps({'version': updated['version'], 'bridgeSha256': expected,
                          'record': str(stage / 'deployment.json')}))


if __name__ == '__main__':
    main()
