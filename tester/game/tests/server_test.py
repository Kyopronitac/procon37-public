"""CUI and HTTP use the same five-player replay, archive and cancellation lifecycle."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest
import urllib.request

REPO = Path(__file__).resolve().parents[3]
HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}))

class ServerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='procon37-game-http-')
        self.root = Path(self.temp.name)
        self.server = subprocess.Popen([str(REPO / 'target/release/game'), 'serve', '--port', '0', '--output-dir', str(self.root)], cwd=REPO, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        line = self.server.stderr.readline()
        self.assertIn('http://127.0.0.1:', line)
        self.url = line.strip().split(' ', 1)[1]

    def tearDown(self):
        self.server.terminate()
        self.server.wait(timeout=5)
        self.server.stderr.close()
        self.temp.cleanup()

    def api(self, path, value=None, method=None, raw=False):
        data = value if isinstance(value,bytes) else json.dumps(value).encode() if value is not None else None
        req = urllib.request.Request(self.url + path, data=data, method=method, headers={'Content-Type':'application/zip' if isinstance(value,bytes) else 'application/json'})
        with HTTP.open(req, timeout=15) as response:
            return response.read() if raw else json.load(response)

    def config(self, mode='valid'):
        config = self.api('api/config')
        config.update(count=1, jobs=1, timeLimitMs=2000, build=None, label='HTTP / CUI verification', targetName='candidate')
        for i in range(5):
            spec = {'program':'python3','args':['-u',str(REPO / 'tester/game/tests/fixture.py'),mode],'cwd':str(self.root),'env':{'PLAYER_ID':str(i)}}
            if i==0: config['solver']=spec
            else: config['opponents'][i-1].update(solver=spec, build=None)
        return config

    def finished(self, id):
        until = time.monotonic()+15
        while time.monotonic()<until:
            result=self.api('api/runs/'+id)
            if result['manifest']['status'] not in ['running','building']:
                return result['manifest']
            time.sleep(.05)
        self.fail('batch did not finish')

    def test_cli_gui_replay_zip_comparison_and_deletion(self):
        config=self.config()
        path=self.root/'config.json';path.write_text(json.dumps(config))
        subprocess.run([str(REPO / 'target/release/game'),'run','--config',str(path),'--output-dir',str(self.root)],cwd=REPO,check=True,capture_output=True,timeout=15)
        cui=self.api('api/runs')[0]
        self.assertEqual(cui['cases'][0]['status'],'AC')
        gui=self.finished(self.api('api/runs',config)['manifest']['id'])
        self.assertEqual(gui['cases'][0]['score'],cui['cases'][0]['score'])
        for m in [cui,gui]:
            replay=self.api(f"api/runs/{m['id']}/0/replay")
            self.assertEqual(replay['schemaVersion'],2)
            self.assertEqual(len(replay['opponents']),4)
            for i in range(5):
                self.api(f"api/runs/{m['id']}/0/stderr?player={i}",raw=True)
        generated=self.api('api/generate?seed=0')
        self.assertEqual(generated['problem']['players'],5)
        zip_data=self.api(f"api/runs/{gui['id']}/export",raw=True)
        imported=self.api('api/runs/import',zip_data)
        self.assertEqual(imported['cases'],gui['cases'])
        self.assertNotEqual(imported['config']['solver']['program'],'python3')
        comparison=self.api('api/compare',{'ids':[gui['id'],imported['id']]})
        self.assertEqual(comparison['rows'][0]['alignment'],'matched')
        self.api('api/runs/'+imported['id'],method='DELETE')
        self.assertFalse((self.root/imported['id']).exists())
        self.assertTrue((self.root/gui['id']).exists())

    def test_cancel_and_independent_identity(self):
        identity=self.api('api/launcher',raw=True).decode().splitlines()
        self.assertEqual(identity[0],'procon37-game-launcher-v1')
        config=self.config('cancel')
        id=self.api('api/runs',config)['manifest']['id']
        until=time.monotonic()+5
        while time.monotonic()<until and not all((self.root/f'started-{i}.json').exists() for i in range(5)):
            time.sleep(.02)
        self.assertTrue(all((self.root/f'started-{i}.json').exists() for i in range(5)))
        self.api(f'api/runs/{id}/cancel',{})
        self.assertEqual(self.finished(id)['status'],'cancelled')

    def test_arena_map_switch_matches_cli_preview_and_saved_runs(self):
        self.assertTrue(self.api('api/config')['arenaMaps'])
        for included in [True, False]:
            flag = '--arena-maps' if included else '--no-arena-maps'
            preview = self.api(f'api/generate?seed=3&arenaMaps={str(included).lower()}')['problem']
            cli = subprocess.run([str(REPO / 'target/release/game'), 'generate', '--seed', '3', flag],
                                 cwd=REPO, check=True, capture_output=True, timeout=15)
            self.assertEqual(json.loads(cli.stdout), preview)
            config = self.config()
            # A command-line switch overrides the opposite setting from JSON.
            config.update(count=None, seed='3', arenaMaps=not included)
            path = self.root / 'arena-config.json'
            path.write_text(json.dumps(config))
            previous_ids = {m['id'] for m in self.api('api/runs')}
            subprocess.run([str(REPO / 'target/release/game'), 'run', '--config', str(path), flag, '--output-dir', str(self.root)],
                           cwd=REPO, check=True, capture_output=True, timeout=15)
            cui = next(m for m in self.api('api/runs') if m['id'] not in previous_ids)
            config['arenaMaps'] = included
            gui = self.finished(self.api('api/runs', config)['manifest']['id'])
            for manifest in [cui, gui]:
                self.assertEqual(manifest['config']['arenaMaps'], included)
                replay = self.api(f"api/runs/{manifest['id']}/3/replay")
                self.assertEqual(replay['case']['problem'], preview)
                self.assertEqual(len(replay['opponents']), 4)
            if included:
                source = json.loads((REPO / 'common/fixtures/arena/match-937.json').read_text())
                source.update(players=5, startsAt=0)
                self.assertEqual(preview, source)
                arena_map = preview['map']
            else:
                self.assertNotEqual(preview['map'], arena_map)
                self.assertTrue(all(seconds == 60 for seconds in preview['daySeconds']))

if __name__=='__main__':
    unittest.main(verbosity=2)
