#!/usr/bin/env python3
"""確定済み評価の間だけ、同じ3台・同じ配置で分散側のバイナリを切り替える。"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import shlex
import sys
import time

import lifecycle
import manage

ROOT = manage.ROOT
D0_SHA = 'e80474cc5f65a30b18d8143eec656e3abe093c927d75080aed2bd9dce72a5de0'


def candidate(name):
    if name == 'D0':
        return 'D0', ROOT / '.server/distributed/bin/distributed-solver', D0_SHA, {}
    base = (ROOT / 'solver/distributed-candidates' / name).resolve()
    manage.require(base.parent == ROOT / 'solver/distributed-candidates', '候補名は直下のディレクトリ名のみ')
    manifest = manage.read(base / 'source-manifest.json')
    validation = manage.read(base / 'validation.json')
    source = manifest['candidateSource']
    manage.require(source['aggregateSha256'] == validation['sourceAggregateSha256'], '検証記録のソース識別不一致')
    files = {}
    for relative, expected in source['files'].items():
        path = (base / relative).resolve()
        manage.require(path.is_relative_to(base), '候補外のソース指定')
        manage.require(manage.sha(path) == expected, '候補ソース不一致: ' + relative)
        files[str(path.relative_to(ROOT))] = expected
    aggregate = hashlib.sha256(''.join(f'{source["files"][p]}  {p}\n' for p in sorted(source['files'])).encode()).hexdigest()
    manage.require(aggregate == source['aggregateSha256'], 'ソース集約識別不一致')
    binary = (base / validation['binary']['path']).resolve()
    manage.require(binary.is_relative_to(base), '候補外のバイナリ指定')
    return manifest['candidate'], binary, validation['binary']['sha256'], files


def launchers(config, mode, baseline=None):
    """必ず記録済みD0から再生成し、前候補の追加引数を持ち越さない。"""
    baseline = baseline or ROOT / '.server/distributed/deployments/20261009-D0'
    original = manage.read(baseline / 'deployment.json')
    result = {}
    for host in ('v1', 'v2', 'v3'):
        record = original['nodes'][host]
        node = config['nodes'][host]
        manage.require(all(record[k] == node[k] for k in ('home', 'directory', 'cpus')), '初期配置と相違: ' + host)
        source = baseline / host / 'launcher'
        manage.require(manage.sha(source) == record['files']['launcher'], '初期launcher識別不一致: ' + host)
        content = source.read_text()
        manage.require(content.endswith(' "$@"\n') and '--cross-elite' not in content, '初期launcher形式不正')
        if mode is not None:
            manage.require(mode in ('on', 'off'), 'elite設定不正')
            content = content[:-6] + ' --cross-elite ' + mode + ' "$@"\n'
        result[host] = content
    return result


def remote_install(host, spec, binary, expected, name='distributed-solver'):
    manage.require(name in ('distributed-solver', 'launcher'), '配置対象名不正')
    directory = spec['directory']
    temporary = directory + '/' + name + '.upload-' + expected[:16]
    lifecycle.execute(host, upload=(str(binary), temporary))
    code = '''import hashlib,json,os,pathlib
root=pathlib.Path(DIRECTORY)
temporary=pathlib.Path(TEMPORARY)
expected=EXPECTED
if hashlib.sha256(temporary.read_bytes()).hexdigest()!=expected:raise SystemExit('uploaded binary hash mismatch')
argv_target=str(root/'distributed-solver')
for p in pathlib.Path('/proc').iterdir():
    if not p.name.isdigit():continue
    try:
        args=(p/'cmdline').read_bytes().decode(errors='replace').split(chr(0))
        if argv_target in args:raise SystemExit('solver process still active: '+p.name)
    except (FileNotFoundError,ProcessLookupError,PermissionError):pass
temporary.chmod(0o755)
os.replace(temporary,root/NAME)
print('__RESULT__'+json.dumps({'sha256':expected,'directory':str(root)}))
'''.replace('DIRECTORY', repr(directory), 1).replace('TEMPORARY', repr(temporary), 1).replace('EXPECTED', repr(expected), 1).replace('NAME', repr(name), 1)
    result = lifecycle.execute(host, 'python3 -c ' + shlex.quote(code))
    return json.loads(result.split('__RESULT__')[-1].strip())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('candidate', help='D0またはdistributed-candidates直下のディレクトリ名')
    parser.add_argument('--validate-only', action='store_true', help='ローカル識別だけ照合し、SSH・変更を行わない')
    parser.add_argument('--cross-elite', choices=('on', 'off'), help='C010/C012で必須。3台の起動引数を同じ設定にする')
    args = parser.parse_args()
    version, binary, expected, files = candidate(args.candidate)
    manage.require((args.candidate in ('c010-cross-elite', 'c012-isolated-cooperation')) == (args.cross_elite is not None), 'C010/C012だけ--cross-elite on/offを必須指定')
    if args.cross_elite is not None:
        version += '-' + args.cross_elite
    manage.require(manage.sha(binary) == expected, '検証済みバイナリ識別不一致')
    if args.validate_only:
        print(json.dumps({'version': version, 'binarySha256': expected, 'sourceFiles': len(files)}))
        return
    with manage.locked():
        manage.require_idle()
        manage.require_no_server()
        old = manage.read(manage.CONFIG)
        generated = launchers(old, args.cross_elite)
        # 古い設定から開始し、固定相手・launch引数・CPU配置を引き継ぐ。
        updated = copy.deepcopy(old)
        updated['version'] = version
        updated['localFiles'] = {p: sha for p, sha in old['localFiles'].items()
                                 if not p.startswith('solver/distributed-candidates/')}
        updated['localFiles'].update(files)
        updated['localFiles'][str(Path(__file__).resolve().relative_to(ROOT))] = manage.sha(Path(__file__))
        for host in ('v1', 'v2', 'v3'):
            manage.require(not old['nodes'][host].get('fixed'), host + 'が固定対象')
            updated['nodes'][host]['files']['distributed-solver'] = expected
            updated['nodes'][host]['files']['launcher'] = hashlib.sha256(generated[host].encode()).hexdigest()
        manage.validate_config(updated)
        manage.require(manage.comparison_conditions(old) == manage.comparison_conditions(updated), '比較条件の変更を検出')
        stage = manage.WORK / 'switches' / (time.strftime('%Y%m%dT%H%M%S') + '-' + version)
        stage.mkdir(parents=True, exist_ok=False)
        for host, content in generated.items():
            (stage / (host + '-launcher')).write_text(content)
        manage.write(stage / 'before-config.json', old)
        manage.write(stage / 'planned-config.json', updated)
        record = {'from': old['version'], 'to': version, 'binary': str(binary), 'binarySha256': expected,
                  'startedUnix': time.time(), 'status': 'checking', 'stop': {}, 'install': {}, 'start': {}}
        def save():
            manage.write(stage / 'switch.json', record)
        save()
        started = []
        try:
            manage.write(stage / 'before.json', manage.snapshot(old))
            record['status'] = 'stopping'
            save()
            for host in ('v1', 'v3', 'v2'):
                record['stop'][host] = lifecycle.run_spec(host, old['nodes'][host], 'stop')
                save()
            record['status'] = 'installing'
            save()
            for host in ('v2', 'v3', 'v1'):
                record['install'][host] = remote_install(host, old['nodes'][host], binary, expected)
                launcher_sha = updated['nodes'][host]['files']['launcher']
                if old['nodes'][host]['files']['launcher'] != launcher_sha:
                    record['install'][host]['launcher'] = remote_install(host, old['nodes'][host], stage / (host + '-launcher'), launcher_sha, 'launcher')
                save()
            manage.write(manage.WORK / 'resume-config.json', updated)
            manage.configure(manage.WORK / 'resume-config.json')
            record['status'] = 'starting'
            save()
            for host in ('v2', 'v3', 'v1'):
                # 起動直後に失敗した場合も回収対象に含める。
                started.append(host)
                record['start'][host] = lifecycle.run_spec(host, updated['nodes'][host], 'start')
                save()
            manage.write(stage / 'after.json', manage.snapshot(updated))
            record.update(status='ready', finishedUnix=time.time())
            save()
        except BaseException as error:
            record.update(status='failed', error=str(error), cleanup={})
            for host in reversed(started):
                try:
                    record['cleanup'][host] = lifecycle.run_spec(host, updated['nodes'][host], 'stop')
                except Exception as cleanup_error:
                    record['cleanup'][host] = {'error': str(cleanup_error)}
            save()
            raise
        print(json.dumps({'status': record['status'], 'version': version, 'record': str(stage / 'switch.json')}, ensure_ascii=False))


if __name__ == '__main__':
    main()
