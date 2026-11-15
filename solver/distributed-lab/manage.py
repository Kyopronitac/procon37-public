#!/usr/bin/env python3
"""再開した分散実験の配置照合・12件評価・結果確定。遠隔への書込みは行わない。"""
import argparse
import collections
import concurrent.futures
import contextlib
import fcntl
import hashlib
import json
import os
import re
import shlex
import subprocess
import sys
import time
from pathlib import Path
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / '.server/distributed'
STATE = ROOT / '.server/distributed-state.json'
CONFIG = WORK / 'lab-config.json'
METRICS = ('uniqueBrands', 'dailyBrands', 'udon')
PROTECTED = ('tester/game/samples', 'solver/docs/solver-pool-experiment')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read(path):
    return json.loads(Path(path).read_text())


def write(path, value):
    path = Path(path)
    temporary = path.with_name(path.name + '.tmp')
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')
    temporary.replace(path)


def sha(path):
    digest = hashlib.sha256()
    with Path(path).open('rb') as source:
        for data in iter(lambda: source.read(1024 * 1024), b''):
            digest.update(data)
    return digest.hexdigest()


def local_path(name):
    path = (ROOT / name).resolve()
    require(path.is_relative_to(ROOT), 'ローカル対象はこのリポジトリ内に限定する')
    require(not any(path.is_relative_to(ROOT / x) for x in PROTECTED), '保護対象はハーネスの照合対象にしない')
    return path


def stage_directory(stage):
    require(re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._-]*', stage), 'stageは英数字・.・_・-のみ')
    return WORK / 'evaluations' / stage


def process_identity(pid):
    try:
        base = Path('/proc') / str(pid)
        fields = (base / 'stat').read_text().rsplit(')', 1)[1].split()
        return {'pid': int(pid), 'startTicks': fields[19], 'state': fields[0],
                'bootId': Path('/proc/sys/kernel/random/boot_id').read_text().strip()}
    except (FileNotFoundError, ProcessLookupError):
        return None


def alive(record):
    current = process_identity(record['pid']) if record and record.get('pid') else None
    return bool(current and current['state'] != 'Z' and
                all(current[key] == record.get(key) for key in ('pid', 'startTicks', 'bootId')))


