import copy
import hashlib
import json
from pathlib import Path
import shlex
import tempfile
import unittest

import switch


class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.config = {'nodes': {}}
        record = {'nodes': {}}
        self.original = {}
        for host, cpus in [('v1', '0-31'), ('v2', '0-15'), ('v3', '0-31')]:
            directory = '/home/test/' + host
            script = '#!/bin/sh\nexec env -i PATH=/usr/bin:/bin taskset -c ' + cpus + ' ' + directory + '/distributed-solver --threads 16 "$@"\n'
            self.original[host] = script
            (self.base / host).mkdir()
            (self.base / host / 'launcher').write_text(script)
            node = {'home': '/home/test', 'directory': directory, 'cpus': cpus}
            self.config['nodes'][host] = copy.deepcopy(node)
            record['nodes'][host] = dict(node, files={'launcher': hashlib.sha256(script.encode()).hexdigest()})
        (self.base / 'deployment.json').write_text(json.dumps(record))

    def test_modes_and_restore_do_not_carry_previous_arguments(self):
        self.config['nodes']['v1']['files'] = {'launcher': 'previous-elite-launcher'}
        for mode in ('on', 'off'):
            scripts = switch.launchers(self.config, mode, self.base)
            for host, script in scripts.items():
                argv = shlex.split(script.splitlines()[1])
                self.assertEqual(argv[-3:], ['--cross-elite', mode, '$@'])
                self.assertEqual(argv.count('--cross-elite'), 1)
                self.assertEqual(argv[:-3], shlex.split(self.original[host].splitlines()[1])[:-1])
        self.assertEqual(switch.launchers(self.config, None, self.base), self.original)

    def test_refuses_changed_cpu_or_directory(self):
        for field in ('cpus', 'directory'):
            config = copy.deepcopy(self.config)
            config['nodes']['v2'][field] = 'different'
            with self.assertRaises(Exception):
                switch.launchers(config, 'on', self.base)

    def test_refuses_tampered_original(self):
        (self.base / 'v3' / 'launcher').write_text('#!/bin/sh\nexit 0\n')
        with self.assertRaises(Exception):
            switch.launchers(self.config, 'off', self.base)


if __name__ == '__main__':
    unittest.main()
