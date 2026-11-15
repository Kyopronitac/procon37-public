"""Exercise an actual bridge binary against a local HTTP server and JSONL solver.

Run: python3 tester/server-bridge/tests/check_bridge_logs.py target/debug/server-bridge
"""
import http.server
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from urllib.parse import urlsplit


BRIDGE = None


class BridgeLogTests(unittest.TestCase):
    def run_match(self, *, revision=1, retry=False, invalid_selection=False, auth=False, failed_plan=False, recording_failure=False):
        state = {'ended': False, 'settings': 0, 'plans': 0, 'paths': []}
        problem = {
            'startsAt': 0, 'daySeconds': [5], 'daySteps': [1],
            'map': {'width': 2, 'height': 2, 'cells': [[0, 0], [0, 0]]},
            'spots': [{'brand': 0, 'pos': 3, 'stocks': 1}], 'agents': [0],
            'fuelLimits': 10, 'players': 1, 'busyThreshold': 1, 'jammedThreshold': 2,
            'extraServerField': {'keep': True},
        }

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def respond(self, status, body):
                raw = json.dumps(body).encode()
                self.send_response(status)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(raw)))
                self.send_header('Procon37-Match-Id', 'log-test')
                self.send_header('Procon37-Select-Remaining-Ms', '2000')
                self.end_headers()
                self.wfile.write(raw)

            def do_GET(self):
                path = urlsplit(self.path).path
                state['paths'].append(('GET', path))
                if auth:
                    self.respond(401, {'message': '認証エラー'})
                elif path == '/setting':
                    state['settings'] += 1
                    if state['settings'] == 1:
                        self.respond(403, {'message': 'match has not started'})
                    else:
                        self.respond(200, problem)
                elif state['ended'] or (failed_plan and state.get('observed')):
                    self.respond(403, {'message': 'match has ended'})
                else:
                    state['observed'] = True
                    observation = {
                        'endsAt': int(time.time()) + 5, 'day': 0,
                        'agents': [{'kind': 0, 'pos': 0, 'fuel': 10}],
                        'others': [], 'traffics': [{'pos': 1, 'status': 2}],
                    }
                    state['observation'] = observation
                    self.respond(200, observation)

            def do_POST(self):
                path = urlsplit(self.path).path
                state['paths'].append(('POST', path))
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                if not recording_failure:
                    saved = next(record_root.iterdir())
                    if path == '/agent':
                        state['problem_saved_before_submission'] = json.loads((saved / 'problem.json').read_text()) == problem
                    else:
                        saved_observations = [json.loads(line) for line in (saved / 'observations.jsonl').read_text().splitlines()]
                        state['state_saved_before_submission'] = saved_observations[-1]['state'] == state['observation']
                if path == '/agent':
                    state['selection'] = body
                    self.respond(200, {})
                else:
                    state['plans'] += 1
                    if retry and state['plans'] == 1:
                        self.respond(503, {'message': '上流に接続できません'})
                    else:
                        state['plan'] = body
                        state['ended'] = True
                        self.respond(200, {'revision': revision})

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            record_root = root / 'match records'
            if recording_failure:
                record_root.write_text('not a directory')
            solver = root / 'solver.py'
            solver.write_text(
                'import json, sys\n'
                'for line in sys.stdin:\n'
                '    request = json.loads(line)\n'
                '    print("[dummy] solver diagnostic", file=sys.stderr, flush=True)\n'
                f'    if {failed_plan!r} and request["phase"] == "plan":\n'
                '        sys.exit(7)\n'
                '    answer = [[0]]\n'
                '    if request["phase"] == "select":\n'
                f'        answer = {"{}" if invalid_selection else "[0]"}\n'
                '    print(json.dumps({"requestId": request["requestId"], '
                '"answer": answer}), flush=True)\n'
            )
            with http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler) as server:
                thread = threading.Thread(target=server.serve_forever, daemon=True)
                thread.start()
                try:
                    result = subprocess.run([
                        str(BRIDGE), '--server', f'http://127.0.0.1:{server.server_port}',
                        '--token', 'test-token', '--solver', sys.executable,
                        '--cwd', str(root), '--record-dir', str(record_root), '--', str(solver),
                    ], capture_output=True, text=True, timeout=15)
                finally:
                    server.shutdown()
                    thread.join(timeout=2)
            self.assertFalse((root / 'stderr.log').exists())
            if auth:
                self.assertFalse(record_root.exists())
            elif recording_failure:
                self.assertEqual(record_root.read_text(), 'not a directory')
                self.assertIn('recording_failed', result.stderr)
                self.assertIn('submissions_continue=true', result.stderr)
            else:
                directories = list(record_root.iterdir())
                self.assertEqual(len(directories), 1)
                saved = directories[0]
                self.assertEqual(json.loads((saved / 'problem.json').read_text()), problem)
                self.assertEqual(json.loads((saved / 'map.json').read_text()), problem['map'])
                metadata = json.loads((saved / 'metadata.json').read_text())
                self.assertEqual(metadata['matchId'], 'log-test')
                self.assertGreater(metadata['selectionDeadlineMs'], metadata['receivedAtMs'])
                observations = [json.loads(line) for line in (saved / 'observations.jsonl').read_text().splitlines()]
                self.assertEqual([item['state'] for item in observations], [state['observation']])
                http_records = (saved / 'http.jsonl').read_text()
                self.assertNotIn('test-token', http_records)
                events = [json.loads(line) for line in http_records.splitlines()]
                self.assertTrue(all(item['path'] in ('/setting', '/agent', '/') for item in events))
                self.assertTrue(any(item['event'] == 'request' and item['method'] == 'POST' and item['body'] == [0] for item in events))
                if not failed_plan:
                    self.assertTrue(any(item['event'] == 'request' and item['method'] == 'POST' and item['body'] == [[0]] for item in events))
                    self.assertTrue(any(item['event'] == 'response' and item['body'] == {'revision': revision} for item in events))
                self.assertIn('recording_started directory=', result.stderr)
                self.assertTrue(state['problem_saved_before_submission'])
                if not failed_plan:
                    self.assertTrue(state['state_saved_before_submission'])
        self.assertEqual(result.stdout, '')  # JSONL replies stay on the protocol pipe.
        self.assertNotRegex(result.stderr, '[\u3040-\u30ff\u3400-\u9fff]')
        self.assertTrue(all(path in ('/setting', '/agent', '/') for _, path in state['paths']))
        if not auth:
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(state['selection'], [0])
            if not failed_plan:
                self.assertEqual(state['plan'], [[0]])
            self.assertIn('server_connection result=reachable http_status=403', result.stderr)
            self.assertIn('problem_wait http_status=403 state=not_ready', result.stderr)
            self.assertIn('width=2 height=2 agents=1 days=1 players=1 spots=1 brands=1', result.stderr)
            self.assertIn('fuel_limit=10 busy_threshold=1 jammed_threshold=2', result.stderr)
            self.assertIn('day_steps=[1] day_seconds=[5]', result.stderr)
            self.assertIn('[dummy] solver diagnostic', result.stderr)
            for phase in ('select', 'plan'):
                self.assertIn(f'solve_start phase={phase}', result.stderr)
                self.assertRegex(result.stderr, rf'solve_end phase={phase} .*solver_ms=\d+')
            self.assertRegex(result.stderr, r'selection_submission result=accepted .*submit_ms=\d+')
            self.assertRegex(result.stderr, r'match_complete elapsed_ms=\d+')
        return result, state

    def test_acceptance_after_gateway_retry_and_connection_recovery(self):
        result, state = self.run_match(retry=True)
        self.assertEqual(state['plans'], 2)
        self.assertIn('connection_lost retrying=true reason=upstream_unavailable http_status=503', result.stderr)
        self.assertRegex(result.stderr, r'connection_restored outage_ms=\d+')
        self.assertRegex(result.stderr, r'plan_submission day=1 .*result=accepted .*submit_ms=\d+ attempts=2')

    def test_http_success_can_still_mean_rejected(self):
        result, _ = self.run_match(revision=-1)
        self.assertRegex(result.stderr, r'plan_submission day=1 .*http_status=200 revision=-1 result=rejected http_accepted=false')

    def test_invalid_selection_reports_the_submitted_fallback(self):
        result, _ = self.run_match(invalid_selection=True)
        self.assertIn('result=failed fallback=true failure_kind=invalid_answer', result.stderr)
        self.assertIn('vehicle_selection kinds=[0] fallback=true', result.stderr)
        self.assertRegex(result.stderr, r'selection_submission result=accepted .*fallback=true')

    def test_authentication_failure_stops_before_starting_the_solver(self):
        result, state = self.run_match(auth=True)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(state['paths'], [('GET', '/setting')])
        self.assertIn('authentication_failed http_status=401 action=check_TOKEN_or_TEAM', result.stderr)
        self.assertNotIn('problem_received', result.stderr)
        self.assertNotIn('[dummy]', result.stderr)

    def test_solver_failure_reports_no_submission_and_elapsed_time(self):
        result, state = self.run_match(failed_plan=True)
        self.assertEqual(state['plans'], 0)
        self.assertRegex(result.stderr, r'plan_submission day=1 result=not_submitted reason=solver_failed failure_kind=\S+ solver_ms=\d+')
        self.assertNotIn(('POST', '/'), state['paths'])

    def test_recording_failure_does_not_prevent_submission(self):
        result, _ = self.run_match(recording_failure=True)
        self.assertIn('result=accepted', result.stderr)


if __name__ == '__main__':
    if len(sys.argv) != 2:
        sys.exit('Usage: check_bridge_logs.py PATH_TO_SERVER_BRIDGE')
    BRIDGE = Path(sys.argv.pop()).resolve(strict=True)
    unittest.main(verbosity=2)
