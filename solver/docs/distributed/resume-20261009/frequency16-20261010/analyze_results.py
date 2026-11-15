#!/usr/bin/env python3
"""Read frozen evaluation results; write only this directory's derived analysis."""
from pathlib import Path
import collections
import datetime
import hashlib
import json
import re

DIRECTORY = Path(__file__).resolve().parent
ROOT = Path(__file__).resolve().parents[5]
METRICS = ('uniqueBrands', 'dailyBrands', 'udon')
SEEDS = (0, 3, 6, 9)
CONSISTENT = (3, 6, 9)


def read(path):
    return json.loads(path.read_text())


def aggregate(cases, seeds):
    rows = [case for case in cases if int(case['seed']) in seeds]
    if not rows:
        return None
    out = []
    for team in range(4):
        players = [next(p for p in case['players'] if p['id'] == team) for case in rows]
        out.append({'team': team, 'cases': len(players),
                    'meanRank': sum(p['rank'] for p in players) / len(players),
                    'scores': {k: sum(p['score'][k] for p in players) for k in METRICS},
                    'statuses': dict(collections.Counter(p['status'] for p in players))})
    return out


def difference(left, right):
    if left is None or right is None:
        return None
    return [{'team': a['team'], 'meanRankDelta': a['meanRank'] - b['meanRank'],
             'scoreDeltas': {k: a['scores'][k] - b['scores'][k] for k in METRICS}}
            for a, b in zip(left, right)]


def identity_comparison(directory):
    if not (directory / 'after.json').exists():
        return {'status': 'pending', 'allEqual': False}
    before, after = (read(directory / name) for name in ('before.json', 'after.json'))
    checks = {key: before[key] == after[key] for key in ('configurationSha256', 'localFiles', 'harnessSha256')}
    checks['localBootId'] = before['local']['bootId'] == after['local']['bootId']
    checks['remoteHostSet'] = set(before['remote']) == set(after['remote'])
    for host in sorted(set(before['remote']) | set(after['remote'])):
        for key in ('files', 'bootId'):
            checks[f'{host}.{key}'] = before['remote'].get(host, {}).get(key) == after['remote'].get(host, {}).get(key)
    return {'status': 'compared-from-saved-snapshots', 'allEqual': all(checks.values()), 'checks': checks}


def communication(path):
    if not path.exists():
        return {'status': 'not-yet-observed'}
    observation = read(path)
    lines = {h: observation['nodes'][h]['logs']['resume-team.log']['diagnosticLines']
             for h in ('v1', 'v2', 'v3')}
    coordinator = []
    for line in lines['v1']:
        if line.startswith('dist request='):
            match = re.search(r'request=(\d+) external_request=(\d+) phase=(\w+) input_digest=([0-9a-f]+).*selected_worker=(\d+).*local_completed=(\d+)/(\d+) remote_results=(\d+)/(\d+)', line)
            if not match:
                raise ValueError('Unexpected coordinator diagnostic')
            a = match.groups()
            coordinator.append({'request': int(a[0]), 'phase': a[2], 'digest': a[3],
                                'worker': int(a[4]), 'local': [int(a[5]), int(a[6])],
                                'remote': [int(a[7]), int(a[8])]})
    expected = [(x['request'], x['phase'], x['digest']) for x in coordinator]
    hosts = {}
    for host, count in (('v1', 31), ('v2', 15), ('v3', 31)):
        stats = []
        starts, finished = [], []
        for line in lines[host]:
            if line.startswith('cross_elite '):
                stats.append({k: int(v) for k, v in re.findall(r'(\w+)=([0-9]+)(?:\s|$)', line)})
            if line.startswith('dist worker request=') and ' phase=' in line:
                match = re.search(r'request=(\d+) job=(\d+) phase=(\w+) input_digest=([0-9a-f]+) started=(\d+)', line)
                a = match.groups()
                starts.append((int(a[0]), a[2], a[3]))
            if line.startswith('dist worker request=') and ' completed=' in line:
                match = re.search(r'request=(\d+) completed=(\d+)/(\d+)', line)
                finished.append(tuple(map(int, match.groups())))
        hosts[host] = {
            'planCounterRows': len(stats),
            'counters': {k: sum(x[k] for x in stats) for k in ('published', 'received', 'imports', 'improved', 'sent', 'rejected', 'context_mismatch', 'oversized', 'checks', 'eligible', 'probes', 'probe_ns', 'repair_ns', 'duplicates')},
            'maxStallMs': max((x['max_stall_ms'] for x in stats), default=0),
            'maxReceiverMailboxAgeMs': max((x['max_receiver_mailbox_age_ms'] for x in stats), default=0),
            'probeSeconds': sum(x['probe_ns'] for x in stats) / 1e9,
            'repairSeconds': sum(x['repair_ns'] for x in stats) / 1e9,
            'zeroCheckDays': sum(x['checks'] == 0 for x in stats),
        }
        if host != 'v1':
            hosts[host].update(requestCount=len(starts), inputSequenceMatchesV1=starts == expected,
                allExpectedThreadsCompleted=len(finished) == 32 and all(a == b == count for _, a, b in finished),
                completionRequestOrderMatches=[x[0] for x in finished] == [x[0] for x in starts])
    return {'status': 'observed', 'source': str(path.relative_to(ROOT)),
            'sourceSha256': hashlib.sha256(path.read_bytes()).hexdigest(), 'legacyStderrIgnored': True,
            'requestCount': len(coordinator), 'phaseCounts': dict(collections.Counter(x['phase'] for x in coordinator)),
            'allV1LocalAndRemoteComplete': len(coordinator) == 32 and all(x['local'] == [31, 31] and x['remote'] == [2, 2] for x in coordinator),
            'finalReceiverWorkerSelections': sum(x['phase'] == 'plan' and x['worker'] in (7, 35, 49) for x in coordinator),
            'hosts': hosts,
            'interpretation': 'Receiver selection/import counters do not prove foreign-solution ancestry of the submitted answer or match strength. Sent counts packets, received counts novel offers, and probes/repair timing cover receiver work, not network or serialization overhead. Mailbox age is age when considered, not end-to-end network latency.'}


