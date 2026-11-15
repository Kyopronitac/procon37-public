#!/usr/bin/env python3
"""Explicit historical comparison to saved D0, not a same-environment claim."""
import argparse
from collections import Counter
from pathlib import Path

import manage


def player(case):
    return next(p for p in case['players'] if p['id'] == 0)


def aggregate(cases):
    return {'cases': len(cases), 'meanRank': sum(player(c)['rank'] for c in cases) / len(cases),
            'scores': {m: sum(player(c)['score'][m] for c in cases) for m in manage.METRICS}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage')
    args = parser.parse_args()
    directory = manage.stage_directory(args.stage)
    result = manage.read(directory / 'results.json')
    manage.require(result['all48AC'] and result['verifiedUnchanged'], 'candidate not verified')
    manage.require(result['version'] == 'C012-isolated-cooperation-on', 'expected frozen C012 on')
    candidate = manage.read(directory / 'run.json')
    baseline_path = manage.ROOT / 'solver/docs/distributed/initial-formal-run.json'
    baseline = manage.read(baseline_path)
    manage.require(baseline['id'] == '1791414715585964511-304414', 'unexpected saved D0')
    for run in (baseline, candidate):
        checked = manage.summarize_run(run, None, run['config']['label'])
        if run is candidate:
            manage.require(checked['players'] == result['players'], 'candidate aggregate differs')
    old = {int(c['seed']): c for c in baseline['cases']}
    rows = []
    for case in candidate['cases']:
        previous = old[int(case['seed'])]
        manage.require(case['format'] == previous['format'], 'seed format differs')
        a, b = player(previous), player(case)
        score_a, score_b = (tuple(p['score'][m] for m in manage.METRICS) for p in (a, b))
        rows.append({'seed': int(case['seed']), 'format': case['format'],
                     'D0': a, 'C012': b,
                     'outcome': 'win' if score_b > score_a else 'loss' if score_b < score_a else 'tie',
                     'rankDelta': b['rank'] - a['rank'],
                     'scoreDelta': {m: b['score'][m] - a['score'][m] for m in manage.METRICS}})
    counts = Counter(r['outcome'] for r in rows)
    historic = manage.read(manage.ROOT / 'solver/docs/distributed/initial-formal-preflight.json')
    manifest = manage.read(directory / 'manifest.json')
    common_files = set(historic['localFiles']) & set(manifest['fixedLocalFiles'])
    identity = {p: {'D0': historic['localFiles'][p], 'current': manifest['fixedLocalFiles'][p],
                    'matches': historic['localFiles'][p] == manifest['fixedLocalFiles'][p]}
                for p in sorted(common_files)}
    report = {'comparisonKind': 'user-requested-historical-formal', 'sameEnvironmentClaim': False,
              'baselineRunId': baseline['id'], 'candidateRunId': candidate['id'],
              'baselineSha256': manage.sha(baseline_path), 'candidateSha256': manage.sha(directory / 'run.json'),
              'all48ACBoth': True, 'D0': aggregate(baseline['cases']), 'C012': aggregate(candidate['cases']),
              'scoreOutcomes': {k: counts[k] for k in ('win', 'tie', 'loss')}, 'cases': rows,
              'formats': {f: {name: aggregate([c for c in run['cases'] if c['format'] == f])
                               for name, run in (('D0', baseline), ('C012', candidate))}
                          for f in ('first', 'semifinal', 'final')},
              'commonLocalFileIdentity': identity,
              'limitations': ['User explicitly requested saved D0 comparison and no second C012 short run.',
                              'Historical raw problem files are not available locally; seed/format/settings match is verified.',
                              'Server and local fixed opponent executable hashes differ after restoration; this is not an isolated causal estimate.',
                              'This report alone neither adopts C012 nor claims statistical significance.'],
              'scriptSha256': manage.sha(Path(__file__))}
    destination = manage.ROOT / 'solver/docs/distributed/resume-20261009/evaluations' / args.stage / 'saved-D0-comparison.json'
    manage.require(not destination.exists(), 'comparison already recorded')
    manage.write(destination, report)
    print({'record': str(destination), 'D0': report['D0'], 'C012': report['C012'],
           'scoreOutcomes': report['scoreOutcomes'], 'sameEnvironmentClaim': False})


if __name__ == '__main__':
    main()
