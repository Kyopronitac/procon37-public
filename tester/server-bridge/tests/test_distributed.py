"""Deployment checks only: no SSH, official server, or solver execution."""
import copy
import contextlib
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest
from unittest.mock import Mock, patch

SCRIPT = Path(__file__).resolve().parents[1] / 'distributed.py'
spec = importlib.util.spec_from_file_location('distributed_deploy', SCRIPT)
d = importlib.util.module_from_spec(spec)
spec.loader.exec_module(d)


class DeploymentTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='distributed-cli-test-')
        self.base = Path(self.temp.name)
        self.cfg = json.loads(SCRIPT.with_name('distributed-config.example.json').read_text())
        self.cfg['server_url'] = 'http://192.168.0.81:11451'
        for name in ('solver', 'bridge'):
            binary = self.base / (name + '-binary')
            binary.write_bytes(b'\x7fELF\x02\x01' + name.encode())
            self.cfg[name + '_binary'] = str(binary)
            self.cfg[name + '_sha256'] = d.sha(binary)

    def tearDown(self):
        self.temp.cleanup()

    def extract_package(self, output, node):
        # Supports Python 3.10 too; only regular, known deployment files are extracted.
        node.mkdir()
        with tarfile.open(output) as archive:
            for member in archive.getmembers():
                self.assertTrue(member.isfile())
                self.assertFalse(Path(member.name).is_absolute())
                self.assertNotIn('..', Path(member.name).parts)
                target = node / member.name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(archive.extractfile(member).read())
                target.chmod(member.mode)

    def test_package_contains_the_registered_compiled_binaries(self):
        output = self.base / 'release.tar.gz'
        digest = d.package(self.cfg, output)
        self.assertEqual(digest, d.sha(output))
        with tarfile.open(output) as archive:
            self.assertEqual(set(archive.getnames()), {'bin/distributed-solver', 'bin/server-bridge',
                                                     'distributed.py', 'node-config.json', 'manifest.json'})
            manifest = json.load(archive.extractfile('manifest.json'))
            for name, expected in manifest.items():
                self.assertEqual(hashlib.sha256(archive.extractfile(name).read()).hexdigest(), expected)
            self.assertEqual(manifest['bin/distributed-solver'], self.cfg['solver_sha256'])
            self.assertEqual(manifest['bin/server-bridge'], self.cfg['bridge_sha256'])
            for name, target in [('solver', 'distributed-solver'), ('bridge', 'server-bridge')]:
                self.assertEqual(archive.extractfile('bin/' + target).read(),
                                 Path(self.cfg[name + '_binary']).read_bytes())
        with self.assertRaisesRegex(RuntimeError, 'already exists'):
            d.package(self.cfg, output)

    def test_wrong_binary_hash_stops_packaging(self):
        self.cfg['solver_sha256'] = '0' * 64
        with self.assertRaisesRegex(RuntimeError, 'SHA256 mismatch'):
            d.package(self.cfg, self.base / 'bad.tar.gz')
        self.assertFalse((self.base / 'bad.tar.gz').exists())

    def test_first_deploy_requires_compile_before_any_ssh_or_package(self):
        config = self.base / 'config.json'
        with patch.object(sys, 'argv', [str(SCRIPT), 'deploy', '--config', str(config)]), \
                patch.object(d, 'remote') as remote, patch.object(d, 'package') as package:
            with self.assertRaisesRegex(RuntimeError, 'compile.sh'):
                d.main()
            remote.assert_not_called()
            package.assert_not_called()
        cfg = d.operator_config(config, 'compile', None)
        self.assertIsNone(cfg['solver_sha256'])
        self.assertIsNone(cfg['bridge_sha256'])

    def test_bundled_binaries_cannot_bypass_compile(self):
        for name, target in [('solver', 'distributed-solver'), ('bridge', 'server-bridge')]:
            cfg = self.cfg | {name + '_binary': 'bundled:' + target}
            output = self.base / (name + '.tar.gz')
            with self.assertRaisesRegex(RuntimeError, 'compile.sh'):
                d.package(cfg, output)
            self.assertFalse(output.exists())

    def test_missing_binary_does_not_fall_back_to_bundled_files(self):
        Path(self.cfg['solver_binary']).unlink()
        output = self.base / 'missing.tar.gz'
        with self.assertRaisesRegex(RuntimeError, 'compile.sh'):
            d.package(self.cfg, output)
        self.assertFalse(output.exists())

    def test_configured_ssh_password_needs_no_prompt_and_helper_is_cleaned(self):
        with patch('builtins.input') as prompt, patch.dict(os.environ, {'PROCON37_SSH_PASSWORD': 'fixture-password'}):
            with d.ssh_auth():
                helper = Path(d.SSH_ENV['SSH_ASKPASS'])
                result = subprocess.run([str(helper)], check=True, capture_output=True, text=True)
                self.assertEqual(result.stdout.strip(), 'fixture-password')
                self.assertEqual(d.SSH_ENV['SSH_ASKPASS_REQUIRE'], 'force')
            prompt.assert_not_called()
            self.assertIsNone(d.SSH_ENV)
            self.assertFalse(helper.exists())

    def test_failed_compile_preserves_the_distribution_configuration(self):
        config = self.base / 'config.json'
        original = d.config_bytes(self.cfg)
        config.write_bytes(original)
        with patch.object(d, 'run', side_effect=[subprocess.CompletedProcess([], 0),
                                               subprocess.CalledProcessError(101, ['cargo'])]):
            with self.assertRaises(subprocess.CalledProcessError):
                d.compile_release(self.cfg, config)
        self.assertEqual(config.read_bytes(), original)

    def test_compile_registers_both_binaries_only_after_success(self):
        config = self.base / 'config.json'
        # Existing bundled configurations must migrate through both Cargo builds too.
        original = self.cfg | {'solver_binary': 'bundled:distributed-solver',
                               'bridge_binary': 'bundled:server-bridge'}
        config.write_bytes(d.config_bytes(original))
        cfg = d.operator_config(config, 'compile', None)
        paths = ['solver/distributed/target/release/distributed-solver', 'target/release/server-bridge']
        for name in paths:
            target = self.base / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(b'\x7fELF\x02\x01' + name.encode())
        with patch.object(d, 'ROOT', self.base), patch.object(d, 'run') as compiler:
            d.compile_release(cfg, config)
        self.assertEqual(compiler.call_count, 2)
        self.assertTrue(all('cargo' in call.args[0] and 'build' in call.args[0]
                            for call in compiler.call_args_list))
        selected = d.load_config(config)
        for kind, name in zip(('solver', 'bridge'), paths):
            self.assertEqual(selected[kind + '_binary'], name)
            self.assertEqual(selected[kind + '_sha256'], d.sha(self.base / name))

    def test_shell_entrypoints_resolve_repository_from_another_directory(self):
        fake = self.base / 'tools/python3'
        fake.parent.mkdir()
        fake.write_text('#!' + sys.executable + '\nimport os,sys,json\nprint(json.dumps([os.getcwd(),sys.argv[1:]]))\n')
        fake.chmod(0o700)
        env = os.environ | {'PATH': str(fake.parent) + ':' + os.environ.get('PATH', '/usr/bin:/bin')}
        for script, extra, action in [('compile.sh', [], 'compile'), ('deploy.sh', [], 'deploy'),
                                      ('run.sh', [], 'start'), ('run.sh', ['status'], 'status'),
                                      ('run.sh', ['stop'], 'stop')]:
            result = subprocess.run([str(SCRIPT.parent / 'distributed' / script), *extra],
                                    cwd=self.base, env=env, check=True, capture_output=True, text=True)
            cwd, argv = json.loads(result.stdout)
            self.assertEqual(cwd, str(d.ROOT))
            expected = ['tester/server-bridge/distributed.py', action]
            if script == 'run.sh':
                expected.insert(0, '-u')
            self.assertEqual(argv, expected)

    def test_configuration_rejects_unsafe_paths_and_wrong_wired_destination(self):
        for mutation in ({'remote_directory': '../old'}, {'remote_directory': 'p/../../old'},
                         {'select_ms': 0}, {'worker_port': True}, {'unexpected': 'value'}):
            cfg = self.cfg | mutation
            path = self.base / 'config.json'
            path.write_text(json.dumps(cfg))
            with self.assertRaises(RuntimeError):
                d.load_config(path)
        cfg = copy.deepcopy(self.cfg)
        cfg['nodes']['v2']['ssh'] = 'user@192.168.0.99'
        path.write_text(json.dumps(cfg))
        with self.assertRaisesRegex(RuntimeError, 'wired IPv4'):
            d.load_config(path)

    def test_changed_deployment_is_rejected_before_start(self):
        output = self.base / 'release.tar.gz'
        d.package(self.cfg, output)
        node = self.base / 'node'
        self.extract_package(output, node)
        (node / 'distributed.py').write_text('changed')
        with self.assertRaisesRegex(RuntimeError, 'deployed file changed: distributed.py'):
            d.verify_node(node, 'v1')

    def test_config_change_is_not_silently_ignored(self):
        output = self.base / 'release.tar.gz'
        d.package(self.cfg, output)
        node = self.base / 'node'
        self.extract_package(output, node)
        changed = self.cfg | {'server_url': 'http://192.168.0.82:11451'}
        with self.assertRaisesRegex(RuntimeError, 'Configuration differs from the deployed release'):
            d.verify_node(node, 'v1', hashlib.sha256(d.config_bytes(changed)).hexdigest())

    def test_complete_node_validation_and_token_permission_boundary(self):
        output = self.base / 'release.tar.gz'
        d.package(self.cfg, output)
        node = self.base / 'node'
        self.extract_package(output, node)
        private = node / 'private'
        private.mkdir(mode=0o700)
        for name, value in [('cluster.token', 'a' * 64), ('competition.token', 'token-p0')]:
            (private / name).write_text(value + '\n')
            (private / name).chmod(0o600)
        with patch.object(d.os, 'sched_getaffinity', return_value=set(range(32))), \
                patch.object(d.os, 'confstr', return_value='glibc 2.39'), \
                patch.object(d.platform, 'machine', return_value='x86_64'):
            checked, cfg = d.verify_node(node, 'v1')
            self.assertEqual(checked, node)
            self.assertEqual(cfg, self.cfg)
            d.write_runtime(node, d.runtime_settings('http://192.168.0.81:11451', 'token-p0', None))
            self.assertEqual(d.load_runtime(node)['token'], 'token-p0')
            (private / 'competition.token').chmod(0o644)
            with self.assertRaisesRegex(RuntimeError, 'permissions/owner'):
                d.load_runtime(node)

    def test_preflight_failure_does_not_install_on_any_node(self):
        commands = []
        def remote(cfg, role, command):
            commands.append((role, command))
            if role == 'v3':
                raise RuntimeError('destination exists')
        with patch.object(d, 'remote', remote), patch.object(d, 'upload') as upload:
            with self.assertRaisesRegex(RuntimeError, 'destination exists'):
                d.deploy(self.cfg, self.base / 'unused')
            upload.assert_not_called()
        self.assertEqual([r for r, _ in commands], ['v1', 'v2', 'v3'])
        self.assertTrue(all('mkdir' not in cmd for _, cmd in commands))

    def test_start_failure_cleans_only_newly_started_nodes(self):
        events = []
        def remote_node(cfg, role, action, **kwargs):
            events.append((role, action))
            if action == 'status':
                return subprocess.CompletedProcess([], 0, json.dumps({
                    'running': role == 'v2', 'identity_mismatch': False, 'orphans': []}))
            return subprocess.CompletedProcess([], 0)
        with patch.object(d, 'remote_node', remote_node), patch.object(Path, 'read_text', return_value='a' * 64), \
                patch.object(d, 'spawn_node') as spawn, patch.object(d, 'wait_node_started'), \
                patch.object(d, 'worker_ready', side_effect=RuntimeError('auth failed')):
            with self.assertRaisesRegex(RuntimeError, 'auth failed'):
                d.start_cluster(self.cfg)
        self.assertEqual([call.args[1] for call in spawn.call_args_list], ['v3'])
        self.assertIn(('v3', 'stop'), events)
        self.assertNotIn(('v2', 'stop'), events)
        self.assertNotIn(('v1', 'start'), events)

    def test_foreground_interrupt_releases_lock_and_stops_all_started_nodes(self):
        runners = {role: Mock() for role in d.SPECS}
        for child in runners.values():
            child.poll.return_value = None
        locked = False
        @contextlib.contextmanager
        def startup_lock():
            nonlocal locked
            locked = True
            try:
                yield
            finally:
                locked = False
        stopped = []
        def remote_node(cfg, role, action):
            self.assertFalse(locked, 'foreground wait must not block status/stop')
            self.assertEqual(action, 'stop')
            stopped.append(role)
        with patch.object(d, 'operator_lock', startup_lock), \
                patch.object(d, 'start_cluster', return_value=runners), \
                patch.object(d, 'remote_node', remote_node), \
                patch.object(d.time, 'sleep', side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                d.run_cluster(self.cfg, None)
        self.assertEqual(set(stopped), set(d.SPECS))
        self.assertEqual(stopped[0], 'v1')
        for child in runners.values():
            child.wait.assert_called_once()

    def test_worker_ssh_failure_stops_the_rest_of_the_cluster(self):
        runners = {role: Mock() for role in d.SPECS}
        for child in runners.values():
            child.poll.return_value = None
        runners['v2'].poll.return_value = 255
        with patch.object(d, 'operator_lock', contextlib.nullcontext), \
                patch.object(d, 'start_cluster', return_value=runners), \
                patch.object(d, 'remote_node') as remote:
            with self.assertRaisesRegex(RuntimeError, 'v2: worker/SSH'):
                d.run_cluster(self.cfg, None)
        self.assertEqual({call.args[1] for call in remote.call_args_list}, set(d.SPECS))

    def test_node_streams_output_before_exit_and_cleans_up_on_terminal_signals(self):
        # A dummy runner exercises real pipes, sessions and cleanup without SSH or a solver.
        runner = self.base / 'distributed.py'
        runner.write_text('import sys,time\n'
                          'print("dummy stdout",flush=True)\n'
                          'print("dummy stderr",file=sys.stderr,flush=True)\n'
                          'time.sleep(30)\n')
        harness = ('import importlib.util,sys\n'
                   'from pathlib import Path\n'
                   f'spec=importlib.util.spec_from_file_location("deploy",{str(SCRIPT)!r})\n'
                   'd=importlib.util.module_from_spec(spec); spec.loader.exec_module(d)\n'
                   'd.verify_node=lambda *args: (Path(args[0]), {})\n'
                   'try: d.main()\n'
                   'except KeyboardInterrupt: sys.exit(130)\n')
        for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            with self.subTest(signal=sig):
                child = subprocess.Popen([sys.executable, '-u', '-c', harness, '_node', 'start',
                                          str(self.base), 'v2'], stdout=subprocess.PIPE,
                                         stderr=subprocess.STDOUT, start_new_session=True)
                saved = None
                try:
                    with selectors.DefaultSelector() as selector:
                        selector.register(child.stdout, selectors.EVENT_READ)
                        output = b''
                        deadline = time.monotonic() + 5
                        while b'v2: started PID' not in output:
                            self.assertGreater(deadline, time.monotonic(), output)
                            if selector.select(timeout=.1):
                                data = os.read(child.stdout.fileno(), 4096)
                                self.assertTrue(data, output)
                                output += data
                    self.assertIn(b'dummy stdout', output)
                    self.assertIn(b'dummy stderr', output)
                    self.assertIsNone(child.poll(), 'logs must arrive while the runner is alive')
                    saved = json.loads((self.base / 'runner.json').read_text())
                    status = subprocess.run([sys.executable, str(SCRIPT), '_node', 'status',
                                             str(self.base), 'v2'], check=True, capture_output=True,
                                            text=True, timeout=3)
                    self.assertTrue(json.loads(status.stdout)['running'])
                    child.send_signal(sig)
                    child.wait(timeout=5)
                    current = d.process(saved['pid'])
                    self.assertTrue(current is None or current['state'] == 'Z')
                    self.assertFalse((self.base / 'logs').exists())
                    self.assertFalse(list(self.base.glob('*.log')))
                finally:
                    if child.poll() is None:
                        child.kill()
                    child.wait(timeout=5)
                    child.stdout.close()
                    if saved:
                        d.stop_owned(saved, self.base)

    def test_idempotent_start_does_not_probe_busy_workers(self):
        def remote_node(cfg, role, action, **kwargs):
            return subprocess.CompletedProcess([], 0, json.dumps({
                'running': True, 'identity_mismatch': False, 'orphans': []}))
        with patch.object(d, 'remote_node', remote_node), patch.object(d, 'worker_ready') as probe:
            d.start_cluster(self.cfg)
            probe.assert_not_called()

    def test_deploy_needs_no_competition_url_or_token(self):
        self.cfg['server_url'] = 'http://CHANGE_ME:11451'
        archive = self.base / 'release.tar.gz'
        d.package(self.cfg, archive)
        with patch.object(d, 'remote'), patch.object(d, 'upload') as upload, patch('builtins.input') as prompt:
            d.deploy(self.cfg, archive)
        prompt.assert_not_called()
        self.assertFalse(any('competition.token' in call.args[3] for call in upload.call_args_list))

    def test_connection_change_does_not_require_a_new_deployment(self):
        archive = self.base / 'release.tar.gz'
        d.package(self.cfg, archive)
        node = self.base / 'node'
        self.extract_package(archive, node)
        (node / 'private').mkdir()
        identity = (node / 'manifest.json').read_bytes(), (node / 'node-config.json').read_bytes()
        first = d.runtime_settings('http://192.168.0.81:11451', 'token-p0', None)
        second = d.runtime_settings('http://10.0.0.1:8080', 'a&b#c=d e', 10000, self.base / 'match data')
        d.write_runtime(node, first)
        self.assertEqual(d.load_runtime(node), first)
        d.write_runtime(node, second)
        self.assertEqual(d.load_runtime(node), second)
        self.assertEqual(second['record_directory'], str(self.base / 'match data'))
        self.assertEqual(identity, ((node / 'manifest.json').read_bytes(), (node / 'node-config.json').read_bytes()))

    def test_running_connection_settings_cannot_be_silently_replaced(self):
        first = d.runtime_settings('http://192.168.0.81:11451', 'token-p0', None)
        second = d.runtime_settings('http://10.0.0.1:8080', 'token-p1', None)
        def remote_node(cfg, role, action, **kwargs):
            return subprocess.CompletedProcess([], 0, json.dumps({
                'running': True, 'identity_mismatch': False, 'orphans': []}))
        with patch.object(d, 'remote_node', remote_node), patch.object(d, 'load_runtime', return_value=first), \
                patch.object(d, 'write_runtime') as write:
            with self.assertRaisesRegex(RuntimeError, 'Connection settings differ from the running configuration'):
                d.start_cluster(self.cfg, second)
            write.assert_not_called()

    def test_run_script_passes_url_token_selection_without_shell_splitting(self):
        fake = self.base / 'tools/python3'
        fake.parent.mkdir()
        fake.write_text('#!' + sys.executable + '\nimport os,json\nprint(json.dumps({k:os.environ[k] for k in ["SERVER","TOKEN","SELECT_MS","RECORD_DIR"]}))\n')
        fake.chmod(0o700)
        env = os.environ | {'PATH': str(fake.parent) + ':' + os.environ.get('PATH', '/usr/bin:/bin'),
                            'SERVER': 'http://10.0.0.1:8080', 'TOKEN': 'a&b#c=d e', 'SELECT_MS': '10000',
                            'RECORD_DIR': str(self.base / 'match data')}
        result = subprocess.run([str(SCRIPT.parent / 'distributed/run.sh')], cwd=self.base, env=env,
                                check=True, capture_output=True, text=True)
        self.assertEqual(json.loads(result.stdout), {k:env[k] for k in ('SERVER', 'TOKEN', 'SELECT_MS', 'RECORD_DIR')})

    def test_single_run_script_passes_recording_directory_without_shell_splitting(self):
        bridge = self.base / 'fake-bridge'
        bridge.write_text('#!' + sys.executable + '\nimport json,sys\nprint(json.dumps(sys.argv[1:]))\nsys.exit(2)\n')
        bridge.chmod(0o700)
        solver = Path(self.cfg['solver_binary'])
        solver.chmod(0o700)
        for directory in (str(self.base / 'match data'), None):
            env = {k:v for k,v in os.environ.items() if k != 'RECORD_DIR'}
            env.update(BRIDGE=str(bridge), SOLVER=str(solver))
            if directory is not None:
                env['RECORD_DIR'] = directory
            result = subprocess.run([str(SCRIPT.parent / 'run.sh')], cwd=self.base, env=env,
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 2, result.stderr)
            argv = json.loads(result.stdout.splitlines()[0])
            expected = directory or str(d.ROOT / '.server/submissions')
            self.assertEqual(argv[argv.index('--record-dir') + 1], expected)

    def test_worker_authentication_checks_offsets_with_fragmented_frames(self):
        payload = json.dumps({'Ready': {'version': 1, 'total_threads': 16, 'offset': 31}}).encode()
        class Wire:
            def __init__(self):
                self.data = len(payload).to_bytes(4, 'big') + payload
                self.sent = b''
            def __enter__(self): return self
            def __exit__(self, *args): pass
            def sendall(self, data): self.sent += data
            def recv(self, n):
                part, self.data = self.data[:min(n, 2)], self.data[min(n, 2):]
                return part
        wire = Wire()
        with patch.object(d.socket, 'create_connection', return_value=wire):
            d.worker_ready(self.cfg, 'v2', 'a' * 64)
        message = json.loads(wire.sent[4:])
        self.assertEqual(message, {'Authenticate': {'version': 1, 'token': 'a' * 64}})
        self.assertNotIn('Solve', message)


@unittest.skipUnless(sys.platform == 'linux' and hasattr(os, 'pidfd_open'), 'Linux pidfd tests')
class OwnedProcessTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='distributed-owned-test-')
        self.base = Path(self.temp.name)
        self.children = []

    def spawn(self, code):
        child = subprocess.Popen([sys.executable, '-c', code], cwd=self.base, start_new_session=True,
                                 stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.children.append(child)
        return child

    def wait_file(self, name):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if (self.base / name).exists():
                return (self.base / name).read_text()
            time.sleep(.01)
        self.fail('dummy process did not become ready')

    def tearDown(self):
        for child in self.children:
            if child.poll() is None:
                child.kill()
            child.wait(timeout=5)
        self.temp.cleanup()

    def test_stop_targets_its_tree_and_preserves_another_session(self):
        outsider = self.spawn('import time; time.sleep(30)')
        parent = self.spawn('import subprocess,sys,time,pathlib; '
                            'c=subprocess.Popen([sys.executable,"-c","import time; time.sleep(30)"]); '
                            'pathlib.Path("child.pid").write_text(str(c.pid)); time.sleep(30)')
        descendant = int(self.wait_file('child.pid'))
        saved = d.process(parent.pid)
        stopped = d.stop_owned(saved, self.base)
        parent.wait(timeout=5)
        self.assertIn(parent.pid, stopped)
        self.assertIn(descendant, stopped)
        self.assertIsNone(outsider.poll())
        current = d.process(descendant)
        self.assertTrue(current is None or current['state'] == 'Z')

    def test_reused_pid_record_never_signals_the_process(self):
        child = self.spawn('import time; time.sleep(30)')
        saved = d.process(child.pid)
        saved['start'] -= 1
        with self.assertRaisesRegex(RuntimeError, 'identity changed'):
            d.stop_owned(saved, self.base)
        self.assertIsNone(child.poll())

    def test_orphans_are_stopped_after_the_supervisor_exits(self):
        parent = self.spawn('import subprocess,sys,time,pathlib; '
                            'c=subprocess.Popen([sys.executable,"-c","import time; time.sleep(30)"]); '
                            'pathlib.Path("orphan.pid").write_text(str(c.pid)); time.sleep(.2)')
        descendant = int(self.wait_file('orphan.pid'))
        saved = d.process(parent.pid)
        parent.wait(timeout=5)
        stopped = d.stop_owned(saved, self.base)
        self.assertIn(descendant, stopped)
        current = d.process(descendant)
        self.assertTrue(current is None or current['state'] == 'Z')

    def test_unresponsive_runner_is_killed_and_reaped(self):
        child = self.spawn('import signal,time,pathlib; signal.signal(signal.SIGTERM,signal.SIG_IGN); '
                           'pathlib.Path("ready").write_text("yes"); time.sleep(30)')
        self.wait_file('ready')
        d.stop_owned(d.process(child.pid), self.base)
        self.assertEqual(child.wait(timeout=5), -9)


if __name__ == '__main__':
    unittest.main()
