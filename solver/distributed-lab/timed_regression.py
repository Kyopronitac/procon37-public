#!/usr/bin/env python3
"""提出時刻変更後の指定4ケースだけを、公式時間・固定4チームで再検証する。"""
import argparse
import collections
import json
import os
from pathlib import Path
import subprocess
import sys
import time

import manage

SEEDS = (3, 6, 7, 9)
BASELINE = '1791561513817407923-45859'
STAGE = 'C013-submit55-dedup-regression-01'
LABEL = '分散C013・55秒提出と5秒開始10秒毎確認・同一内容再提出省略・前回締切超過4件の再検証'


def check_case(run, seed):
    cfg = run['config']
    manage.require(run['status'] == 'completed' and run['completed'] == run['total'] == len(run['cases']) == 1,
                   'single case incomplete')
    manage.require(cfg['players'] == 4 and cfg['seed'] == str(seed) and cfg['count'] is None
                   and cfg['format'] is None and cfg['arenaMaps'] and cfg['timeLimitMs'] is None,
                   'single case conditions differ')
    case = run['cases'][0]
    manage.require(int(case['seed']) == seed and case['format'] == ('first', 'semifinal', 'final')[seed % 3],
                   'seed or format differs')
    manage.require(sorted(p['id'] for p in case['players']) == list(range(4)), 'missing team')
    return all(p['status'] == 'AC' for p in case['players']) and case['status'] == 'AC' and run['verdict'] == 'AC'


