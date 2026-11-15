#!/usr/bin/env python3
"""同一条件の4チーム12件を検証し、順位・得点・前後勝敗を保存する。"""
import argparse
import collections
import json
from pathlib import Path

METRICS = ('uniqueBrands', 'dailyBrands', 'udon')
NAMES = ('distributed', 'parallel-fixed', 'generation6', 'generation8')

def summarize(path, short_ms=None):
    run = json.loads(Path(path).read_text())
    cfg = run['config']
    assert run['status'] == 'completed' and run['verdict'] == 'AC', '未完了または非AC'
    assert run['completed'] == run['total'] == len(run['cases']) == 12
    assert cfg['players'] == 4 and cfg['count'] == 12 and cfg.get('seed') is None
    assert cfg.get('format') is None and cfg['arenaMaps'] is True
    assert cfg.get('timeLimitMs') == short_ms, '制限時間相違'
    assert sorted(int(c['seed']) for c in run['cases']) == list(range(12))
    assert collections.Counter(c['format'] for c in run['cases']) == {'first': 4, 'semifinal': 4, 'final': 4}
    rows = [dict(team=i, name=name, rankSum=0, scores={m: 0 for m in METRICS}) for i, name in enumerate(NAMES)]
    for case in run['cases']:
        assert case['status'] == 'AC'
        assert len(case['players']) == 4 and sorted(p['id'] for p in case['players']) == list(range(4))
        for p in case['players']:
            assert p['status'] == 'AC' and 1 <= p['rank'] <= 4
            row = rows[p['id']]
            row['rankSum'] += p['rank']
            for m in METRICS:
                row['scores'][m] += p['score'][m]
    for row in rows:
        row['meanRank'] = row['rankSum'] / 12
    return {'runId': run['id'], 'config': cfg, 'all48AC': True, 'players': rows}, run

def compare(before, after):
    for key in ('count', 'seed', 'format', 'arenaMaps', 'players', 'timeLimitMs', 'serverProgram'):
        assert before['config'].get(key) == after['config'].get(key), f'条件相違: {key}'
    a = {int(c['seed']): c for c in before['cases']}
    result = {'win': 0, 'tie': 0, 'loss': 0}
    for c in after['cases']:
        old = next(p for p in a[int(c['seed'])]['players'] if p['id'] == 0)['score']
        new = next(p for p in c['players'] if p['id'] == 0)['score']
        x, y = tuple(new[m] for m in METRICS), tuple(old[m] for m in METRICS)
        result['win' if x > y else 'loss' if x < y else 'tie'] += 1
    return result

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('run_json')
    parser.add_argument('--short-ms', type=int)
    parser.add_argument('--before')
    parser.add_argument('--save')
    args = parser.parse_args()
    report, run = summarize(args.run_json, args.short_ms)
    if args.before:
        previous, before = summarize(args.before, args.short_ms)
        report.update(comparison=compare(before, run), before=previous['players'][0],
                      comparisonNote='同じ4チーム条件。相手行動による交通混雑と時間制探索のばらつきを含む。')
    value = json.dumps(report, ensure_ascii=False, indent=2) + '\n'
    if args.save:
        Path(args.save).write_text(value)
    print(value, end='')
