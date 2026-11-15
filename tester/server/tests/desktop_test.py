"""Independent launch state for the match server's desktop entry. Build target/release/server first."""
import importlib.util
from pathlib import Path
import unittest
spec = importlib.util.spec_from_file_location("solo_desktop", Path(__file__).resolve().parents[3] / "tester/solo/tests/desktop_test.py")
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)

class ServerDesktopTests(base.DesktopTests):
    APP = "server"
    SOURCE = "tester/server"
    COMMAND = "server"
    LEGACY_COMMAND = "procon37-server"
    STORAGE = ".server"
    IDENTITY = "procon37-server-launcher-v1"

    def check_config(self, config):
        # The GUI stays usable for history even if the match window port is taken.
        self.assertEqual(config["serverProgram"], "tester/server/procon-server")
        self.assertEqual(config["players"], 1)

if __name__ == "__main__":
    unittest.main(verbosity=2)
