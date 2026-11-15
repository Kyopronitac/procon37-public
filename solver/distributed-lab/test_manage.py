import copy
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('distributed_lab_manage', Path(__file__).with_name('manage.py'))
MANAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MANAGE)


class EvaluationChecks(unittest.TestCase):
    def setUp(self):
        self.run = MANAGE.read(MANAGE.ROOT / 'solver/docs/distributed/initial-formal-run.json')

    def summarize(self, value=None, short_ms=None):
        value = self.run if value is None else value
        return MANAGE.summarize_run(value, short_ms, self.run['config']['label'])

    def test_saved_formal_results(self):
        result = self.summarize()
        self.assertTrue(result['all48AC'])
        self.assertEqual(result['players'][0]['rankSum'], 20)
        self.assertEqual(result['players'][0]['scores'], {'uniqueBrands': 222, 'dailyBrands': 1276, 'udon': 2989})

    def test_rejects_invalid_runs(self):
        changes = [lambda r: r['cases'][0].update(seed='1'),
                   lambda r: r['cases'][0]['players'][3].update(status='WA'),
                   lambda r: r['config'].update(arenaMaps=False),
                   lambda r: r['config'].update(timeLimitMs=10000),
                   lambda r: r['cases'][0].update(format='final'),
                   lambda r: r['config'].update(label='別の評価')]
        for mutate in changes:
            with self.subTest(mutation=mutate):
                invalid = copy.deepcopy(self.run)
                mutate(invalid)
                with self.assertRaises(ValueError):
                    self.summarize(invalid)

    def test_short_cannot_use_formal(self):
        with self.assertRaises(ValueError):
            self.summarize(short_ms=10000)

    def test_hardware_change_requires_a_new_baseline(self):
        cfg = {'fixedLocalFiles': {}, 'localProcesses': [], 'nodes': {},
               'serverUrl': 'http://192.168.0.81:11451'}
        old = MANAGE.comparison_conditions(cfg)
        cfg['environmentIdentity'] = {'localCpu': 'Ryzen 7 5800H', 'profile': 'performance'}
        self.assertNotEqual(old, MANAGE.comparison_conditions(cfg))
        self.assertEqual(MANAGE.comparison_conditions(cfg),
                         MANAGE.comparison_conditions(copy.deepcopy(cfg)))

    def test_stage_and_protected_paths(self):
        for name in ('../outside', '', '/tmp/run'):
            with self.assertRaises(ValueError):
                MANAGE.stage_directory(name)
        for name in ('tester/game/samples/a', 'solver/docs/solver-pool-experiment/a', '../a'):
            with self.assertRaises(ValueError):
                MANAGE.local_path(name)

    def test_pending_evaluation_blocks_configure_even_after_pid_exit(self):
        with patch.object(MANAGE, 'state', return_value={'evaluation': {'status': 'running', 'pid': 99999999}}):
            with self.assertRaises(ValueError):
                MANAGE.require_idle()

    def test_pid_reuse_is_not_alive(self):
        record = {'pid': 100, 'startTicks': '1000', 'bootId': 'a'}
        for current in (None, dict(record, startTicks='2000', state='S'), dict(record, bootId='b', state='S'), dict(record, state='Z')):
            with patch.object(MANAGE, 'process_identity', return_value=current):
                self.assertFalse(MANAGE.alive(record))

    def test_snapshot_change_detection(self):
        before = {'configurationSha256': 'config', 'harnessSha256': 'harness',
                  'localFiles': {'binary': 'a'}, 'local': {'bootId': 'local'},
                  'remote': {'v1': {'files': {'binary': 'b'}, 'bootId': 'remote'}}}
        MANAGE.unchanged(before, copy.deepcopy(before))
        for mutate in (lambda s: s['localFiles'].update(binary='new'),
                       lambda s: s['remote']['v1']['files'].update(binary='new'),
                       lambda s: s['remote']['v1'].update(bootId='new'),
                       lambda s: s.update(configurationSha256='new')):
            after = copy.deepcopy(before)
            mutate(after)
            with self.assertRaises(ValueError):
                MANAGE.unchanged(before, after)

    def test_missing_host_does_not_create_stage_or_start_server(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            configuration = root / 'config.json'
            configuration.write_text(json.dumps({'version': 'D0'}))
            with patch.object(MANAGE, 'WORK', root), patch.object(MANAGE, 'CONFIG', configuration), \
                    patch.object(MANAGE, 'require_idle'), patch.object(MANAGE, 'require_no_server'), \
                    patch.object(MANAGE, 'snapshot', side_effect=ConnectionError('NOMLAB offline')), \
                    patch.object(MANAGE.subprocess, 'Popen') as popen:
                with self.assertRaises(ConnectionError):
                    MANAGE.launch('offline-check', 'D0・短縮12件', 10000)
                popen.assert_not_called()
                self.assertFalse((root / 'evaluations/offline-check').exists())


if __name__ == '__main__':
    unittest.main()
