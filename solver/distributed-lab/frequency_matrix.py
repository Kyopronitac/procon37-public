#!/usr/bin/env python3
"""有限のPC間共有周期比較。固定4チーム、16x16のseed 0/3/6/9、候補ごと4件。"""
import argparse
import collections
import concurrent.futures
import copy
from datetime import datetime
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import sys
import time

import lifecycle
import manage
import observe
import switch

SEEDS = (0, 3, 6, 9)
VARIANTS = (('C014', 'c014-share100ms'), ('C015', 'c015-share2000ms'))
SELF = Path(__file__).resolve()


def write(path, value):
    manage.write(path, value)


def observe_nodes(cfg, phase, marker=None):
    clock = (manage.ROOT / 'solver/distributed-lab/clock_sync.py').read_text()
    def collect(host, node):
        code = observe.PROGRAM.replace('NODE', repr(node), 1).replace('PHASE', repr(phase), 1)
        code = code.replace('MARKER', repr((marker or {}).get('nodes', {}).get(host)), 1)
        code = code.replace('CLOCK_SOURCE', repr(clock), 1)
        if host == 'local':
            output = subprocess.run([sys.executable, '-c', code], capture_output=True, text=True,
                                    check=True, timeout=40).stdout
        else:
            output = lifecycle.execute(host, 'python3 -c ' + shlex.quote(code), timeout=45)
        return json.loads(output.split('__RESULT__')[-1].strip())
    nodes = dict(cfg['nodes'], local={'directory': str(manage.ROOT)})
    with concurrent.futures.ThreadPoolExecutor(max_workers=5) as pool:
        futures = {host: pool.submit(collect, host, node) for host, node in nodes.items()}
        data = {host: job.result() for host, job in futures.items()}
    for host in cfg['nodes']:
        manage.require(data[host]['files'] == cfg['nodes'][host]['files'], host + ': changed files')
    return {'phase': phase, 'createdUnix': time.time(), 'nodes': data}


def switch_binary(cfg, candidate_name, directory):
    version, binary, expected, sources = switch.candidate(candidate_name)
    manage.require(manage.sha(binary) == expected, 'candidate binary mismatch')
    manage.require(version.lower().startswith(candidate_name.split('-')[0]), 'candidate identity differs')
    updated = copy.deepcopy(cfg)
    updated['version'] = version + '-on-submit55-dedup'
    updated['localFiles'] = {p: sha for p, sha in cfg['localFiles'].items()
                             if not p.startswith('solver/distributed-candidates/')}
    updated['localFiles'].update(sources)
    for filename in ('source-manifest.json', 'validation.json'):
        path = manage.ROOT / 'solver/distributed-candidates' / candidate_name / filename
        updated['localFiles'][str(path.relative_to(manage.ROOT))] = manage.sha(path)
    # Binary identity is also checked between the four cases, not only at upload.
    updated['localFiles'][str(binary.relative_to(manage.ROOT))] = expected
    for host in ('v1', 'v2', 'v3'):
        updated['nodes'][host]['files']['distributed-solver'] = expected
    manage.validate_config(updated)
    manage.require(manage.comparison_conditions(cfg) == manage.comparison_conditions(updated), 'fixed conditions changed')
    for host in ('v1', 'v2', 'v3'):
        a = copy.deepcopy(cfg['nodes'][host]); b = copy.deepcopy(updated['nodes'][host])
        a['files'].pop('distributed-solver'); b['files'].pop('distributed-solver')
        manage.require(a == b, 'launcher/resources/bridge changed: ' + host)
    record = {'from': cfg['version'], 'to': updated['version'], 'binary': str(binary),
              'binarySha256': expected, 'sourceFiles': sources, 'status': 'stopping',
              'startedUnix': time.time(), 'stop': {}, 'install': {}, 'start': {}}
    write(directory / 'before-switch-config.json', cfg)
    write(directory / 'planned-config.json', updated)
    def save(): write(directory / 'switch.json', record)
    save()
    manage.require_no_server()
    for host in ('v1', 'v3', 'v2'):
        record['stop'][host] = lifecycle.run_spec(host, cfg['nodes'][host], 'stop'); save()
    record['status'] = 'installing'; save()
    for host in ('v2', 'v3', 'v1'):
        node = cfg['nodes'][host]
        # Preserve the old deployed binary by its verified digest before atomic replacement.
        old_sha = node['files']['distributed-solver']
        code = "import hashlib,pathlib,shutil; r=pathlib.Path(" + repr(node['directory']) + "); p=r/'distributed-solver'; expected=" + repr(old_sha) + "; assert hashlib.sha256(p.read_bytes()).hexdigest()==expected; b=r/('distributed-solver.backup-'+expected); shutil.copy2(p,b) if not b.exists() else None; assert hashlib.sha256(b.read_bytes()).hexdigest()==expected"
        lifecycle.execute(host, 'python3 -c ' + shlex.quote(code))
        record['install'][host] = switch.remote_install(host, node, binary, expected); save()
    write(manage.WORK / 'resume-config.json', updated)
    write(manage.CONFIG, updated)
    record['status'] = 'starting'; save()
    for host in ('v2', 'v3', 'v1'):
        record['start'][host] = lifecycle.run_spec(host, updated['nodes'][host], 'start'); save()
    record.update(status='ready', finishedUnix=time.time()); save()
    return updated


