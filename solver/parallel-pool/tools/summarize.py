#!/usr/bin/env python3
"""server正式12件を全6チームの保存順位で検証・集計する。"""
import argparse
import json
from pathlib import Path

METRICS = ('uniqueBrands', 'dailyBrands', 'udon')
IDS = ('P0', 'P1', 'P2', 'P3', 'S0', 'S1')

def summarize(path):
    run = json.loads(Path(path).read_text())
    config = run['config']
    assert run['status'] == 'completed', '未完了'
    assert run['total'] == run['completed'] == len(run['cases']) == 12, '12件が未完了'
    assert config['players'] == 6 and config['count'] == 12 and config.get('seed') is None
    assert config.get('format') is None and config.get('timeLimitMs') is None, '短縮/形式固定'
    assert config.get('arenaMaps') is True, '練習場設定相違'
    assert sorted(int(c['seed']) for c in run['cases']) == list(range(12)), 'seed欠落/重複'
    rows = [dict(id=name, team=i, rankSum=0, scores={m: 0 for m in METRICS}) for i, name in enumerate(IDS)]
    for case in run['cases']:
        assert case['status'] == 'AC', f"seed {case['seed']} 非AC"
        players = case['players']
        assert len(players) == 6 and sorted(p['id'] for p in players) == list(range(6))
        for player in players:
            assert player['status'] == 'AC', f"seed {case['seed']} team {player['id']} 非AC"
            assert 1 <= player['rank'] <= 6
            row = rows[player['id']]
            row['rankSum'] += player['rank']
            for m in METRICS:
                row['scores'][m] += player['score'][m]
    for row in rows:
        row['meanRank'] = row['rankSum'] / 12
    lowest = max(range(4), key=lambda i: (rows[i]['rankSum'], -i))
    strict = [r['id'] for r in rows if all(r['rankSum'] < x['rankSum'] for x in rows if x is not r)]
    return dict(runId=run['id'], players=rows, lowestParallel=IDS[lowest], strictWinner=strict), run

def compare(before, after, team):
    a = {int(c['seed']): c for c in before['cases']}
    b = {int(c['seed']): c for c in after['cases']}
    outcome = {'win': 0, 'tie': 0, 'loss': 0}
    for seed in range(12):
        old = next(p for p in a[seed]['players'] if p['id'] == team)['score']
        new = next(p for p in b[seed]['players'] if p['id'] == team)['score']
        x, y = tuple(new[m] for m in METRICS), tuple(old[m] for m in METRICS)
        outcome['win' if x > y else 'loss' if x < y else 'tie'] += 1
    return outcome

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('run_json')
    parser.add_argument('--before')
    parser.add_argument('--team', type=int, choices=range(4))
    args = parser.parse_args()
    report, run = summarize(args.run_json)
    if args.before:
        assert args.team is not None
        _, before = summarize(args.before)
        report['comparison'] = compare(before, run, args.team)
        report['comparisonNote'] = '他5体と同一条件での対象枠更新。対戦相手の行動変化による道路混雑の差を含む。'
    print(json.dumps(report, ensure_ascii=False, indent=2))