@contextlib.contextmanager
def locked():
    WORK.mkdir(parents=True, exist_ok=True)
    with (WORK / 'lab.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield


def state():
    return read(STATE) if STATE.exists() else {}


def require_idle():
    evaluation = state().get('evaluation', {})
    require(evaluation.get('status') not in ('starting', 'running'),
            '評価が未確定。summarizeまたは終了後のrelease-failedを先に実行する')


def require_no_server():
    for process in Path('/proc').iterdir():
        if not process.name.isdigit():
            continue
        try:
            argv = (process / 'cmdline').read_bytes().decode(errors='replace').split('\0')
            if argv and Path(argv[0]).name == 'server' and 'run' in argv and (process / 'cwd').resolve() == ROOT:
                raise ValueError('このリポジトリのserver runが残っている: PID ' + process.name)
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            pass


def validate_config(config):
    require(set(config['nodes']) == {'v1', 'v2', 'v3', 'nomlab'}, '4台すべてを登録する')
    require(config['nodes']['nomlab'].get('fixed') is True, 'NOMLABはfixed必須')
    require(config.get('fixedLocalFiles') and config.get('localProcesses'), '固定相手とローカル参加プロセスが必要')
    require(config.get('version'), '版名が必要')
    for section in ('localFiles', 'fixedLocalFiles'):
        for name, expected in config[section].items():
            local_path(name)
            require(re.fullmatch('[0-9a-f]{64}', expected), name + ': SHA256が必要')
    require(config.get('server', 'target/release/server') in config['fixedLocalFiles'], 'server実行ファイルを固定対象へ登録する')
    require('tester/server/procon-server' in config['fixedLocalFiles'], '公式サーバーを固定対象へ登録する')
    endpoint = urlparse(config['serverUrl'])
    require(endpoint.scheme == 'http' and endpoint.hostname and endpoint.port, 'serverUrlにはhttp://IP:PORTを指定する')
    for name, node in config['nodes'].items():
        directory = Path(node['directory'])
        require(Path(node['home']).is_absolute() and directory.is_absolute() and '..' not in directory.parts
                and directory.is_relative_to(Path(node['home'])), name + ': home配下の絶対directoryが必要')
        require(node['processes'], name + ': 必須プロセスが必要')
        require(re.fullmatch(r'[A-Za-z0-9._-]+', node['user']), name + ': SSH user不正')
        require(re.fullmatch(r'[A-Za-z0-9.:-]+', node['address']), name + ': SSH address不正')
        require(re.fullmatch(r'[0-9,-]+', node['cpus']), name + ': CPU指定不正')
        for filename, expected in node['files'].items():
            require(not Path(filename).is_absolute() and '..' not in Path(filename).parts, '遠隔ファイルはdirectory内の相対パスのみ')
            require(re.fullmatch('[0-9a-f]{64}', expected), filename + ': SHA256が必要')


def fixed_identity(config):
    return {'localFiles': config['fixedLocalFiles'], 'localProcesses': config['localProcesses'],
            'nodes': {key: value for key, value in config['nodes'].items() if value.get('fixed')}}


def configure(path):
    require_idle()
    require_no_server()
    config = read(path)
    validate_config(config)
    if CONFIG.exists():
        require(fixed_identity(read(CONFIG)) == fixed_identity(config), '固定相手・基盤の登録内容変更は禁止。別条件の評価は新しい実験として記録する')
    write(CONFIG, config)
    current = state()
    current.update(stage='configured', configuration=str(CONFIG), repository=str(ROOT),
                   adoptedVersion=current.get('adoptedVersion', 'D0'))
    write(STATE, current)
    return {'configuration': str(CONFIG), 'configurationSha256': sha(CONFIG), 'version': config['version']}


PROBE = r'''
import hashlib,json,pathlib
cfg=CONFIGURATION
files={}
for name in cfg['files']:
    p=pathlib.Path(cfg['directory'])/name
    h=hashlib.sha256()
    with p.open('rb') as f:
        for data in iter(lambda:f.read(1048576),b''):h.update(data)
    files[name]=h.hexdigest()
processes=[]
for p in pathlib.Path('/proc').iterdir():
    if not p.name.isdigit():continue
    try:
        argv=(p/'cmdline').read_bytes().decode(errors='replace').split('\0')
        if not any(x['matchArg'] in argv for x in cfg['processes']):continue
        status=dict(x.split(':',1) for x in (p/'status').read_text().splitlines() if ':' in x)
        fields=(p/'stat').read_text().rsplit(')',1)[1].split()
        if fields[0]=='Z':continue
        processes.append({'pid':int(p.name),'startTicks':fields[19],'argv':argv,
                          'cpus':status['Cpus_allowed_list'].strip(),'threads':int(status['Threads'])})
    except (FileNotFoundError,PermissionError,ProcessLookupError):pass
print(json.dumps({'files':files,'processes':processes,'bootId':pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip()}))
'''


def ssh_probe(node):
    program = PROBE.replace('CONFIGURATION', repr(node), 1)
    command = 'python3 -c ' + shlex.quote(program)
    options = ['-o', 'StrictHostKeyChecking=accept-new', '-o', 'ConnectTimeout=8']
    destination = node['user'] + '@' + node['address']
    password = os.environ.get('PROCON37_SSH_PASSWORD')
    if not password:
        result = subprocess.run(['ssh', *options, '-o', 'BatchMode=yes', destination, command],
                                capture_output=True, text=True, timeout=40, check=True).stdout
    else:
        import pexpect
        child = pexpect.spawn('ssh', [*options, destination, command], encoding='utf-8', timeout=40, echo=False)
        output = []
        try:
            while True:
                event = child.expect([r'(?i)password:', pexpect.EOF])
                output.append(child.before)
                if event == 1:
                    child.close()
                    require(child.exitstatus == 0, destination + ': SSH照合失敗')
                    break
                child.sendline(password)
        finally:
            if child.isalive():
                child.close(force=True)
        result = ''.join(output)
    return json.loads(next(line for line in result.splitlines() if line.startswith('{')))


def check_processes(expected, found, host):
    for item in expected:
        matching = [p for p in found if item['matchArg'] in p['argv']]
        require(matching, host + ': 必須プロセスが不在: ' + item['matchArg'])
        require(all(p['cpus'] == item['cpus'] for p in matching), host + ': CPU affinity相違')
        if 'threads' in item:
            require(all(p['threads'] == item['threads'] for p in matching), host + ': thread数相違')


def snapshot(config):
    validate_config(config)
    expected = {**config['localFiles'], **config['fixedLocalFiles']}
    d0 = read(ROOT / 'solver/docs/distributed/initial-formal-preflight.json')['sourceFiles']
    expected.update({'solver/distributed/' + name: value for name, value in d0.items()})
    local = {name: sha(local_path(name)) for name in expected}
    require(local == expected, 'ローカルファイルが登録SHAと不一致')
    local_probe = { 'directory': str(ROOT), 'files': [], 'processes': config['localProcesses'] }
    result = subprocess.run([sys.executable, '-c', PROBE.replace('CONFIGURATION', repr(local_probe), 1)],
                            text=True, capture_output=True, check=True)
    local_processes = json.loads(result.stdout)
    check_processes(config['localProcesses'], local_processes['processes'], 'local')
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        futures = {name: pool.submit(ssh_probe, node) for name, node in config['nodes'].items()}
        remote = {name: future.result() for name, future in futures.items()}
    for name, data in remote.items():
        node = config['nodes'][name]
        require(data['files'] == node['files'], name + ': 遠隔ファイルが登録SHAと不一致')
        check_processes(node['processes'], data['processes'], name)
    return {'createdUnix': time.time(), 'configurationSha256': sha(CONFIG), 'localFiles': local,
            'local': local_processes, 'remote': remote, 'harnessSha256': sha(Path(__file__))}


def unchanged(before, after):
    for key in ('configurationSha256', 'localFiles', 'harnessSha256'):
        require(before[key] == after[key], '評価中の変更: ' + key)
    require(before['local']['bootId'] == after['local']['bootId'], '評価中のlocal再起動')
    require(set(before['remote']) == set(after['remote']), '評価中のホスト構成変更')
    for host in before['remote']:
        for key in ('files', 'bootId'):
            require(before['remote'][host][key] == after['remote'][host][key], host + ': 評価中の変更: ' + key)


def clean_environment():
    return {k: v for k, v in os.environ.items()
            if not k.startswith('PROCON37_') and k not in ('SELECT_MS', 'TIME_LIMIT_MS')}


def launch(stage, label, short_ms):
    require_idle()
    require_no_server()
    require(label.strip(), 'labelは空白だけにできない')
    require(short_ms is None or short_ms > 0, '短縮時間は正の整数')
    directory = stage_directory(stage)
    require(not directory.exists(), '既存stageは上書きしない')
    config = read(CONFIG)
    before = snapshot(config)  # offline / 必須プロセス不在なら作成・起動しない
    directory.mkdir(parents=True)
    write(directory / 'manifest.json', config)
    write(directory / 'before.json', before)
    run_config = {'count': 12, 'players': 4, 'seed': None, 'format': None, 'arenaMaps': True,
                  'label': label, 'timeLimitMs': short_ms,
                  'teamNames': ['分散' + config['version'], '固定並列', '固定第6世代', '固定第8世代'],
                  'serverProgram': 'tester/server/procon-server'}
    write(directory / 'run-config.json', run_config)
    server = local_path(config.get('server', 'target/release/server'))
    command = [str(server), 'run', '12', '--players', '4', '--config', str(directory / 'run-config.json'),
               '--label', label, '--match-port', str(urlparse(config['serverUrl']).port)]
    if short_ms is not None:
        command += ['--time-limit-ms', str(short_ms)]
    evaluation = {'stage': stage, 'status': 'starting', 'label': label, 'shortMs': short_ms,
                  'startedUnix': time.time(), 'command': command, 'directory': str(directory),
                  'history': str(ROOT / '.server/runs'), 'version': config['version'],
                  'existingRunIds': sorted(p.name for p in (ROOT / '.server/runs').glob('*') if p.is_dir())}
    current = state()
    current.update(stage='evaluating', evaluation=evaluation, activeEvaluation=True)
    write(STATE, current)
    try:
        with (directory / 'server.log').open('xb') as log:
            process = subprocess.Popen(command, cwd=ROOT, env=clean_environment(), stdin=subprocess.DEVNULL,
                                       stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        evaluation.update(process_identity(process.pid) or {'pid': process.pid})
        evaluation['status'] = 'running'
        write(directory / 'process.json', evaluation)
        current['evaluation'] = evaluation
        write(STATE, current)
    except Exception:
        evaluation['status'] = 'launch-failed'
        write(directory / 'process.json', evaluation)
        current['evaluation'] = evaluation
        current['activeEvaluation'] = False
        write(STATE, current)
        raise
    return evaluation


def summarize_run(run, short_ms, label):
    config = run['config']
    require(run['status'] == 'completed' and run['verdict'] == 'AC', '未完了または非AC')
    require(run['completed'] == run['total'] == len(run['cases']) == 12, '12件未完了')
    require(config['count'] == 12 and config['players'] == 4, '件数・チーム数相違')
    require(config.get('seed') is None and config.get('format') is None and config['arenaMaps'] is True, '生成条件相違')
    require(config.get('timeLimitMs') == short_ms and config['label'] == label, '時間またはlabel相違')
    require(sorted(int(c['seed']) for c in run['cases']) == list(range(12)), 'seed相違')
    require(collections.Counter(c['format'] for c in run['cases']) == {'first': 4, 'semifinal': 4, 'final': 4}, '形式相違')
    rows = [{'team': i, 'rankSum': 0, 'scores': dict.fromkeys(METRICS, 0)} for i in range(4)]
    for case in run['cases']:
        require(case['status'] == 'AC' and sorted(p['id'] for p in case['players']) == list(range(4)), '4チームAC未成立')
        for player in case['players']:
            require(player['status'] == 'AC' and 1 <= player['rank'] <= 4, '非ACまたは順位不正')
            row = rows[player['id']]
            row['rankSum'] += player['rank']
            for metric in METRICS:
                row['scores'][metric] += player['score'][metric]
    for row in rows:
        row['meanRank'] = row['rankSum'] / 12
    return {'runId': run['id'], 'all48AC': True, 'players': rows}


def comparison_conditions(config):
    return {'fixed': fixed_identity(config), 'serverUrl': config['serverUrl'],
            'environmentIdentity': config.get('environmentIdentity'),
            'placement': {name: {k: node[k] for k in ('user', 'address', 'home', 'directory', 'cpus')}
                          for name, node in config['nodes'].items()}}


def finish(stage, run_id, before_stage):
    directory = stage_directory(stage)
    evaluation = read(directory / 'process.json')
    require(not alive(evaluation), '評価プロセスが実行中')
    require(re.fullmatch(r'[A-Za-z0-9._-]+', run_id), 'run ID不正')
    require(run_id not in evaluation['existingRunIds'], '開始前から存在するrunは流用できない')
    run_path = ROOT / '.server/runs' / run_id / 'run.json'
    require(run_path.stat().st_mtime >= evaluation['startedUnix'] - 1, '開始前のrunは流用できない')
    run = read(run_path)
    require(run['id'] == run_id, 'run ID相違')
    report = summarize_run(run, evaluation['shortMs'], evaluation['label'])
    config = read(directory / 'manifest.json')
    require(config == read(CONFIG), '評価後に登録configが変更されている')
    after = snapshot(config)
    unchanged(read(directory / 'before.json'), after)
    report.update(version=config['version'], stage=stage, verifiedUnchanged=True, config=run['config'])
    if before_stage:
        previous_dir = stage_directory(before_stage)
        previous = read(previous_dir / 'results.json')
        previous_config = read(previous_dir / 'manifest.json')
        require(comparison_conditions(previous_config) == comparison_conditions(config), '比較相手または配置条件が異なる。D0を同条件で再評価する')
        previous_run = read(previous_dir / 'run.json')
        summarize_run(previous_run, evaluation['shortMs'], previous_run['config']['label'])
        old = {int(c['seed']): c for c in previous_run['cases']}
        counts = {'win': 0, 'tie': 0, 'loss': 0}
        for case in run['cases']:
            def score(c):
                p = next(p for p in c['players'] if p['id'] == 0)
                return tuple(p['score'][m] for m in METRICS)
            a, b = score(case), score(old[int(case['seed'])])
            counts['win' if a > b else 'loss' if a < b else 'tie'] += 1
        report.update(comparison=counts, beforeRunId=previous['runId'], before=previous['players'][0])
    write(directory / 'after.json', after)
    write(directory / 'run.json', run)
    write(directory / 'results.json', report)
    current = state()
    if current.get('evaluation', {}).get('stage') == stage:
        current['evaluation'].update(status='verified', runId=run_id, results=str(directory / 'results.json'))
        current['stage'] = 'evaluated'
        current['activeEvaluation'] = False
        write(STATE, current)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='action', required=True)
    init = commands.add_parser('configure')
    init.add_argument('config')
    snap = commands.add_parser('snapshot')
    snap.add_argument('--save', required=True)
    snap.add_argument('--before')
    run = commands.add_parser('run')
    run.add_argument('stage')
    run.add_argument('--label', required=True)
    run.add_argument('--short-ms', type=int)
    summary = commands.add_parser('summarize')
    summary.add_argument('stage')
    summary.add_argument('--run-id', required=True)
    summary.add_argument('--before-stage')
    failed = commands.add_parser('release-failed')
    failed.add_argument('stage')
    failed.add_argument('--reason', required=True)
    commands.add_parser('status')
    args = parser.parse_args()
    with locked():
        if args.action == 'configure':
            result = configure(args.config)
        elif args.action == 'run':
            result = launch(args.stage, args.label, args.short_ms)
        elif args.action == 'summarize':
            result = finish(args.stage, args.run_id, args.before_stage)
        elif args.action == 'snapshot':
            target = local_path(args.save)
            require(target.is_relative_to(WORK) and not target.exists(), 'snapshot保存先は.server/distributed内の新規ファイル')
            result = snapshot(read(CONFIG))
            if args.before:
                unchanged(read(local_path(args.before)), result)
                result['verifiedUnchanged'] = True
            target.parent.mkdir(parents=True, exist_ok=True)
            write(target, result)
            result = {'saved': str(target), 'verifiedUnchanged': result.get('verifiedUnchanged', False)}
        elif args.action == 'release-failed':
            current = state()
            evaluation = current.get('evaluation', {})
            require(evaluation.get('stage') == args.stage and not alive(evaluation), '指定評価が実行中または不一致')
            require(evaluation.get('status') in ('starting', 'running', 'launch-failed'), '未確定の失敗評価だけ解除できる')
            require_no_server()
            require(args.reason.strip(), '失敗理由が必要')
            evaluation.update(status='failed', failureReason=args.reason, releasedUnix=time.time())
            current['stage'] = 'evaluation_failed'
            current['activeEvaluation'] = False
            write(stage_directory(args.stage) / 'failure.json', evaluation)
            write(STATE, current)
            result = evaluation
        else:
            result = state()
            if result.get('evaluation'):
                result['evaluation']['processAlive'] = alive(result['evaluation'])
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        sys.exit(str(error))
