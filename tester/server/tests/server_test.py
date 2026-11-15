"""CUI and HTTP run matches through the match window and reproduce them from the recorded submissions.

Uses tests/fake_official.py instead of the distributed procon-server. Build target/release/server first.
"""
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request

REPO = Path(__file__).resolve().parents[3]
FAKE = Path(__file__).resolve().parent / 'fake_official.py'
HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def request(url, value=None, method=None):
    data = json.dumps(value).encode() if value is not None else None
    req = urllib.request.Request(url, data=data, method=method, headers={'Content-Type': 'application/json'})
    try:
        with HTTP.open(req, timeout=10) as response:
            raw = response.read()
            return response.status, json.loads(raw) if raw.strip().startswith((b'{', b'[')) else raw.decode()
    except urllib.error.HTTPError as e:
        with e:
            raw = e.read()
        return e.code, json.loads(raw) if raw.strip().startswith(b'{') else raw.decode()


class Team(threading.Thread):
    """A solver PC: waits for the problem on the official API only, then answers each day like server-bridge."""
    def __init__(self, window, team, mode):
        super().__init__(daemon=True)
        self.window, self.token, self.mode = window, f'token-p{team}', mode

    def call(self, path, value=None, method=None):
        return request(f'{self.window}{path}?token={self.token}', value, method)

    def run(self):
        until = time.monotonic() + 30
        while True:
            try:
                status, problem = self.call('/setting')
                if status == 200:
                    break
            except urllib.error.URLError:
                pass  # The CUI opens the window after this thread starts.
            assert time.monotonic() < until
            time.sleep(.05)
        agents = len(problem['agents'])
        self.call('/agent', [0] * agents)
        for day, steps in enumerate(problem['daySteps']):
            while True:
                status, state = self.call('/')
                if status == 200 and state['day'] >= day:
                    break
                if status != 200 and 'ended' in state['message']:
                    return
                time.sleep(.05)
            plan = [[-steps]] * agents
            if self.mode == 'invalid' and day == 0:
                plan = [[0]]
            time.sleep(.02)  # Think briefly so the recorded answer time is measurable.
            self.call('/', plan)


class ServerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='procon37-server-http-')
        self.root = Path(self.temp.name)
        self.match_port = free_port()
        self.window = f'http://127.0.0.1:{self.match_port}'
        self.server = subprocess.Popen([str(REPO / 'target/release/server'), 'serve', '--port', '0', '--match-port', str(self.match_port),
                                        '--output-dir', str(self.root), '--server-program', str(FAKE)],
                                       cwd=REPO, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        for line in self.server.stderr:
            if line.startswith('ビジュアライザ: '):
                self.url = line.strip().split(' ', 1)[1]
                break

    def tearDown(self):
        self.server.terminate()
        self.server.wait(timeout=10)
        self.server.stderr.close()
        self.temp.cleanup()

    def api(self, path, value=None, method=None):
        status, body = request(self.url + path, value, method)
        self.assertLess(status, 400, body)
        return body

    def published(self):
        return request(self.window + '/setting?token=token-p1')[0] == 200

    def finished(self, id):
        until = time.monotonic() + 60
        while time.monotonic() < until:
            manifest = self.api('api/runs/' + id)['manifest']
            if manifest['status'] != 'running':
                return manifest
            time.sleep(.2)
        self.fail('run did not finish')

    def test_gui_run_reproduces_accepted_rejected_and_missing_answers(self):
        teams = [Team(self.window, 0, 'valid'), Team(self.window, 1, 'invalid')]
        for team in teams:
            team.start()
        config = self.api('api/config')
        config.update(count=1, format='first', players=3, teamNames=['A', 'B'], timeLimitMs=1000, label='HTTP verification')
        manifest = self.finished(self.api('api/runs', config)['manifest']['id'])
        self.assertEqual(manifest['status'], 'completed')
        case = manifest['cases'][0]
        self.assertEqual([p['name'] for p in case['players']], ['A', 'B', 'チーム2'])
        self.assertEqual([p['status'] for p in case['players']], ['AC', 'WA', 'TLE'])
        self.assertEqual(case['status'], 'TLE')
        self.assertTrue(all(p['rank'] == 1 for p in case['players']))
        replay = self.api(f"api/runs/{manifest['id']}/0/replay")
        self.assertEqual(replay['schemaVersion'], 2)
        self.assertEqual(len(replay['opponents']), 2)
        self.assertTrue(all(d['failure'] is None for d in replay['days']))
        self.assertGreater(replay['answerTimeMs'], 0)
        rejected = replay['opponents'][0]
        self.assertEqual(rejected['days'][0]['failure']['kind'], 'invalid_plan')
        self.assertIsNone(rejected['days'][1]['failure'])
        self.assertEqual(replay['opponents'][1]['selectionFailure']['kind'], 'timeout')
        status, log = request(f"{self.url}api/runs/{manifest['id']}/0/team-log?player=1")
        self.assertEqual(status, 200)
        self.assertIn('却下', log)
        self.assertIn('listening', request(f"{self.url}api/runs/{manifest['id']}/0/server-log")[1])
        self.assertEqual(request(self.window + '/setting?token=token-p0')[0], 403)
        # The window answers like a finished match so a waiting bridge stops.
        status, body = request(self.window + '/?token=token-p0')
        self.assertEqual((status, body['message']), (403, 'match has ended'))

        # The CUI refuses to share the busy match window and reads the same history.
        cli = subprocess.run([str(REPO / 'target/release/server'), 'run', '1', '--match-port', str(self.match_port), '--output-dir', str(self.root)],
                             cwd=REPO, capture_output=True, text=True, timeout=10)
        self.assertNotEqual(cli.returncode, 0)
        self.assertIn('対戦窓口のポート', cli.stderr)
        runs = self.api('api/runs')
        self.assertEqual([m['id'] for m in runs], [manifest['id']])

    def test_cancel_withdraws_the_match_and_keeps_the_record(self):
        config = self.api('api/config')
        config.update(count=3, players=2, timeLimitMs=1000)
        id = self.api('api/runs', config)['manifest']['id']
        until = time.monotonic() + 10
        while not self.published():
            self.assertLess(time.monotonic(), until)
            time.sleep(.05)
        with HTTP.open(self.window + '/setting?token=token-p1', timeout=10) as setting:
            self.assertEqual(json.loads(setting.read())['players'], 2)
            # The shortened time limit is told only through this header; the official server never sends it.
            self.assertLessEqual(int(setting.headers['Procon37-Select-Remaining-Ms']), 1000)
        status, _ = request(self.url + 'api/shutdown', {}, 'POST')
        self.assertEqual(status, 409)
        self.api(f'api/runs/{id}/cancel', {})
        manifest = self.finished(id)
        self.assertEqual((manifest['status'], manifest['completed']), ('cancelled', 0))
        self.assertEqual(request(self.window + '/setting?token=token-p1'), (403, {'code': 'AccessTimeError', 'message': 'match has ended'}))
        self.api('api/runs/' + id, method='DELETE')
        self.assertFalse((self.root / id).exists())
        status, error = request(self.url + 'api/runs', dict(config, players=17))
        self.assertEqual(status, 400)
        self.assertIn('チーム数', error['error'])


class CliTests(unittest.TestCase):
    def test_cli_run_and_compare_share_the_history_format(self):
        with tempfile.TemporaryDirectory(prefix='procon37-server-cli-') as temp:
            port = free_port()
            window = f'http://127.0.0.1:{port}'
            ids = []
            for label in ['first', 'second']:
                team = Team(window, 0, 'valid')
                team.start()
                result = subprocess.run([str(REPO / 'target/release/server'), 'run', '--seed', '1', '--format', 'first', '--time-limit-ms', '1000',
                                         '--label', label, '--match-port', str(port), '--output-dir', temp, '--server-program', str(FAKE)],
                                        cwd=REPO, capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn('チーム別', result.stdout)
                ids.append(result.stdout.strip().splitlines()[-1].rsplit('/', 1)[1])
                team.join(timeout=5)
            compared = subprocess.run([str(REPO / 'target/release/server'), 'compare', *ids, '--output-dir', temp, '--json'],
                                      cwd=REPO, capture_output=True, text=True, timeout=10, check=True)
            report = json.loads(compared.stdout)
            self.assertEqual(report['scopes'][0]['matched'], 1)
            self.assertEqual(report['scopes'][0]['results'][1]['outcomes'], {'wins': 0, 'draws': 1, 'losses': 0})


if __name__ == '__main__':
    unittest.main(verbosity=2)