def main():
    baseline = read(DIRECTORY / 'baseline.json')
    report = {'createdAt': datetime.datetime.now().astimezone().isoformat(),
              'scope': '16x16 official-time4cases; no new baseline run',
              'limitations': baseline['limitations'] + ['Fixed opponents also vary between timed matches; no lexicographic score win/tie/loss metric is used.'],
              'reference4': aggregate(baseline['cases'], SEEDS),
              'referenceC013Only3': aggregate(baseline['cases'], CONSISTENT), 'variants': {}}
    full_cases = {}
    for variant in ('C014', 'C015'):
        directory = ROOT / '.server/distributed/evaluations' / f'frequency16-{variant}-official-01'
        source = directory / 'results.json'
        if not source.exists():
            source = directory / 'progress.json'
        if not source.exists():
            report['variants'][variant] = {'status': 'not-started', 'completedSeeds': []}
            continue
        data = read(source)
        cases = [item['case'] for item in data.get('runs', [])]
        seeds = [int(case['seed']) for case in cases]
        full_cases[variant] = cases
        current = aggregate(cases, seeds)
        reference = aggregate(baseline['cases'], seeds)
        identity = identity_comparison(directory)
        complete = sorted(seeds) == list(SEEDS) and all(len(case['players']) == 4 and all(p['status'] == 'AC' for p in case['players']) for case in cases)
        rows = []
        for case in cases:
            previous = next(c for c in baseline['cases'] if int(c['seed']) == int(case['seed']))
            for player in case['players']:
                old = next(p for p in previous['players'] if p['id'] == player['id'])
                rows.append({'seed': int(case['seed']), 'referenceVersion': previous['version'],
                             'team': player['id'], 'rank': player['rank'], 'rankReference': old['rank'],
                             'rankDelta': player['rank'] - old['rank'], 'score': player['score'],
                             'scoreReference': old['score'],
                             'scoreDeltas': {k: player['score'][k] - old['score'][k] for k in METRICS}})
        report['variants'][variant] = {'status': data['status'], 'completedSeeds': seeds,
            'acJudgements': sum(p['status'] == 'AC' for case in cases for p in case['players']),
            'totalJudgements': sum(len(case['players']) for case in cases),
            'isCompleteVerified': complete and identity['allEqual'] and data.get('status') == 'verified' and data.get('all16AC') is True and data.get('verifiedUnchanged') is True,
            'independentBeforeAfterComparison': identity,
            'completionEvidence': {name: {'source': str((directory / name).relative_to(ROOT)),
                'sha256': hashlib.sha256((directory / name).read_bytes()).hexdigest(),
                'archive': f'evidence/evaluation-metadata/{variant}-{name}'}
                for name in ('results.json', 'before.json', 'after.json', 'manifest.json') if (directory / name).exists()},
            'source': str(source.relative_to(ROOT)), 'sourceSha256': hashlib.sha256(source.read_bytes()).hexdigest(),
            'currentAggregate': current, 'matchingReferenceAggregate': reference,
            'deltasVersusMatchingReference': difference(current, reference),
            'samePolicyC013SeedSubset': [s for s in seeds if s in CONSISTENT],
            'currentC013SeedSubsetAggregate': aggregate(cases, CONSISTENT),
            'deltasVersusC013SeedSubset': difference(aggregate(cases, CONSISTENT), aggregate(baseline['cases'], [s for s in seeds if s in CONSISTENT])),
            'perSeedAllTeams': rows,
            'communication': communication(directory / 'observation-end.json')}
    common = sorted(set(int(c['seed']) for c in full_cases.get('C014', [])) & set(int(c['seed']) for c in full_cases.get('C015', [])))
    report['newCandidatePair'] = {'commonCompletedSeeds': common,
        'C014minusC015': difference(aggregate(full_cases.get('C014', []), common), aggregate(full_cases.get('C015', []), common))}
    report['newEvaluationChecks'] = {
        'acJudgements': sum(v.get('acJudgements', 0) for v in report['variants'].values()),
        'totalJudgements': sum(v.get('totalJudgements', 0) for v in report['variants'].values()),
        'expectedJudgements': 32,
        'all32ACAndBothVariantsUnchanged': all(v.get('isCompleteVerified') for v in report['variants'].values()),
        'rawRunAndArtifactHashes': 'evidence-index.json'}
    report['recommendation'] = 'Pending root judgement; partial results must not be treated as a final selection.'
    target = DIRECTORY / 'selection-analysis.json'
    body = json.dumps(report, ensure_ascii=False, indent=2) + '\n'
    target.write_text(body)
    (DIRECTORY / 'comparison.json').write_text(body)
    print(json.dumps({'saved': str(target), 'completed': {v: x['completedSeeds'] for v, x in report['variants'].items()}}, ensure_ascii=False))


if __name__ == '__main__':
    main()
