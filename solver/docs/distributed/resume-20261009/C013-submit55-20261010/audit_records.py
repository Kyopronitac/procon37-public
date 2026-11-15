#!/usr/bin/env python3
"""完了した4件の実通信記録から、重複省略・受理・保存を読み取り監査する。"""
import concurrent.futures
import json
from pathlib import Path
import shlex
import subprocess
import sys

ROOT = next(p for p in Path(__file__).resolve().parents if (p / 'solver/distributed-lab/manage.py').is_file())
sys.path.insert(0, str(ROOT / 'solver/distributed-lab'))
import lifecycle
import manage

STAGE = 'C013-submit55-dedup-regression-01'
PROGRAM = r'''
import json,hashlib,collections
from pathlib import Path
spec=SPEC
rows=[]
for name in spec['directories']:
    root=Path(spec['root'])/name
    data={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in root.iterdir() if p.is_file()}
    meta=json.loads((root/'metadata.json').read_text())
    problem=json.loads((root/'problem.json').read_text())
    saved_map=json.loads((root/'map.json').read_text())
    http=[json.loads(line) for line in (root/'http.jsonl').read_text().splitlines()]
    observations=[json.loads(line)['state'] for line in (root/'observations.jsonl').read_text().splitlines()]
    setting=next(e['body'] for e in http if e['event']=='response' and e['method']=='GET' and e['path']=='/setting')
    assert problem==setting and saved_map==setting['map']
    assert meta['matchId'] in spec['matches'] and meta['selectionAnchorSource']=='server_header'
    days=len(problem['daySeconds']);assert all(s==60 for s in problem['daySeconds'])
    assert len(observations)==days and [s['day'] for s in observations]==list(range(days))
    events=[e for e in http if e['event'] in ('scheduledSubmission','scheduledSubmissionSkipped')]
    phases=collections.defaultdict(list)
    for e in events:phases[(e['phase'],e['requestId'])].append(e)
    assert set(phases)=={('select',0)}|{('plan',d+1) for d in range(days)}
    expected=[1,2,3,4,5,6] if spec['distributed'] else [1]
    submits=skips=0
    for (phase,rid),items in phases.items():
        assert [e['checkpoint'] for e in items]==expected
        start=meta['selectionStartMs'] if phase=='select' else (observations[rid-1]['endsAt']-60)*1000
        previous=None
        for e in items:
            offset=(5000+(e['checkpoint']-1)*10000) if spec['distributed'] else 55000
            assert e['targetMs']==start+offset
            assert e['fromSolver'] is True
            if e['event']=='scheduledSubmissionSkipped':
                assert previous is not None and e['answer']==previous and e['reason']=='unchanged_answer'
                skips+=1
            else:
                assert previous is None or e['answer']!=previous
                assert e['outcome'] is True and e['httpAttempts']==1
                previous=e['answer'];submits+=1
        assert items[0]['event']=='scheduledSubmission'
    sent=[e for e in http if e['event']=='request' and e['method']=='POST']
    replies=[e for e in http if e['event']=='response' and e['method']=='POST']
    actual=[e for e in events if e['event']=='scheduledSubmission']
    assert len(sent)==len(replies)==len(actual)==submits
    errors=[]
    for req,rep,event in zip(sent,replies,actual):
        assert req['body']==event['answer'] and req['path']==rep['path']
        assert rep['httpStatus']==200
        if req['path']=='/':assert rep['body']['revision']>=0
        errors.append(req['attemptedAtMs']-event['targetMs'])
    assert min(errors)>=0 and max(errors)<1000
    rows.append({'matchId':meta['matchId'],'directory':str(root),'files':data,
                 'days':days,'phases':len(phases),'submitted':submits,'skipped':skips,
                 'allPostAccepted':True,'allAnswersFromSolver':True,'exactDedupVerified':True,
                 'allPhaseFirstSubmitted':True,'exactProblemAndMapSaved':True,
                 'observedAllDays':True,'minimumTargetDelayMs':min(errors),'maximumTargetDelayMs':max(errors)})
assert {r['matchId'] for r in rows}==set(spec['matches'])
print('__RESULT__'+json.dumps(rows))
'''


def main():
    config = manage.read(manage.CONFIG)
    observations = manage.read(manage.WORK / 'observations' / STAGE / 'end.json')
    results = manage.read(manage.stage_directory(STAGE) / 'results.json')
    matches = [r['runId'] + '-s' + str(r['seed']) for r in results['runs']]
    specifications = {}
    for host in ('v1', 'nomlab', 'local2', 'local3'):
        node = config['localTeams'][host] if host.startswith('local') else config['nodes'][host]
        if host.startswith('local'):
            lines = (Path(node['directory']) / 'resume-team.log').read_text().splitlines()
        else:
            lines = observations['nodes'][host]['logs']['resume-team.log']['diagnosticLines']
        directories = [l.split('directory=', 1)[1] for l in lines if l.startswith('[bridge] recording_started')][-4:]
        assert len(directories) == 4
        specifications[host] = {'root': node['directory'], 'directories': directories,
                                'matches': matches, 'distributed': host == 'v1'}

    def inspect(host):
        source = PROGRAM.replace('SPEC', repr(specifications[host]), 1)
        if host.startswith('local'):
            output = subprocess.run([sys.executable, '-c', source], capture_output=True, text=True, check=True).stdout
        else:
            output = lifecycle.execute(host, 'python3 -c ' + shlex.quote(source))
        return host, json.loads(output.split('__RESULT__')[-1])

    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        nodes = dict(pool.map(inspect, specifications))
    record = {'stage': STAGE, 'verified': True, 'nodes': nodes}
    manage.write(Path(__file__).with_name('submission-record-audit.json'), record)
    print(json.dumps({h: {'submitted': sum(r['submitted'] for r in rows),
                         'skipped': sum(r['skipped'] for r in rows),
                         'maxTargetDelayMs': max(r['maximumTargetDelayMs'] for r in rows)}
                      for h, rows in nodes.items()}))


if __name__ == '__main__':
    main()