def launch(args):
    with manage.locked():
        manage.require_idle(); manage.require_no_server()
        directory = manage.stage_directory(args.stage)
        manage.require(not directory.exists(), 'matrix already exists; no overwrite or implicit rerun')
        cfg = manage.read(manage.CONFIG)
        cpus = set()
        for part in cfg['serverCpus'].split(','):
            bounds = list(map(int, part.split('-'))); cpus.update(range(bounds[0], bounds[-1] + 1))
        os.sched_setaffinity(0, cpus)
        policy = cfg['environmentIdentity']['submissionPolicy']
        manage.require(policy.get('skipUnchangedPayload') is True, 'dedup submission policy required')
        cfg['localFiles'][str(SELF.relative_to(manage.ROOT))] = manage.sha(SELF)
        write(manage.CONFIG, cfg); write(manage.WORK / 'resume-config.json', cfg)
        before = manage.snapshot(cfg)
        deadline = datetime.fromisoformat(args.deadline).timestamp()
        manage.require(deadline > time.time(), 'deadline passed')
        directory.mkdir(parents=True)
        problems = {}
        for seed in SEEDS:
            command = [str(manage.local_path(cfg['server'])), 'generate', '--seed', str(seed), '--players', '4',
                       '--label', '共有周期比較・日程確認のみ・seed' + str(seed)]
            problem = json.loads(subprocess.run(command, cwd=manage.ROOT, env=manage.clean_environment(),
                                               capture_output=True, text=True, timeout=30, check=True).stdout)
            manage.require(problem['map']['width'] == problem['map']['height'] == 16, 'expected 16x16')
            if args.time_limit_ms is not None:
                problem['daySeconds'] = [args.time_limit_ms // 1000] * len(problem['daySeconds'])
            problems[str(seed)] = problem
        seconds = sum((args.time_limit_ms / 1000 if args.time_limit_ms else 60)
                      + sum(problem['daySeconds']) + 5 for problem in problems.values())
        plan = {'stage': args.stage, 'seeds': list(SEEDS), 'variants': list(VARIANTS),
                'timeLimitMs': args.time_limit_ms, 'deadlineUnix': deadline,
                'deadlineIso': args.deadline, 'nominalVariantSeconds': seconds,
                'nominalTotalSeconds': seconds * len(VARIANTS), 'isFormalFullTest': False,
                'baselineReference': args.baseline_record, 'baselineRerun': False,
                'initialVersion': cfg['version'], 'harnessSha256': manage.sha(SELF),
                'createdUnix': time.time(), 'history': str(manage.ROOT / '.server/runs')}
        manage.require(time.time() + seconds * len(VARIANTS) + 90 < deadline, 'insufficient total time for finite matrix')
        write(directory / 'plan.json', plan); write(directory / 'initial-config.json', cfg)
        write(directory / 'initial-before.json', before); write(directory / 'expected-problems.json', problems)
        current = manage.state()
        evaluation = {'stage': args.stage, 'status': 'starting', 'startedUnix': time.time(),
                      'directory': str(directory), 'seeds': list(SEEDS), 'shortMs': args.time_limit_ms,
                      'deadlineUnix': deadline, 'runs': [], 'isFormalFullTest': False}
        current.update(stage='evaluating', evaluation=evaluation, activeEvaluation=True)
        write(manage.STATE, current)
        command = [sys.executable, str(SELF), 'execute', '--stage', args.stage]
        with (directory / 'matrix.log').open('xb') as log:
            child = subprocess.Popen(command, cwd=manage.ROOT, stdin=subprocess.DEVNULL,
                                     stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        evaluation.update(manage.process_identity(child.pid)); evaluation.update(status='running', command=command)
        write(directory / 'process.json', evaluation); current['evaluation'] = evaluation; write(manage.STATE, current)
        print(json.dumps(evaluation, ensure_ascii=False))


def execute(args):
    directory = manage.stage_directory(args.stage)
    # Parent registration is atomic under the shared lab lock.
    for _ in range(100):
        try:
            with manage.locked():
                manage.require(manage.state()['evaluation']['pid'] == os.getpid(), 'registration mismatch')
            break
        except BlockingIOError:
            time.sleep(.05)
    else: raise RuntimeError('parent did not release registration lock')
    plan = manage.read(directory / 'plan.json')
    expected_problems = manage.read(directory / 'expected-problems.json')
    report = {'stage': args.stage, 'status': 'running', 'variants': [], 'runs': [], 'startedUnix': time.time(),
              'isFormalFullTest': False, 'baselineReference': plan['baselineReference'], 'baselineRerun': False}
    child = None
    def save():
        write(directory / 'progress.json', report)
        current = manage.state(); manage.require(current['evaluation']['stage'] == args.stage, 'active stage changed')
        current['evaluation'].update(status=report['status'], runs=report['runs'],
                                     currentVariant=report.get('currentVariant'), currentSeed=report.get('currentSeed'))
        if report['status'] != 'running': current.update(activeEvaluation=False, stage='frequency_' + report['status'])
        write(manage.STATE, current)
    def interrupted(signum, frame): raise InterruptedError('matrix interrupted by signal ' + str(signum))
    signal.signal(signal.SIGTERM, interrupted); signal.signal(signal.SIGINT, interrupted)
    try:
        cfg = manage.read(manage.CONFIG)
        for version, candidate_name in plan['variants']:
            manage.require(time.time() + plan['nominalVariantSeconds'] + 35 < plan['deadlineUnix'], 'insufficient boundary budget')
            manage.require(manage.sha(SELF) == plan['harnessSha256'], 'harness changed')
            manage.require_no_server()
            name = 'frequency16-' + version + '-' + (str(plan['timeLimitMs'] // 1000) + 's' if plan['timeLimitMs'] else 'official') + '-01'
            variant_dir = manage.stage_directory(name)
            manage.require(not variant_dir.exists(), 'variant history exists: ' + name)
            variant_dir.mkdir(parents=True)
            report['currentVariant'] = version; save()
            write(variant_dir / 'before-switch-snapshot.json', manage.snapshot(cfg))
            cfg = switch_binary(cfg, candidate_name, variant_dir)
            before = manage.snapshot(cfg)
            write(variant_dir / 'manifest.json', cfg); write(variant_dir / 'before.json', before)
            marker = observe_nodes(cfg, 'mark'); write(variant_dir / 'observation-mark.json', marker)
            result = {'version': version, 'candidate': candidate_name, 'stage': name, 'runs': [], 'status': 'running'}
            write(variant_dir / 'progress.json', result)
            for seed in SEEDS:
                manage.require(manage.read(manage.CONFIG) == cfg, 'configuration changed during variant')
                manage.require(manage.sha(SELF) == plan['harnessSha256'], 'harness changed during variant')
                case_seconds = (plan['timeLimitMs'] / 1000 if plan['timeLimitMs'] else 60) + sum(expected_problems[str(seed)]['daySeconds']) + 5
                manage.require(time.time() + case_seconds + 20 < plan['deadlineUnix'], 'insufficient case budget')
                report['currentSeed'] = seed; save()
                label = f"分散{version}・PC間共有周期比較・16x16指定4件中seed{seed}・" + (f"短縮{plan['timeLimitMs']//1000}秒" if plan['timeLimitMs'] else '公式時間')
                case_cfg = {'count': None, 'seed': str(seed), 'format': None, 'arenaMaps': True,
                            'label': label, 'players': 4, 'timeLimitMs': plan['timeLimitMs'],
                            'teamNames': ['分散' + version, '固定並列', '固定第6世代', '固定第8世代'],
                            'serverProgram': 'tester/server/procon-server'}
                path = variant_dir / f'seed-{seed}-config.json'; write(path, case_cfg)
                old_ids = {p.name for p in (manage.ROOT / '.server/runs').iterdir() if p.is_dir()}
                command = [str(manage.local_path(cfg['server'])), 'run', '--seed', str(seed), '--players', '4',
                           '--config', str(path), '--label', label, '--match-port', '11451']
                if plan['timeLimitMs'] is not None: command += ['--time-limit-ms', str(plan['timeLimitMs'])]
                with (variant_dir / f'seed-{seed}-server.log').open('xb') as log:
                    child = subprocess.Popen(command, cwd=manage.ROOT, env=manage.clean_environment(),
                                             stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                    write(variant_dir / f'seed-{seed}-process.json', {'command': command, 'startedUnix': time.time(), **manage.process_identity(child.pid)})
                    if seed == SEEDS[0]:
                        time.sleep(8)
                        write(variant_dir / 'observation-start.json', observe_nodes(cfg, 'start', marker))
                    exit_code = child.wait(timeout=max(1, plan['deadlineUnix'] - time.time()))
                child = None
                new = [p for p in (manage.ROOT / '.server/runs').iterdir() if p.is_dir() and p.name not in old_ids]
                manage.require(len(new) == 1, 'ambiguous new run')
                run = manage.read(new[0] / 'run.json'); case = run['cases'][0]
                problem = manage.read(new[0] / str(seed) / 'problem.json')
                conditions = run['config']
                manage.require(problem == expected_problems[str(seed)], 'actual problem differs from frozen case')
                manage.require(conditions['seed'] == str(seed) and conditions['players'] == 4 and conditions['count'] is None
                               and conditions['format'] is None and conditions['arenaMaps'] is True
                               and conditions['timeLimitMs'] == plan['timeLimitMs'] and conditions['label'] == label, 'run conditions differ')
                manage.require(int(case['seed']) == seed and case['format'] == 'first', 'case seed/format differs')
                ac = (exit_code == 0 and run['status'] == 'completed' and run['verdict'] == 'AC'
                      and run['completed'] == run['total'] == len(run['cases']) == 1 and case['status'] == 'AC'
                      and sorted(p['id'] for p in case['players']) == list(range(4))
                      and all(p['status'] == 'AC' for p in case['players']))
                entry = {'version': version, 'seed': seed, 'runId': run['id'], 'exitCode': exit_code,
                         'all4AC': ac, 'problemSha256': manage.sha(new[0] / str(seed) / 'problem.json'), 'case': case}
                result['runs'].append(entry); report['runs'].append(entry); save()
                write(variant_dir / 'progress.json', result)
                print(json.dumps(entry, ensure_ascii=False), flush=True)
                manage.require(ac, 'non-AC case; matrix stops without retry')
            after = manage.snapshot(cfg); manage.unchanged(before, after)
            write(variant_dir / 'after.json', after)
            write(variant_dir / 'observation-end.json', observe_nodes(cfg, 'end', marker))
            result.update(status='verified', all16AC=True, verifiedUnchanged=True, players=[])
            for team in range(4):
                rows = [next(p for p in r['case']['players'] if p['id'] == team) for r in result['runs']]
                result['players'].append({'team': team, 'meanRank': sum(p['rank'] for p in rows) / 4,
                    'verdicts': dict(collections.Counter(p['status'] for p in rows)),
                    'scores': {m: sum(p['score'][m] for p in rows) for m in manage.METRICS}})
            write(variant_dir / 'results.json', result); report['variants'].append(result); save()
        report.update(status='verified', all32AC=True, finishedUnix=time.time())
        write(directory / 'results.json', report); save()
    except BaseException as error:
        if child is not None and child.poll() is None:
            os.killpg(child.pid, signal.SIGTERM)
            try: child.wait(timeout=5)
            except subprocess.TimeoutExpired: os.killpg(child.pid, signal.SIGKILL); child.wait()
        report.update(status='failed', error=str(error), finishedUnix=time.time(), cleanup={})
        cfg = manage.read(manage.CONFIG)
        for host in ('v1', 'v3', 'v2', 'nomlab', 'local2', 'local3'):
            try:
                spec = cfg['localTeams'][host] if host.startswith('local') else cfg['nodes'][host]
                report['cleanup'][host] = lifecycle.run_spec('local' if host.startswith('local') else host, spec, 'stop')
            except Exception as cleanup_error: report['cleanup'][host] = {'error': str(cleanup_error)}
        write(directory / 'failure.json', report); save(); raise


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('launch', 'execute'))
    parser.add_argument('--stage', default='frequency16-matrix-20261010-01')
    parser.add_argument('--deadline', help='timezone付きISO時刻。launchで必須')
    parser.add_argument('--time-limit-ms', type=int, default=None)
    parser.add_argument('--baseline-record', default='historical-user-selected-16x16-reference; no baseline rerun')
    args = parser.parse_args()
    if args.action == 'launch':
        manage.require(args.deadline is not None, '--deadline is required')
        manage.require(args.time_limit_ms is None or args.time_limit_ms > 0 and args.time_limit_ms % 1000 == 0,
                       'time limit must be positive whole seconds')
        launch(args)
    else: execute(args)