def launch():
    with manage.locked():
        manage.require_idle()
        manage.require_no_server()
        directory = manage.stage_directory(STAGE)
        manage.require(not directory.exists(), 'regression already exists; never overwrite/restart silently')
        cfg = manage.read(manage.CONFIG)
        manage.require(cfg['environmentIdentity'].get('submissionPolicy', {}).get('revision') == '20261010-user-submit55',
                       'new submission policy not configured')
        manage.require(cfg['environmentIdentity']['submissionPolicy'].get('skipUnchangedPayload') is True,
                       'unchanged-payload policy not configured')
        before = manage.snapshot(cfg)
        directory.mkdir(parents=True)
        manage.write(directory / 'manifest.json', cfg)
        manage.write(directory / 'before.json', before)
        evaluation = {'stage': STAGE, 'status': 'starting', 'label': LABEL, 'seeds': list(SEEDS),
                      'shortMs': None, 'isFormalFullTest': False, 'startedUnix': time.time(),
                      'directory': str(directory), 'history': str(manage.ROOT / '.server/runs'),
                      'version': cfg['version'], 'runs': []}
        current = manage.state()
        current.update(stage='evaluating', evaluation=evaluation, activeEvaluation=True)
        manage.write(manage.STATE, current)
        command = [sys.executable, str(Path(__file__).resolve()), 'execute', '--stage', STAGE]
        with (directory / 'batch.log').open('xb') as log:
            child = subprocess.Popen(command, cwd=manage.ROOT, stdin=subprocess.DEVNULL,
                                     stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        evaluation.update(manage.process_identity(child.pid))
        evaluation.update(status='running', command=command)
        manage.write(directory / 'process.json', evaluation)
        current['evaluation'] = evaluation
        manage.write(manage.STATE, current)
        print(json.dumps(evaluation, ensure_ascii=False))


def execute():
    directory = manage.stage_directory(STAGE)
    # Parent writes process identity before releasing the registration lock.
    deadline = time.monotonic() + 5
    while True:
        try:
            with manage.locked():
                cfg = manage.read(directory / 'manifest.json')
                manage.require(manage.state()['evaluation']['pid'] == os.getpid(), 'parent registration incomplete')
            break
        except BlockingIOError:
            if time.monotonic() >= deadline:
                raise
            time.sleep(.01)
    report = {'stage': STAGE, 'seeds': list(SEEDS), 'status': 'running', 'runs': [], 'isFormalFullTest': False}
    def save():
        manage.write(directory / 'progress.json', report)
        current = manage.state()
        manage.require(current['evaluation']['stage'] == STAGE, 'different active evaluation')
        current['evaluation']['runs'] = report['runs']
        current['evaluation']['currentSeed'] = report.get('currentSeed')
        if report['status'] != 'running':
            current['evaluation']['status'] = report['status']
            current['activeEvaluation'] = False
            current['stage'] = 'regression_' + report['status']
        manage.write(manage.STATE, current)
    try:
        for seed in SEEDS:
            manage.require(manage.read(manage.CONFIG) == cfg, 'configuration changed during batch')
            report['currentSeed'] = seed
            save()
            label = LABEL + '・seed' + str(seed)
            run_config = {'count': None, 'seed': str(seed), 'format': None, 'arenaMaps': True,
                          'label': label, 'players': 4, 'timeLimitMs': None,
                          'teamNames': ['分散C013・変更時提出', '固定並列・55秒', '固定第6世代・55秒', '固定第8世代・55秒'],
                          'serverProgram': 'tester/server/procon-server'}
            path = directory / ('seed-' + str(seed) + '-config.json')
            manage.write(path, run_config)
            old_ids = {p.name for p in (manage.ROOT / '.server/runs').iterdir() if p.is_dir()}
            command = [str(manage.local_path(cfg['server'])), 'run', '--seed', str(seed), '--players', '4',
                       '--config', str(path), '--label', label, '--match-port', '11451']
            with (directory / ('seed-' + str(seed) + '-server.log')).open('xb') as log:
                child = subprocess.Popen(command, cwd=manage.ROOT, env=manage.clean_environment(),
                                         stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
                manage.write(directory / ('seed-' + str(seed) + '-process.json'),
                             {'command': command, 'startedUnix': time.time(), **manage.process_identity(child.pid)})
                exit_code = child.wait()
            new = [p for p in (manage.ROOT / '.server/runs').iterdir() if p.is_dir() and p.name not in old_ids]
            manage.require(len(new) == 1, 'cannot unambiguously identify new run')
            run = manage.read(new[0] / 'run.json')
            ac = check_case(run, seed)
            manage.require(run['config']['label'] == label, 'run label differs')
            problem = new[0] / str(seed) / 'problem.json'
            previous = manage.ROOT / '.server/runs' / BASELINE / str(seed) / 'problem.json'
            manage.require(manage.read(problem) == manage.read(previous), 'actual problem differs from failed case')
            manage.require(all(s == 60 for s in manage.read(problem)['daySeconds']), 'daily official time changed')
            entry = {'seed': seed, 'runId': run['id'], 'exitCode': exit_code, 'all4AC': ac,
                     'problemMatchesFailedCase': True, 'case': run['cases'][0]}
            report['runs'].append(entry)
            save()
            print(json.dumps(entry, ensure_ascii=False), flush=True)
        manage.require(manage.read(manage.CONFIG) == cfg, 'configuration changed')
        after = manage.snapshot(cfg)
        manage.unchanged(manage.read(directory / 'before.json'), after)
        manage.write(directory / 'after.json', after)
        report['verifiedUnchanged'] = True
        report['all16AC'] = all(r['all4AC'] and r['exitCode'] == 0 for r in report['runs'])
        report['status'] = 'verified' if report['all16AC'] else 'failed'
        report['players'] = []
        for team in range(4):
            players = [next(p for p in r['case']['players'] if p['id'] == team) for r in report['runs']]
            report['players'].append({'team': team, 'verdicts': dict(collections.Counter(p['status'] for p in players)),
                                      'meanRank': sum(p['rank'] for p in players) / 4,
                                      'scores': {m: sum(p['score'][m] for p in players) for m in manage.METRICS}})
        report['finishedUnix'] = time.time()
        manage.write(directory / 'results.json', report)
        save()
    except BaseException as error:
        report.update(status='failed', error=str(error), finishedUnix=time.time())
        manage.write(directory / 'failure.json', report)
        save()
        raise


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('launch', 'execute'))
    parser.add_argument('--stage', default=STAGE)
    arguments = parser.parse_args()
    STAGE = arguments.stage
    manage.stage_directory(STAGE)
    launch() if arguments.action == 'launch' else execute()
