#!/usr/bin/env python3
"""Render only already-saved analysis; never touch evaluation inputs."""
from pathlib import Path
import json

DIRECTORY = Path(__file__).resolve().parent
TEAMS = ('分散', '固定並列', '固定第6世代', '固定第8世代')


def scores(value):
    return '/'.join(str(value[key]) for key in ('uniqueBrands', 'dailyBrands', 'udon'))


def table(rows):
    text = ['| 実行版・対象 | チーム | 件数 | 平均順位 | 種類数/日別種類数/玉数の合計 |',
            '|---|---|---:|---:|---|']
    for label, aggregate in rows:
        if aggregate:
            for team in aggregate:
                text.append(f"| {label} | {TEAMS[team['team']]} | {team['cases']} | {team['meanRank']:.3f} | {scores(team['scores'])} |")
    return text


def main():
    report = json.loads((DIRECTORY / 'selection-analysis.json').read_text())
    baseline = json.loads((DIRECTORY / 'baseline.json').read_text())
    all_verified = all(value.get('isCompleteVerified') for value in report['variants'].values())
    output = [
        '# 16×16・指定4 seed の共有周期比較', '',
        f"集計時刻: {report['createdAt']}", '',
        '状態: ' + ('新2案の4件ずつが完了し、全員AC・前後識別一致を確認済み。最終選定と配置は担当者の判断を待つ。' if all_verified else '実行途中。未完了の部分集計は最終採用の根拠として扱わない。'), '',
        '対象は seed 0,3,6,9 の16×16・1回戦マップ4件。公式の制限時間を使用するが、正式12件評価ではない。新規実行は C014（100 ms）と C015（2,000 ms）の2案。C013（500 ms）の参考成績は保存結果を再利用し、seed 0 だけは C012 の結果が混在する。', '',
        '## 4件全体と途中経過', '',
        '参照4件と未完了の候補を件数の異なるまま直接比較しない。候補の進捗は下記の件数に示す。', '',
    ]
    rows = [('保存参照（C012 seed0 + C013 seed3/6/9）', report['reference4'])]
    rows += [(name + ' seed' + ','.join(map(str, value['completedSeeds'])), value.get('currentAggregate')) for name, value in report['variants'].items()]
    output += table(rows)
    output += ['', '## C013 と提出方針が共通する3件', '',
               'seed 3,6,9 のみ。短い時間差の探索ばらつき、他チームとの相互作用は残る。', '']
    rows = [('保存C013 seed3/6/9', report['referenceC013Only3'])]
    rows += [(name + ' seed' + ','.join(map(str, value.get('samePolicyC013SeedSubset', []))), value.get('currentC013SeedSubsetAggregate')) for name, value in report['variants'].items()]
    output += table(rows)
    output += ['', '## 各ケースの4チーム値', '',
               '表の種類数/日別種類数/玉数は公式の生値。各チームの順位はその試合の対戦相手との相互作用を含む。', '',
               '| 版 | seed | チーム | 順位 | 種類数/日別種類数/玉数 |', '|---|---:|---|---:|---|']
    for case in baseline['cases']:
        for player in case['players']:
            output.append(f"| 保存{case['version']} | {case['seed']} | {TEAMS[player['id']]} | {player['rank']} | {scores(player['score'])} |")
    for name, value in report['variants'].items():
        for row in value.get('perSeedAllTeams', []):
            output.append(f"| {name} | {row['seed']} | {TEAMS[row['team']]} | {row['rank']} | {scores(row['score'])} |")
    output += ['', '## 通信・探索の成立確認', '',
               '| 版 | 要求数 | v1の31/31・remote2/2 | v2の15/15・入力一致 | v3の31/31・入力一致 | 取り込み/内部改善 | 送信パケット数 |',
               '|---|---:|---|---|---|---|---:|']
    for name, value in report['variants'].items():
        comm = value.get('communication', {})
        if comm.get('status') != 'observed':
            output.append(f'| {name} | 未集計 | — | — | — | — | — |')
            continue
        hosts = comm['hosts']
        checks = [comm['allV1LocalAndRemoteComplete']]
        checks += [hosts[host]['allExpectedThreadsCompleted'] and hosts[host]['inputSequenceMatchesV1'] and hosts[host]['completionRequestOrderMatches'] for host in ('v2', 'v3')]
        counter = {key: sum(hosts[host]['counters'][key] for host in hosts) for key in ('imports', 'improved', 'sent')}
        output.append(f"| {name} | {comm['requestCount']} | {checks[0]} | {checks[1]} | {checks[2]} | {counter['imports']}/{counter['improved']} | {counter['sent']} |")
    output += ['',
        '通信ログは当該期間の resume-team.log のみを使用し、古い stderr.log は使用しない。内部取り込み・改善は最終提出内容への寄与や対戦成績の改善を証明しない。共有周期定数はローカル公開・ネット送出・残時間ガードに共通し、受信確認250 msと取り込みクールダウン1秒は全案で同じ。', '',
        '## AC判定と前後識別の根拠', '',
        f"新規評価の完了判定: {report['newEvaluationChecks']['acJudgements']}/{report['newEvaluationChecks']['totalJudgements']} AC（全完了時の予定判定数32）。両案の4件完了・全員AC・前後識別一致: {report['newEvaluationChecks']['all32ACAndBothVariantsUnchanged']}。", '',
    ]
    for name, value in report['variants'].items():
        if value.get('isCompleteVerified'):
            links = ', '.join(f"[{filename}]({evidence['archive']})" for filename, evidence in value['completionEvidence'].items())
            output.append(f'- {name}: {links}。results の all16AC / verifiedUnchanged と before/after の凍結対象識別を記録。')
    output += ['',
        '## 判断上の制約と証跡', '',
        '- 優先指標は公式順位の平均。種類数、日別種類数、玉数は独立して併記し、独自の辞書順勝敗や合成点は作らない。',
        '- 4件を1回ずつ試した小標本であり、小差から一般的な優位や再現性を断定しない。',
        '- 参考C013の seed3/6/9 は日別種類数が理論上限（130/88/60）に達している。玉1個の差でも順位が動くため、平均順位だけで差の大きさを誇張しない。',
        '- 固定相手のバイナリ・設定を固定していても時間制探索と対戦相互作用で結果は変動する。4チームすべての値を併記し、単純な補正値は設けない。',
        '- 参照seed0の元C012 run全体には別ケースのWAが含まれるが、この比較で使うseed0は全4AC。選択したケースと元run全体の判定を区別する。',
        '- 原本 run.json は evidence/raw-runs/ に保存。problem/config/replay はコピーせず識別用SHAを evidence-index.json に記録。',
        '- 詳細な機械可読値は [comparison.json](comparison.json)、原本とハッシュ一覧は [evidence-index.json](evidence-index.json)、判断方針は [selection-method.md](selection-method.md) を参照。最終選定は root 担当。', '',
    ]
    target = DIRECTORY / 'comparison.md'
    target.write_text('\n'.join(output))
    print(target)


if __name__ == '__main__':
    main()
