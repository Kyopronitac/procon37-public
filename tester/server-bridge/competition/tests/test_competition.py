"""Local fixture checks, no SSH, match server, real search or protected solver code."""
import contextlib
import ast
import hashlib
import importlib.util
import io
import json
import os
import socket
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
spec = importlib.util.spec_from_file_location('competition_controller', HERE / 'distributed.py')
d = importlib.util.module_from_spec(spec)
spec.loader.exec_module(d)


class CompetitionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='competition-mock-')
        self.root = Path(self.temp.name)
        self.cfg = json.loads((HERE / 'config.example.json').read_text())
        self.cfg['remote_directory'] = 'procon37-competition/releases/C013-fixture'
        for name in ('solver', 'bridge'):
            file = self.root / name
            file.write_bytes(b'\x7fELF\x02\x01' + name.encode())
            self.cfg[name + '_binary'] = str(file)
            self.cfg[name + '_sha256'] = d.base.sha(file)
        self.config = self.root / 'config.json'
        self.config.write_bytes(d.base.config_bytes(self.cfg))
        self.evidence = self.root / 'evaluation.json'
        self.evidence.write_text(json.dumps({'files': {'solver': self.cfg['solver_sha256'],
                                                      'bridge': self.cfg['bridge_sha256']}}))

    def tearDown(self):
        self.temp.cleanup()

    def package(self):
        archive = self.root / 'release.tar.gz'
        with patch.object(d, 'BRIDGE_SHA256', self.cfg['bridge_sha256']), contextlib.redirect_stdout(io.StringIO()):
            d.package(self.config, archive, self.evidence, 'C013-fixture')
        return archive

    def extract(self, archive):
        root = self.root / 'release'
        root.mkdir()
        with tarfile.open(archive) as tar:
            for member in tar.getmembers():
                self.assertTrue(member.isfile())
                self.assertNotIn('..', Path(member.name).parts)
                file = root / member.name
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_bytes(tar.extractfile(member).read())
                file.chmod(member.mode)
        (root / 'private').mkdir()
        token = root / 'private/cluster.token'
        token.write_text('x' * 64)
        token.chmod(0o600)
        return root

    def verify(self, root):
        with patch.object(d, 'BRIDGE_SHA256', self.cfg['bridge_sha256']), \
                patch.object(os, 'sched_getaffinity', return_value=set(range(32))):
            return d.verify_node(root, 'v1')

    def test_all_threads_cross_elite_and_both_stream_flags_are_fixed(self):
        for role, count in [('v1', 32), ('v2', 16), ('v3', 32)]:
            args = d.solver_arguments(self.cfg, role)
            self.assertEqual(args[args.index('--threads') + 1], str(count))
            self.assertEqual(args[args.index('--cross-elite') + 1], 'on')
            self.assertEqual('--stream-responses' in args, role == 'v1')
        settings = {'server_url': 'http://localhost:1234', 'token': 'fixture', 'select_ms': None}
        args = d.bridge_arguments(self.root, self.cfg, settings)
        before, after = args[:args.index('--')], args[args.index('--') + 1:]
        self.assertEqual(before[before.index('--submit-at-ms') + 1], '5000,15000,25000,35000,45000,55000')
        self.assertIn('--stream-responses', before)
        self.assertIn('--stream-responses', after)
        self.assertEqual(before[before.index('--record-dir') + 1], str(self.root / 'records'))
        self.assertNotIn('--select-ms', args)
        self.assertNotIn('--time-limit-ms', args)

    def test_mdns_addresses_preserve_config_and_follow_dhcp(self):
        for index, node in enumerate(self.cfg['nodes'].values(), 1):
            node['address'] = f'voidlinux-{index}.local'
            node['ssh'] = f'voidlinux-{index}@voidlinux-{index}.local'
        self.config.write_bytes(d.base.config_bytes(self.cfg))
        loaded = d.base.load_config(self.config, allow_placeholder=True)
        original = d.base.config_bytes(loaded)
        def resolve(host, *args):
            index = host.split('.')[0][-1]
            return [(socket.AF_INET, socket.SOCK_STREAM, 6, '', (f'169.254.1.{index}', 0)),
                    (socket.AF_INET, socket.SOCK_STREAM, 6, '', (f'172.28.1.{index}', 0))]
        with patch.object(socket, 'getaddrinfo', side_effect=resolve):
            args = d.solver_arguments(loaded, 'v1')
            self.assertEqual(args[args.index('--peers') + 1],
                             '172.28.1.2:39371/16/31,172.28.1.3:39371/32/46')
            args = d.solver_arguments(loaded, 'v2')
            self.assertEqual(args[args.index('--listen') + 1], '172.28.1.2:39371')
        self.assertEqual(d.base.config_bytes(loaded), original)
        with patch.object(socket, 'getaddrinfo', return_value=[
                (socket.AF_INET, socket.SOCK_STREAM, 6, '', ('172.28.2.22', 0))]):
            args = d.solver_arguments(loaded, 'v2')
            self.assertEqual(args[args.index('--listen') + 1], '172.28.2.22:39371')
        with patch.object(socket, 'getaddrinfo', side_effect=socket.gaierror('unavailable')):
            with self.assertRaises(socket.gaierror):
                d.solver_arguments(loaded, 'v2')

    def test_practice_match_and_connection_test_selection_durations(self):
        for duration, last_checkpoint, checkpoints in ((20000, 55000, 6), (30000, 55000, 6),
                                                       (60000, 55000, 6), (90000, 85000, 9),
                                                       (120000, 115000, 12)):
            settings = {'server_url': 'http://fixture.invalid:8080', 'token': 'fixture',
                        'select_ms': duration}
            args = d.bridge_arguments(self.root, self.cfg, settings)
            self.assertEqual(args[args.index('--select-ms') + 1], str(duration))
            self.assertLess(args.index('--select-ms'), args.index('--'))
            schedule = list(map(int, args[args.index('--submit-at-ms') + 1].split(',')))
            self.assertEqual(schedule[:6], [5000, 15000, 25000, 35000, 45000, 55000])
            self.assertEqual(schedule[-1], last_checkpoint)
            self.assertEqual(len(schedule), checkpoints)
            self.assertTrue(all(b - a == 10000 for a, b in zip(schedule, schedule[1:])))
        settings['select_ms'] = 45000
        with self.assertRaisesRegex(RuntimeError, 'duration'):
            d.bridge_arguments(self.root, self.cfg, settings)

    def test_stable_entry_passes_map_modes_to_selected_release(self):
        repo = self.root / 'repo'
        entry = repo / 'tester/server-bridge/distributed/run.sh'
        entry.parent.mkdir(parents=True)
        entry.write_bytes((HERE.parent / 'distributed/run.sh').read_bytes())
        release = self.root / 'fake-release'
        release.mkdir()
        (release / 'manifest.json').write_text('{}')
        (release / 'run.sh').write_text('#!/bin/sh\nprintf "%s\\n%s\\n" "$SELECT_MS" "$1"\n')
        (release / 'run.sh').chmod(0o755)
        selected = repo / '.server/distributed-production/selected-release'
        selected.parent.mkdir(parents=True)
        selected.symlink_to(release, target_is_directory=True)
        env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin')}
        result = subprocess.run(['sh', str(entry)], env=env, capture_output=True, text=True, check=True)
        self.assertEqual(result.stdout.splitlines(), ['30000', 'start'])
        for mode, duration in [('test', '20000'), ('practice', '30000'), ('match', '60000'),
                               ('match24', '90000'), ('match32', '120000')]:
            for action in ('start', 'status', 'stop'):
                with self.subTest(mode=mode, action=action):
                    result = subprocess.run(['sh', str(entry), mode, action],
                                            env={**env, 'SELECT_MS': '123'},
                                            capture_output=True, text=True, check=True)
                    self.assertEqual(result.stdout.splitlines(), [duration, action])
            result = subprocess.run(['sh', str(entry)], env={**env, 'MATCH_MODE': mode},
                                    capture_output=True, text=True, check=True)
            self.assertEqual(result.stdout.splitlines(), [duration, 'start'])

    def test_large_map_durations_pass_start_validation_without_network(self):
        for duration in ('90000', '120000'):
            with self.subTest(duration=duration), \
                    patch.object(sys, 'argv', ['distributed.py', 'start', '--config', str(self.config)]), \
                    patch.dict(os.environ, {'SERVER': 'http://fixture.invalid:8080', 'TOKEN': 'fixture',
                                            'SELECT_MS': duration}, clear=True), \
                    patch.object(d.base, 'require_void1'), \
                    patch.object(d.base, 'terminal_signals', side_effect=contextlib.nullcontext), \
                    patch.object(d.base, 'ssh_auth', side_effect=contextlib.nullcontext), \
                    patch.object(d.base, 'run_cluster') as start:
                d.main()
                self.assertEqual(start.call_args.args[1]['select_ms'], int(duration))

    def test_package_preserves_exact_binaries_and_detects_tampering(self):
        root = self.extract(self.package())
        self.verify(root)
        self.assertEqual((root / 'bin/distributed-solver').read_bytes(), Path(self.cfg['solver_binary']).read_bytes())
        self.assertEqual((root / 'bin/server-bridge').read_bytes(), Path(self.cfg['bridge_binary']).read_bytes())
        (root / 'bin/distributed-solver').write_bytes(b'changed')
        with self.assertRaisesRegex(RuntimeError, 'release file changed'):
            self.verify(root)

    def test_no_binary_without_evaluation_evidence_is_packaged(self):
        self.evidence.write_text('{}')
        with self.assertRaisesRegex(RuntimeError, 'evaluation evidence'):
            self.package()
        self.assertFalse((self.root / 'release.tar.gz').exists())

    def test_editing_operator_entry_does_not_change_release_identity(self):
        root = self.extract(self.package())
        operator = self.root / 'operator'
        operator.mkdir()
        (operator / 'current').symlink_to(root, target_is_directory=True)
        entry = operator / 'run.sh'
        edited = d.OPERATOR_ENTRY.replace('SERVER="${SERVER:-}"', 'SERVER="http://fixture.invalid:1234"')
        edited = edited.replace('TOKEN="${TOKEN:-}"', 'TOKEN="fixture-token"')
        entry.write_text(edited)
        entry.chmod(0o755)
        self.verify(root)
        # Observe only the configuration boundary; never connect to the fictitious endpoint.
        fake = self.root / 'fake-release'
        fake.mkdir()
        (fake / 'run.sh').write_text('#!/bin/sh\nprintf "%s\\n%s\\n%s\\n" "$SERVER" "$TOKEN" "$1"\n')
        (fake / 'run.sh').chmod(0o755)
        (operator / 'current').unlink()
        (operator / 'current').symlink_to(fake, target_is_directory=True)
        result = subprocess.run([str(entry), 'status'], capture_output=True, text=True, check=True)
        self.assertEqual(result.stdout.splitlines(), ['http://fixture.invalid:1234', 'fixture-token', 'status'])
        self.verify(root)

    def test_endpoint_missing_refuses_start_without_ssh(self):
        with patch.object(sys, 'argv', ['distributed.py', 'start', '--config', str(self.config)]), \
                patch.dict(os.environ, {}, clear=True), patch.object(d.base, 'require_void1'), \
                patch.object(d.base, 'ssh_auth') as ssh:
            with self.assertRaisesRegex(RuntimeError, 'Set the actual competition SERVER'):
                d.main()
            ssh.assert_not_called()

    def test_worker_authentication_requires_protocol4_and_cross_elite(self):
        class Socket:
            def __init__(self, data):
                body = json.dumps(data).encode()
                self.data = len(body).to_bytes(4, 'big') + body
                self.sent = b''
            def __enter__(self): return self
            def __exit__(self, *args): pass
            def sendall(self, data): self.sent += data
            def recv(self, count):
                result, self.data = self.data[:count], self.data[count:]
                return result
        socket = Socket({'Ready': {'version': 4, 'total_threads': 16, 'offset': 31, 'cross_elite': True}})
        with patch.object(d.socket, 'create_connection', return_value=socket), contextlib.redirect_stdout(io.StringIO()):
            d.worker_ready(self.cfg, 'v2', 'fixture-secret')
        self.assertEqual(json.loads(socket.sent[4:]), {'Authenticate': {'version': 4, 'token': 'fixture-secret', 'cross_elite': True}})
        socket = Socket({'Ready': {'version': 3, 'total_threads': 16, 'offset': 31, 'cross_elite': True}})
        with patch.object(d.socket, 'create_connection', return_value=socket):
            with self.assertRaisesRegex(RuntimeError, 'protocol/cross-elite'):
                d.worker_ready(self.cfg, 'v2', 'fixture-secret')

    def test_operator_password_is_private_runtime_data_only(self):
        private = self.root / self.cfg['remote_directory'] / 'private'
        private.mkdir(parents=True)
        with patch.object(Path, 'home', return_value=self.root), \
                patch.dict(os.environ, {'PROCON37_SSH_PASSWORD': 'fixture-secret'}):
            d.configure_operator_auth(self.cfg)
        password = private / 'ssh-password'
        self.assertEqual(password.stat().st_mode & 0o777, 0o600)
        self.assertEqual(password.read_text(), 'fixture-secret\n')
        with tarfile.open(self.package()) as archive:
            self.assertNotIn('private/ssh-password', archive.getnames())
            self.assertTrue(all(b'fixture-secret' not in archive.extractfile(member).read()
                                for member in archive.getmembers()))

    def test_probe_cleanup_runs_when_authentication_fails(self):
        private = self.root / self.cfg['remote_directory'] / 'private'
        private.mkdir(parents=True)
        (private / 'cluster.token').write_text('fixture-token')
        status = type('Reply', (), {'stdout': '{"running":false,"identity_mismatch":false,"orphans":[]}'})()
        with patch.object(Path, 'home', return_value=self.root), \
                patch.object(d.base, 'remote_node', return_value=status), \
                patch.object(d.base, 'spawn_node', side_effect=['worker2', 'worker3']), \
                patch.object(d.base, 'wait_node_started'), \
                patch.object(d, 'worker_ready', side_effect=RuntimeError('authentication failed')), \
                patch.object(d.base, 'stop_runners', return_value=[]) as stop:
            with self.assertRaisesRegex(RuntimeError, 'authentication failed'):
                d.probe(self.cfg)
            stop.assert_called_once_with(self.cfg, {'v2': 'worker2', 'v3': 'worker3'})

    def test_partial_cluster_is_not_adopted_and_left_running_after_exit(self):
        statuses = [type('Reply', (), {'stdout': json.dumps({'running': running})})()
                    for running in (False, True, False)]
        with patch.object(d.base, 'remote_node', side_effect=statuses), \
                patch.object(d, 'ORIGINAL_START_CLUSTER') as original:
            with self.assertRaisesRegex(RuntimeError, 'partially running'):
                d.start_cluster(self.cfg, {})
            original.assert_not_called()
        statuses = [type('Reply', (), {'stdout': '{"running":false}'})() for _ in range(3)]
        with patch.object(d.base, 'remote_node', side_effect=statuses), \
                patch.object(d, 'ORIGINAL_START_CLUSTER', return_value='started') as original:
            self.assertEqual(d.start_cluster(self.cfg, {}), 'started')
            original.assert_called_once_with(self.cfg, {})

    def test_only_known_start_refusals_are_visible_as_solver_exit_reasons(self):
        spec = importlib.util.spec_from_file_location('competition_progress', HERE / 'progress.py')
        progress = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(progress)
        filtered = progress.Progress()
        for reason in ('partial_cluster', 'endpoint_unset'):
            self.assertEqual(filtered.line(f'[competition] start_refused reason={reason}\n'.encode()),
                             f'[run.sh] solver_exited result=start_failed reason={reason}')
        self.assertIsNone(filtered.line(b'[competition] verbose detail'))
        self.assertEqual(filtered.line(b'[bridge] solver_started'), '[run.sh] solver_started')

    def test_only_network_functions_differ_from_frozen_runtime(self):
        def non_network_tree(path):
            tree = ast.parse(path.read_text())
            tree.body = [node for node in tree.body if not (
                isinstance(node, ast.FunctionDef)
                and node.name in {'node_ipv4', 'load_config', 'require_void1'})]
            return ast.dump(tree)
        self.assertEqual(non_network_tree(HERE / 'runtime_base.py'),
                         non_network_tree(HERE.parent / 'distributed.py'))
        self.assertEqual((HERE / 'console.py').read_bytes(), (HERE.parent / 'console.py').read_bytes())


if __name__ == '__main__':
    unittest.main(verbosity=2)
