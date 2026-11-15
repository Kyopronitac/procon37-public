#!/usr/bin/env python3
"""Archive a verified server run and check its worker completion logs."""
import argparse
import json
from pathlib import Path
import re
import shutil

import manage


def worker_report(lines, count):
    inputs = []
    completions = []
    for line in lines:
        started = re.fullmatch(r'dist worker request=(\d+) job=(\d+) phase=(\w+) input_digest=([0-9a-f]+) started=(\d+) budget_ms=(\d+)', line)
        if started:
            request, job, phase, digest, threads, budget = started.groups()
            inputs.append({'request': int(request), 'job': int(job), 'phase': phase,
                           'digest': digest, 'started': int(threads), 'budgetMs': int(budget)})
        done = re.match(r'dist worker request=(\d+) completed=(\d+)/(\d+) selected_worker=(\d+) elapsed_ms=(\d+)', line)
        if done:
            request, completed, total, selected, elapsed = map(int, done.groups())
            completions.append({'request': request, 'completed': completed, 'total': total,
                                'selectedWorker': selected, 'elapsedMs': elapsed})
    return {'inputs': inputs, 'completions': completions, 'inputCount': len(inputs),
            'completionCount': len(completions),
            'requestOrderMatches': [x['request'] for x in inputs] == [x['request'] for x in completions],
            'allWorkersStarted': bool(inputs) and all(x['started'] == count for x in inputs),
            'allWorkersCompleted': bool(completions) and all(x['completed'] == x['total'] == count for x in completions)}


def cross_report(lines):
    rows = []
    for line in lines:
        if not line.startswith('cross_elite '):
            continue
        row = {}
        for key, value in re.findall(r'(\w+)=(Some\(\d+\)|None|\[[^]]*\]|\d+)', line):
            row[key] = (int(value[5:-1]) if value.startswith('Some(') else
                        None if value == 'None' else json.loads(value))
        rows.append(row)
    scalar = ('published', 'received', 'sent', 'imports', 'improved', 'checks', 'eligible',
              'probes', 'probe_ns', 'repair_ns', 'rejected', 'context_mismatch', 'oversized', 'duplicates')
    return {'rows': rows, 'observedDays': len(rows),
            'totals': {k: sum(r.get(k, 0) for r in rows) for k in scalar},
            'zeroCheckDays': sum(r.get('checks', 0) == 0 for r in rows),
            'maxima': {k: max((r.get(k, 0) for r in rows), default=0)
                       for k in ('max_stall_ms', 'max_receiver_mailbox_age_ms')}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage')
    args = parser.parse_args()
    source = manage.stage_directory(args.stage)
    result = manage.read(source / 'results.json')
    manage.require(result['all48AC'] and result['verifiedUnchanged'], 'run not verified')
    run = manage.read(source / 'run.json')
    observations = manage.WORK / 'observations' / args.stage
    end = manage.read(observations / 'end.json')
    cfg = manage.read(source / 'manifest.json')
    manage.require(end['configurationSha256'] == manage.sha(source / 'manifest.json'), 'observation configuration mismatch')
    destination = manage.ROOT / 'solver/docs/distributed/resume-20261009/evaluations' / args.stage
    destination.mkdir(parents=True, exist_ok=True)
    for name in ('manifest.json', 'before.json', 'after.json', 'run-config.json', 'process.json', 'run.json', 'results.json'):
        shutil.copyfile(source / name, destination / name)
    for phase in ('mark', 'start', 'end'):
        shutil.copyfile(observations / (phase + '.json'), destination / ('observation-' + phase + '.json'))
    history = manage.ROOT / '.server/runs' / run['id']
    artifacts = [{'path': str(p.relative_to(manage.ROOT)), 'bytes': p.stat().st_size, 'sha256': manage.sha(p)}
                 for p in sorted(history.rglob('*')) if p.is_file()]
    manage.write(destination / 'history-artifacts.json', {'runId': run['id'], 'files': artifacts,
                 'count': len(artifacts), 'totalBytes': sum(x['bytes'] for x in artifacts)})
    days = {c['seed']: len(manage.read(history / c['seed'] / 'problem.json')['daySteps']) for c in run['cases']}
    expected = sum(days.values()) + len(days)
    expected_order = [(request, 'select' if request == 1 else 'plan')
                      for count in days.values() for request in range(1, count + 2)]
    workers = {h: worker_report(end['nodes'][h]['logs']['resume-team.log']['diagnosticLines'], n)
               for h, n in (('v2', 15), ('v3', 31))}
    sequence = lambda h: [(x['request'], x['job'], x['phase'], x['digest']) for x in workers[h]['inputs']]
    complete = all(v['inputCount'] == v['completionCount'] == expected and v['allWorkersStarted']
                   and v['allWorkersCompleted'] and v['requestOrderMatches']
                   and [(x['request'], x['phase']) for x in v['inputs']] == expected_order
                   for v in workers.values())
    summary = {'stage': args.stage, 'runId': run['id'], 'expectedRequests': expected, 'days': days,
               'workers': workers, 'allExpectedWorkerRequestsComplete': complete,
               'workerInputSequencesMatch': sequence('v2') == sequence('v3'),
               'clock': {h: v['clockProbe'] for h, v in end['nodes'].items()},
               'errorLines': {h: {n: v['errorLines'] for n, v in node['logs'].items()} for h, node in end['nodes'].items()},
               'parserSha256': manage.sha(Path(__file__))}
    summary['crossElite'] = {h: cross_report(v['logs'].get('stderr.log' if h == 'v1' else 'resume-team.log', {}).get('diagnosticLines', []))
                             for h, v in end['nodes'].items() if h in ('v1', 'v2', 'v3')}
    summary['scope'] = 'v2/v3 complete appended intervals; v1 stderr final case only. Cross counters measure mechanism, not match improvement.'
    manage.write(destination / 'participation-completion.json', summary)
    manage.require(complete and summary['workerInputSequencesMatch'], 'worker participation requires investigation')
    print(json.dumps({'record': str(destination), 'requests': expected, 'allWorkerRequestsComplete': complete,
                      'result': result['players'][0]}, ensure_ascii=False))


if __name__ == '__main__':
    main()
