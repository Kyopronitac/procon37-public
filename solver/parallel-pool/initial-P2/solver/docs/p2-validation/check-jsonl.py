#!/usr/bin/env python3
"""Small JSON Lines/thread-count smoke check; no opposing solver is inspected."""
import json
import pathlib
import selectors
import subprocess
import time

ROOT = pathlib.Path(__file__).resolve().parents[3]
REPORT = pathlib.Path(__file__).with_name('jsonl-result.json')
LOG = pathlib.Path(__file__).with_name('jsonl-stderr.log')
problem = {
    'startsAt': 0, 'daySeconds': [60] * 4, 'daySteps': [40] * 4,
    'map': {'height': 5, 'width': 7, 'cells': [[0] * 7 for _ in range(5)]},
    'spots': [{'brand': i // 2, 'pos': pos, 'stocks': 2} for i, pos in enumerate([2, 5, 8, 15, 21, 27, 30, 33])],
    'agents': [0, 6, 28, 34], 'fuelLimits': 16, 'players': 6,
    'busyThreshold': 2, 'jammedThreshold': 5,
}
records = []
max_threads = 0
with LOG.open('w') as stderr:
    process = subprocess.Popen([str(ROOT / 'target/release/solver')], cwd=ROOT, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr, text=True, bufsize=1)
    poll = selectors.DefaultSelector()
    poll.register(process.stdout, selectors.EVENT_READ)
    def exchange(request):
        global max_threads
        began = time.monotonic()
        process.stdin.write(json.dumps(request, separators=(',', ':')) + '\n')
        process.stdin.flush()
        sample_max = 0
        while True:
            threads = len(list(pathlib.Path(f'/proc/{process.pid}/task').iterdir()))
            sample_max = max(sample_max, threads)
            max_threads = max(max_threads, threads)
            assert threads <= 15, threads
            if poll.select(0.003):
                line = process.stdout.readline()
                assert line, 'unexpected EOF'
                response = json.loads(line)
                break
            assert time.monotonic() - began < 10, 'response timeout'
        elapsed = (time.monotonic() - began) * 1000
        assert response['requestId'] == request['requestId'], response
        assert isinstance(response['answer'], list)
        records.append({'requestId': request['requestId'], 'phase': request['phase'], 'timeLimitMs': request['timeLimitMs'], 'elapsedMs': round(elapsed, 3), 'maxThreads': sample_max})
        return response['answer']
    def request(number, phase, budget, **extra):
        return {'protocolVersion': 1, 'requestId': number, 'phase': phase, 'timeLimitMs': budget, **extra}
    kinds = exchange(request(0, 'select', 1600, problem=problem))
    assert len(kinds) == 4 and kinds.count(1) == 1
    state = {'endsAt': 0, 'day': 0, 'agents': [{'kind': k, 'pos': pos, 'fuel': 16} for k, pos in zip(kinds, problem['agents'])], 'others': [], 'traffics': []}
    plan = exchange(request(1, 'plan', 1600, state=state, steps=40, previousAccepted=True))
    assert len(plan) == 4 and all(all(isinstance(x, int) for x in car) for car in plan)
    # Simulate rejection: the real team waited, so its positions/fuel are unchanged.
    state['day'] = 1
    plan = exchange(request(2, 'plan', 1, state=state, steps=40, previousAccepted=False))
    assert plan == [[-40]] * 4
    state['day'] = 2
    plan = exchange(request(3, 'plan', 1600, state=state, steps=40, previousAccepted=True))
    assert len(plan) == 4
    # A new match reuses the process without overlapping worker pools.
    kinds = exchange(request(4, 'select', 1600, problem=problem))
    assert len(kinds) == 4 and kinds.count(1) == 1
    process.stdin.close()
    assert process.wait(timeout=10) == 0
assert max_threads == 15, max_threads
result = {'passed': True, 'maxSolverThreads': max_threads, 'requests': records}
REPORT.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(result, ensure_ascii=False))
