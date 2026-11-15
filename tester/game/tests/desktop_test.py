"""Independent launch state and coexisting tester/game desktop entries."""
import importlib.util
from pathlib import Path
import shutil
import unittest
spec = importlib.util.spec_from_file_location("solo_desktop", Path(__file__).resolve().parents[3] / "tester/solo/tests/desktop_test.py")
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)

class GameDesktopTests(base.DesktopTests):
    APP = "game"
    SOURCE = "tester/game"
    COMMAND = "game"
    LEGACY_COMMAND = "procon37-game"
    STORAGE = ".game"
    IDENTITY = "procon37-game-launcher-v1"

    def test_both_launchers_coexist_and_stop_independently(self):
        scripts = self.repo / "tester/solo/desktop"
        shutil.copytree(base.REPO / "tester/solo/desktop", scripts)
        (self.repo / "target/release/tester").symlink_to(base.REPO / "target/release/tester")
        self.command("bash", str(scripts / "setup.sh"), "--no-build", "--bin-dir", str(self.bin), "--data-dir", str(self.data))
        self.addCleanup(lambda: self.command("bash", str(scripts / "launch.sh"), "--stop", check=False))
        game_url = self.launch("--port", "0", "--no-open").stdout.strip()
        solo_url = self.command(str(self.bin / "solo"), "--port", "0", "--no-open").stdout.strip()
        self.assertNotEqual(game_url, solo_url)
        for url, identity, storage in [(game_url,self.IDENTITY,".game"),(solo_url,"procon37-launcher-v1",".tester")]:
            with base.HTTP.open(url + "api/launcher", timeout=2) as response:
                lines = response.read().decode().splitlines()
                self.assertEqual(lines[0], identity)
                self.assertEqual(lines[2], str(self.repo / storage / "runs"))
        self.launch("--stop")
        with base.HTTP.open(solo_url + "api/config", timeout=2) as response:
            self.assertNotIn("mode", base.json.load(response))
        self.install("--uninstall")
        self.assertTrue((self.bin / "solo").exists())
        self.assertTrue((self.data / "applications/solo.desktop").exists())

if __name__ == "__main__":
    unittest.main(verbosity=2)
