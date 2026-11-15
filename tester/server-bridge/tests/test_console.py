"""run.sh logging and shutdown checks; no SSH or real match server required.

python3 -m unittest discover -s tester/server-bridge/tests -p test_console.py -v
BRIDGE_TEST_BINARY=target/debug/server-bridge ... enables the HTTP/map integration check.
"""
import http.server
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'tester/server-bridge'
spec = importlib.util.spec_from_file_location('bridge_console', BASE / 'console.py')
console = importlib.util.module_from_spec(spec)
spec.loader.exec_module(console)


class ConsoleTests(unittest.TestCase):
    def test_only_requested_progress_is_displayed(self):
        progress = console.Progress()
        self.assertIsNone(progress.line(b'[solver] lengthy search stats\n'))
        self.assertIsNone(progress.line(b'[bridge] schedule day_steps=[1, 2]\n'))
        self.assertIsNone(progress.line(b'[bridge] recording_failed directory=x\n'))
        for event in [b'v2: wired TCP authentication OK (15 search threads)',
                      b'[bridge] connection_check server=http://x path=/setting',
                      b'[bridge] server_connection result=reachable http_status=403',
                      b'[bridge] problem_received width=16 height=16 days=7',
                      b'[bridge] solver_started', b'[bridge] day_state day=2']:
            self.assertIsNotNone(progress.line(event))
        progress.line(b'[bridge] solve_start phase=plan day=2 limit_ms=54000')
        rendered = progress.line(b'[bridge] checkpoint_submit phase=plan request_id=2 checkpoint=6 elapsed_ms=55007')
        self.assertIn('elapsed_sec=55.007', rendered)
        self.assertIn('day=2', rendered)
        self.assertIsNone(progress.line(b'[bridge] submission_attempt path=/ attempt=1 remaining_ms=4900'))
        self.assertIn('result=accepted', progress.line(b'[bridge] checkpoint_result phase=plan checkpoint=6 result=accepted'))
        self.assertIsNone(progress.line(b'[bridge] plan_submission result=accepted scheduled=true'))
        self.assertEqual(progress.line(b'[bridge] solver_exited'), '[run.sh] solver_exited')

    def test_distributed_runtime_access_events_exclude_search_statistics(self):
        progress = console.Progress()
        self.assertEqual(progress.line(b'dist peer=192.168.0.22:39370 connection=ready version=4 threads=16 offset=31'),
                         '[run.sh] distributed_node_access node=192.168.0.22:39370 result=ok')
        for state in ('unavailable', 'disconnected'):
            self.assertEqual(progress.line(f'dist peer=192.168.0.33:39370 request=17 {state}=connection reset by peer'.encode()),
                             f'[run.sh] distributed_node_access node=192.168.0.33:39370 result={state}')
        self.assertEqual(progress.line(b'dist peer=192.168.0.22:39370 rejected_configuration version=3 threads=16 offset=31'),
                         '[run.sh] distributed_node_access node=192.168.0.22:39370 result=configuration_rejected')
        for raw in (b'dist peer=192.168.0.22:39370 request=17 completed=15/15 elapsed_ms=123 rx_bytes=300 tx_bytes=500',
                    b'dist peer=192.168.0.22:39370 ignored_stale_or_mismatched_result request=17',
                    b'dist peer=192.168.0.22:39370 error=unrelated solver diagnostic'):
            self.assertIsNone(progress.line(raw))

    def test_raw_bytes_exit_code_and_no_diagnostic_leak(self):
        with tempfile.TemporaryDirectory() as temp:
            logs = Path(temp) / 'logs with spaces'
            script = 'import os; os.write(1,b"[bridge] solver_started\\n"); os.write(2,b"private diagnostic \\xff\\n"); raise SystemExit(2)'
            result = subprocess.run([sys.executable, str(BASE / 'console.py'), '--log-dir', str(logs),
                                     '--repeat-bridge', '--', sys.executable, '-c', script],
                                    capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 2)
            self.assertIn(b'[run.sh] solver_started', result.stdout)
            self.assertNotIn(b'private diagnostic', result.stdout + result.stderr)
            saved = list(logs.iterdir())
            self.assertEqual(len(saved), 1)
            self.assertIn(b'private diagnostic \xff\n', saved[0].read_bytes())
            self.assertEqual(saved[0].stat().st_mode & 0o777, 0o600)

    def test_run_sh_passes_default_and_streaming_schedule(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            bridge = root / 'bridge.py'
            arguments = root / 'arguments.json'
            bridge.write_text('#!' + sys.executable + '\nimport json, sys\nfrom pathlib import Path\n'
                              f'Path({str(arguments)!r}).write_text(json.dumps(sys.argv[1:]))\n'
                              'raise SystemExit(2)\n')
            bridge.chmod(0o700)
            environment = os.environ | {'BRIDGE': str(bridge), 'SOLVER': str(bridge),
                                       'BRIDGE_LOG_DIR': str(root / 'logs')}
            environment.pop('SUBMIT_AT_MS', None)
            environment.pop('STREAM_RESPONSES', None)
            result = subprocess.run(['sh', str(BASE / 'run.sh')], env=environment,
                                    capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 2)
            args = json.loads(arguments.read_text())
            self.assertEqual(args[args.index('--submit-at-ms') + 1], '55000')
            self.assertNotIn('--stream-responses', args)
            self.assertEqual(args[args.index('--record-dir') + 1], str(ROOT / '.server/submissions'))
            result = subprocess.run(['sh', str(BASE / 'run.sh')],
                                    env=environment | {'SUBMIT_AT_MS': '5000,15000,25000,35000,45000,55000',
                                                       'STREAM_RESPONSES': '1'},
                                    capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 2)
            args = json.loads(arguments.read_text())
            self.assertEqual(args[args.index('--submit-at-ms') + 1], '5000,15000,25000,35000,45000,55000')
            self.assertIn('--stream-responses', args)

    def test_sigterm_reaches_child_and_does_not_restart(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            child = root / 'child.py'
            child.write_text('import signal, time\nfrom pathlib import Path\n'
                             f'Path({str(root / "ready")!r}).write_text("ready")\n'
                             'def stop(sig, frame):\n'
                             f'    Path({str(root / "stopped")!r}).write_text("stopped")\n'
                             '    raise SystemExit(0)\n'
                             'signal.signal(signal.SIGTERM, stop)\n'
                             'while True: time.sleep(.02)\n')
            runner = subprocess.Popen([sys.executable, str(BASE / 'console.py'), '--log-dir', str(root / 'logs'),
                                       '--repeat-bridge', '--', sys.executable, str(child)],
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                for _ in range(100):
                    if (root / 'ready').exists():
                        break
                    time.sleep(.02)
                self.assertTrue((root / 'ready').exists())
                runner.send_signal(signal.SIGTERM)
                runner.communicate(timeout=5)
                self.assertEqual(runner.returncode, 143)
                self.assertTrue((root / 'stopped').exists())
            finally:
                if runner.poll() is None:
                    runner.kill()
                    runner.communicate(timeout=2)

    @unittest.skipUnless(os.environ.get('BRIDGE_TEST_BINARY'), 'set BRIDGE_TEST_BINARY for real HTTP/map check')
    def test_run_sh_saves_exact_problem_map_and_roads_before_submission(self):
        actual_bridge = Path(os.environ['BRIDGE_TEST_BINARY']).resolve(strict=True)
        problem = {'startsAt': 0, 'daySeconds': [5], 'daySteps': [1],
                   'map': {'width': 2, 'height': 2, 'cells': [[0, 1], [0, 0]]},
                   'spots': [{'brand': 0, 'pos': 3, 'stocks': 1}], 'agents': [0],
                   'fuelLimits': 10, 'players': 1, 'busyThreshold': 1, 'jammedThreshold': 2,
                   'extraServerField': {'preserve': True}}
        state = {'ended': False, 'observations': [], 'posted': []}
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            records = root / 'match records'
            logs = root / 'raw logs'

            class Handler(http.server.BaseHTTPRequestHandler):
                def log_message(self, *_args):
                    pass

                def respond(self, status, body):
                    data = json.dumps(body).encode()
                    self.send_response(status)
                    self.send_header('Content-Length', str(len(data)))
                    self.send_header('Procon37-Match-Id', 'run-sh-record-test')
                    self.send_header('Procon37-Select-Remaining-Ms', '3000')
                    self.end_headers()
                    self.wfile.write(data)

                def do_GET(self):
                    if self.path.split('?')[0] == '/setting':
                        self.respond(200, problem)
                    elif state['ended']:
                        self.respond(403, {'message': 'match has ended'})
                    else:
                        observation = {'endsAt': int(time.time()) + 5, 'day': 0,
                                       'agents': [{'kind': 0, 'pos': 0, 'fuel': 10}],
                                       'others': [], 'traffics': [{'pos': 1, 'status': 2}]}
                        state['observations'].append(observation)
                        self.respond(200, observation)

                def do_POST(self):
                    body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                    state['posted'].append(body)
                    directory = next(records.iterdir())
                    state['problem_before_post'] = json.loads((directory / 'problem.json').read_text()) == problem
                    if self.path.split('?')[0] == '/agent':
                        self.respond(200, {})
                    else:
                        state['roads_before_post'] = json.loads((directory / 'observations.jsonl').read_text().splitlines()[-1])['state']['traffics'] == [{'pos': 1, 'status': 2}]
                        state['ended'] = True
                        self.respond(200, {'revision': 0})

            solver = root / 'solver.py'
            solver.write_text('#!' + sys.executable + '\nimport json, sys\n'
                              'for line in sys.stdin:\n'
                              ' r=json.loads(line)\n'
                              ' print("private solver detail", file=sys.stderr, flush=True)\n'
                              ' print(json.dumps({"requestId":r["requestId"],"answer":[0] if r["phase"]=="select" else [[0]]}), flush=True)\n')
            solver.chmod(0o700)
            bridge = root / 'one-match-bridge.py'
            bridge.write_text('#!' + sys.executable + '\nimport subprocess, sys\nfrom pathlib import Path\n'
                              f'marker=Path({str(root / "ran")!r})\n'
                              'if marker.exists(): raise SystemExit(2)\nmarker.touch()\n'
                              f'raise SystemExit(subprocess.call([{str(actual_bridge)!r}]+sys.argv[1:]))\n')
            bridge.chmod(0o700)
            with http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler) as server:
                thread = threading.Thread(target=server.serve_forever, daemon=True)
                thread.start()
                try:
                    result = subprocess.run(['sh', str(BASE / 'run.sh')], cwd='/tmp',
                                            env=os.environ | {'BRIDGE': str(bridge), 'SOLVER': str(solver),
                                                'SERVER': f'http://127.0.0.1:{server.server_port}', 'TOKEN': 'test-secret',
                                                'RECORD_DIR': str(records), 'BRIDGE_LOG_DIR': str(logs),
                                                'SUBMIT_AT_MS': '', 'STREAM_RESPONSES': '0'},
                                            capture_output=True, text=True, timeout=15)
                finally:
                    server.shutdown()
                    thread.join(timeout=2)
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertTrue(state['problem_before_post'])
            self.assertTrue(state['roads_before_post'])
            saved = next(records.iterdir())
            self.assertEqual(json.loads((saved / 'problem.json').read_text()), problem)
            self.assertEqual(json.loads((saved / 'map.json').read_text()), problem['map'])
            raw = next(logs.iterdir()).read_text()
            self.assertIn('private solver detail', raw)
            self.assertIn('recording_started', raw)
            self.assertNotIn('test-secret', raw)
            self.assertNotIn('private solver detail', result.stdout + result.stderr)
            self.assertNotIn('recording_started', result.stdout + result.stderr)
            self.assertIn('problem_received map=2x2 days=1', result.stdout)
            self.assertIn('submission_status phase=plan day=1 result=accepted', result.stdout)
            for line in result.stdout.splitlines():
                self.assertRegex(line, r'^\[run.sh\] (distributed_node_access|problem_server_access|problem_received|solver_started|day=|submission |submission_status |solver_exited)')


if __name__ == '__main__':
    unittest.main(verbosity=2)
