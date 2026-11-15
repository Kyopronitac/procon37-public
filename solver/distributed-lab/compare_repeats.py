#!/usr/bin/env python3
"""Compare verified, paired runs without mixing environments or test durations."""
import argparse
from collections import Counter
from pathlib import Path

import manage


def player(case, team=0):
    return next(p for p in case['players'] if p['id'] == team)


def score(case):
    return tuple(player(case)['score'][m] for m in manage.METRICS)


def verdict(a, b):
    return 'win' if a > b else 'loss' if a < b else 'tie'


def outcomes(pairs):
    counts = Counter(verdict(a, b) for a, b in pairs)
    return {k: counts[k] for k in ('win', 'tie', 'loss')}


def aggregate(cases):
    return {'cases': len(cases),
            'meanRank': sum(player(c)['rank'] for c in cases) / len(cases),
            'scoreTotals': {m: sum(player(c)['score'][m] for c in cases)
                            for m in manage.METRICS}}


def load_stage(stage):
    directory = manage.stage_directory(stage)
    result = manage.read(directory / 'results.json')
    manage.require(result['all48AC'] and result['verifiedUnchanged'], 'unverified run: ' + stage)
    process = manage.read(directory / 'process.json')
    run = manage.read(directory / 'run.json')
    checked = manage.summarize_run(run, process['shortMs'], process['label'])
    manage.require(checked['players'] == result['players'], 'aggregate mismatch: ' + stage)
    manifest = manage.read(directory / 'manifest.json')
    cases = {int(c['seed']): c for c in run['cases']}
    problem_hashes = {s: manage.sha(manage.ROOT / '.server/runs' / run['id'] / str(s) / 'problem.json')
                      for s in cases}
    return {'stage': stage, 'runId': run['id'], 'result': result, 'cases': cases,
            'manifest': manifest, 'shortMs': process['shortMs'], 'problemHashes': problem_hashes,
            'records': {name: manage.sha(directory / name)
                        for name in ('manifest.json', 'run.json', 'results.json')}}


def compare(baseline_names, candidate_names):
    manage.require(len(baseline_names) == len(candidate_names) > 0, 'equal positive pair count required')
    manage.require(len(set(baseline_names + candidate_names)) == len(baseline_names + candidate_names),
                   'a run must not be reused in more than one pair')
    baseline = [load_stage(s) for s in baseline_names]
    candidate = [load_stage(s) for s in candidate_names]
    runs = baseline + candidate
    first = baseline[0]
    for run in runs:
        manage.require(manage.comparison_conditions(run['manifest']) ==
                       manage.comparison_conditions(first['manifest']), 'comparison conditions differ')
        manage.require(run['shortMs'] == first['shortMs'], 'durations differ')
        manage.require(run['problemHashes'] == first['problemHashes'], 'actual problem files differ')
    for cohort in (baseline, candidate):
        manage.require(all(r['manifest'] == cohort[0]['manifest'] for r in cohort),
                       'implementation or configuration changed within a cohort')
    pairs = []
    for a, b in zip(baseline, candidate):
        x, y = a['result']['players'][0], b['result']['players'][0]
        pairs.append({'baseline': a['stage'], 'candidate': b['stage'],
                      'rankSumDelta': y['rankSum'] - x['rankSum'],
                      'scoreDelta': {m: y['scores'][m] - x['scores'][m] for m in manage.METRICS},
                      'outcomes': outcomes((score(b['cases'][s]), score(a['cases'][s])) for s in range(12))})
    cohorts = {}
    for name, cohort in (('baseline', baseline), ('candidate', candidate)):
        cases = [c for r in cohort for c in r['cases'].values()]
        cohorts[name] = aggregate(cases)
        cohorts[name]['meanScoreTotalsPerRun'] = {m: v / len(cohort)
                                                  for m, v in cohorts[name]['scoreTotals'].items()}
        cohorts[name]['formats'] = {f: aggregate([c for c in cases if c['format'] == f])
                                    for f in ('first', 'semifinal', 'final')}
    seeds = []
    for seed in range(12):
        sums = [tuple(sum(score(r['cases'][seed])[i] for r in cohort)
                      for i in range(len(manage.METRICS))) for cohort in (baseline, candidate)]
        seeds.append({'seed': seed, 'format': first['cases'][seed]['format'],
                      'baselineScoreSum': sums[0], 'candidateScoreSum': sums[1],
                      'meanScoreDelta': [(b - a) / len(baseline) for a, b in zip(*sums)],
                      'outcome': verdict(sums[1], sums[0])})
    return {'baselineStages': baseline_names, 'candidateStages': candidate_names,
            'environment': first['manifest'].get('environmentIdentity'),
            'shortMs': first['shortMs'], 'pairCount': len(pairs), 'pairs': pairs, 'cohorts': cohorts,
            'seedMeanOutcomes': outcomes((tuple(s['candidateScoreSum']), tuple(s['baselineScoreSum']))
                                         for s in seeds), 'seeds': seeds,
            'inputs': [{'stage': r['stage'], 'runId': r['runId'], 'recordSha256': r['records'],
                        'problemSha256': r['problemHashes']} for r in runs],
            'scriptSha256': manage.sha(Path(__file__)),
            'note': 'Paired score comparisons use uniqueBrands, dailyBrands, udon lexicographically. '
                    'Scores are averaged within each seed before the seed-level comparison. '
                    'Small timed runs do not establish statistical significance.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', nargs='+', required=True)
    parser.add_argument('--candidate', nargs='+', required=True)
    parser.add_argument('--save', required=True)
    args = parser.parse_args()
    destination = Path(args.save).resolve()
    manage.require(destination.is_relative_to(manage.ROOT) and not destination.exists(), 'invalid or existing output')
    result = compare(args.baseline, args.candidate)
    manage.write(destination, result)
    print({'record': str(destination), 'pairCount': result['pairCount'],
           'baselineRank': result['cohorts']['baseline']['meanRank'],
           'candidateRank': result['cohorts']['candidate']['meanRank'],
           'seedMeanOutcomes': result['seedMeanOutcomes']})


if __name__ == '__main__':
    main()
