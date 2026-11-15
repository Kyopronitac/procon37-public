#!/usr/bin/env python3
"""Archive completed run summaries only; keep large replays in server history."""
from pathlib import Path
import datetime
import hashlib
import json

DIRECTORY = Path(__file__).resolve().parent
ROOT = Path(__file__).resolve().parents[5]
SEEDS = (0, 3, 6, 9)


def read(path):
    return json.loads(path.read_text())


def sha(path):
    result = hashlib.sha256()
    with path.open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            result.update(chunk)
    return result.hexdigest()


def main():
    baseline = read(DIRECTORY / 'baseline.json')
    previous = read(ROOT / '.server/distributed/evaluations/C013-submit55-dedup-regression-01/results.json')
    previous_ids = {int(row['seed']): row['runId'] for row in previous['runs']}
    references = [('reference', 'C012', 0, '1791561513817407923-45859')]
    references += [('reference', 'C013', seed, previous_ids[seed]) for seed in (3, 6, 9)]
    for variant in ('C014', 'C015'):
        directory = ROOT / '.server/distributed/evaluations' / f'frequency16-{variant}-official-01'
        source = directory / 'results.json'
        if not source.exists():
            source = directory / 'progress.json'
        if source.exists():
            references += [('new-candidate', variant, int(row['seed']), row['runId']) for row in read(source).get('runs', [])]
    destination = DIRECTORY / 'evidence/raw-runs'
    destination.mkdir(parents=True, exist_ok=True)
    index = []
    for kind, version, seed, run_id in references:
        source = ROOT / '.server/runs' / run_id / 'run.json'
        data = source.read_bytes()
        run = json.loads(data)
        assert run['status'] == 'completed' and run['id'] == run_id
        case = next(c for c in run['cases'] if int(c['seed']) == seed)
        if kind == 'reference':
            reference = next(c for c in baseline['cases'] if int(c['seed']) == seed)
            assert case['players'] == reference['players']
        target = destination / f'{kind}-{version}-seed{seed}-{run_id}.json'
        if target.exists():
            assert target.read_bytes() == data, 'Existing archive changed'
        else:
            target.write_bytes(data)
        case_directory = source.parent / str(seed)
        files = {}
        for filename in ('problem.json', 'config.json', 'replay.json'):
            path = case_directory / filename
            if path.exists():
                files[filename] = {'sourcePath': str(path.relative_to(ROOT)), 'bytes': path.stat().st_size,
                                   'sha256': sha(path), 'copied': False}
        index.append({'kind': kind, 'version': version, 'seed': seed, 'runId': run_id,
                      'sourceRun': str(source.relative_to(ROOT)), 'archivedRun': str(target.relative_to(ROOT)),
                      'runSha256': hashlib.sha256(data).hexdigest(), 'runBytes': len(data),
                      'runOverallVerdict': run['verdict'], 'selectedCaseVerdict': case['status'],
                      'selectedCaseAll4AC': all(p['status'] == 'AC' for p in case['players']), 'caseFiles': files})
    metadata = []
    for variant in ('C014', 'C015'):
        directory = ROOT / '.server/distributed/evaluations' / f'frequency16-{variant}-official-01'
        if not (directory / 'results.json').exists():
            continue
        for filename in ('results.json', 'before.json', 'after.json', 'manifest.json'):
            path = directory / filename
            data = path.read_bytes()
            target = DIRECTORY / 'evidence/evaluation-metadata' / f'{variant}-{filename}'
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.exists():
                assert target.read_bytes() == data, 'Existing evaluation metadata archive changed'
            else:
                target.write_bytes(data)
            metadata.append({'version': variant, 'sourcePath': str(path.relative_to(ROOT)),
                             'archivedPath': str(target.relative_to(ROOT)), 'bytes': len(data),
                             'sha256': hashlib.sha256(data).hexdigest()})
    out = {'createdAt': datetime.datetime.now().astimezone().isoformat(),
           'scope': 'User-specified16x16 four-seed comparison; not formal12',
           'referenceCount': sum(x['kind'] == 'reference' for x in index),
           'newCandidateCount': sum(x['kind'] == 'new-candidate' for x in index),
           'largeReplaysCopied': False,
           'notes': ['The seed0 reference is the AC case of a C012 full12 run that had WA in other cases.',
                     'Historical reference mixes C012 seed0 and C013 seed3/6/9; no new baseline run.'],
           'records': index, 'completedEvaluationMetadata': metadata}
    (DIRECTORY / 'evidence-index.json').write_text(json.dumps(out, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({'referenceCount': out['referenceCount'], 'newCandidateCount': out['newCandidateCount'],
                      'index': str(DIRECTORY / 'evidence-index.json')}, ensure_ascii=False))


if __name__ == '__main__':
    main()
