#!/usr/bin/env python3
"""採用済みD0を未使用の再開領域へ配置する。起動やNOMLAB変更は行わない。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shlex
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'solver/distributed/tools'))
from ssh import execute

D0_SHA = 'e80474cc5f65a30b18d8143eec656e3abe093c927d75080aed2bd9dce72a5de0'
BRIDGE_SHA = '24cb1008dbb537f6941f08d2e9c31042ff5587a9215748dd9b8e8b15f933c15e'
NODES = {
    'v1': ('0-31', ['--mode', 'coordinator', '--threads', '32', '--seed', '0', '--peers',
                    '192.168.0.22:39370/16/31,192.168.0.33:39370/32/46']),
    'v2': ('0-15', ['--mode', 'worker', '--threads', '16', '--worker-offset', '31', '--listen', '192.168.0.22:39370']),
    'v3': ('0-31', ['--mode', 'worker', '--threads', '32', '--worker-offset', '46', '--listen', '192.168.0.33:39370']),
}


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--name', required=True, help='新しい配置名。既存領域は上書きしない')
    args = parser.parse_args()
    if not args.name.replace('-', '').replace('_', '').isalnum():
        parser.error('配置名は英数字・-・_のみ')
    work = ROOT / '.server/distributed'
    binary, bridge = work / 'bin/distributed-solver', work / 'bin/server-bridge'
    if sha(binary) != D0_SHA or sha(bridge) != BRIDGE_SHA:
        raise SystemExit('D0アーカイブの識別値と不一致')
    stage = work / 'deployments' / args.name
    stage.mkdir(parents=True, exist_ok=False)
    token = stage / 'cluster.token'
    with open(token, 'x', opener=lambda p, flags: os.open(p, flags, 0o600)) as f:
        f.write(secrets.token_hex(32) + '\n')
    homes = {host: execute(host, 'printf "__HOME__%s\\n" "$HOME"').split('__HOME__')[-1].strip()
             for host in NODES}
    for host, home in homes.items():
        destination = home + '/procon37-distributed-resume/' + args.name
        execute(host, 'test ! -e ' + shlex.quote(destination))
    record = {'version': 'D0', 'running': False, 'nodes': {}, 'fixedNomlabUnchanged': True}
    for host, (cpus, arguments) in NODES.items():
        directory = homes[host] + '/procon37-distributed-resume/' + args.name
        local = stage / host
        local.mkdir()
        launcher = local / 'launcher'
        environment = ['PATH=/usr/bin:/bin', 'PROCON37_G007_FUTURE_WEIGHT=0', 'PROCON37_G007_RESERVE_WEIGHT=15',
                       'PROCON37_DISTRIBUTED_TOKEN_FILE=' + directory + '/cluster.token']
        launcher.write_text('#!/bin/sh\nexec env -i ' + shlex.join(environment) + ' taskset -c ' + cpus + ' ' +
                            shlex.join([directory + '/distributed-solver', *arguments]) + ' "$@"\n')
        launcher.chmod(0o755)
        files = {'distributed-solver': binary, 'server-bridge': bridge, 'launcher': launcher, 'cluster.token': token}
        execute(host, 'mkdir -p ' + shlex.quote(str(Path(directory).parent)) +
                ' && mkdir ' + shlex.quote(directory))
        for name, source in files.items():
            execute(host, upload=(str(source), directory + '/' + name))
        execute(host, 'chmod 700 ' + shlex.quote(directory) + '; chmod 600 ' + shlex.quote(directory + '/cluster.token') +
                '; chmod 755 ' + shlex.join([directory + '/' + x for x in ('distributed-solver', 'server-bridge', 'launcher')]))
        hashes = {name: sha(path) for name, path in files.items() if name != 'cluster.token'}
        verify = 'cd ' + shlex.quote(directory) + ' && sha256sum distributed-solver server-bridge launcher'
        output = execute(host, verify)
        for name, value in hashes.items():
            if value + '  ' + name not in output:
                raise RuntimeError(host + ': 転送後のSHA不一致')
        record['nodes'][host] = {'home': homes[host], 'directory': directory, 'cpus': cpus,
                                 'arguments': arguments, 'files': hashes}
        (stage / 'deployment.json').write_text(json.dumps(record, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(record, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
