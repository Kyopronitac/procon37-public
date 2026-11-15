"""Linux launcher integration tests. Build target/release/tester first."""
import concurrent.futures
import http.server
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import threading
import time
import unittest
import urllib.request

REPO = Path(__file__).resolve().parents[3]
HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}))


class DesktopTests(unittest.TestCase):
    APP = "tester"
    SOURCE = "tester/solo"
    COMMAND = "solo"
    LEGACY_COMMAND = "procon37"
    STORAGE = ".tester"
    IDENTITY = "procon37-launcher-v1"
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="procon37-desktop-")
        self.base = Path(self.temp.name)
        self.repo = self.base / "repo ' space $value `literal` % back\\slash"
        self.scripts = self.repo / self.SOURCE / "desktop"
        shutil.copytree(REPO / self.SOURCE / "desktop", self.scripts)
        shutil.copytree(REPO / "common/desktop", self.repo / "common/desktop")
        binary = self.repo / f"target/release/{self.APP}"
        binary.parent.mkdir(parents=True)
        binary.symlink_to(REPO / f"target/release/{self.APP}")
        self.bin = self.base / 'bin "quoted" $value `literal` % back\\slash'
        self.data = self.base / "data directory"
        self.stubs = self.base / "stubs"
        self.stubs.mkdir()
        (self.stubs / "cargo").write_text('#!/bin/sh\nprintf "build\\n" >> "$PROCON37_TEST_BUILDS"\n')
        (self.stubs / "xdg-open").write_text('#!/bin/sh\nprintf "%s\\n" "$1" >> "$PROCON37_TEST_OPENS"\n')
        (self.stubs / "notify-send").write_text('#!/bin/sh\nexit 0\n')
        for stub in self.stubs.iterdir():
            stub.chmod(0o755)
        self.env = {**os.environ, "PATH":f"{self.stubs}:{os.environ['PATH']}",
                    "PROCON37_TEST_BUILDS":str(self.base / "builds"),
                    "PROCON37_TEST_OPENS":str(self.base / "opens")}
        self.env.pop("WSL_DISTRO_NAME", None)
        self.install()

    def command(self, *args, check=True):
        return subprocess.run(args, cwd="/tmp", env=self.env, check=check,
                              capture_output=True, text=True, timeout=25)

    def install(self, *extra, check=True):
        return self.command("bash", str(self.scripts / "setup.sh"), "--no-build",
                            "--bin-dir", str(self.bin), "--data-dir", str(self.data),
                            *extra, check=check)

    def launch(self, *args, check=True):
        return self.command(str(self.bin / self.COMMAND), *args, check=check)

    def tearDown(self):
        self.command("bash", str(self.scripts / "launch.sh"), "--stop", check=False)
        self.temp.cleanup()

    def check_config(self, config):
        self.assertEqual(config["solver"]["cwd"], str(self.repo))

    def test_terminal_desktop_concurrent_launch_restart_and_uninstall(self):
        self.assertIn("--restart", self.launch("--help").stdout)
        desktop = self.data / f"applications/{self.COMMAND}.desktop"
        if shutil.which("desktop-file-validate"):
            self.command("desktop-file-validate", str(desktop))
        with concurrent.futures.ThreadPoolExecutor(2) as pool:
            calls = [pool.submit(self.launch, "--port", "0", "--no-open") for _ in range(2)]
            urls = [call.result().stdout.strip() for call in calls]
        self.assertEqual(urls[0], urls[1])
        url = urls[0]
        with HTTP.open(url + "api/config", timeout=2) as response:
            self.check_config(json.load(response))
        self.assertEqual((self.base / "builds").read_text().splitlines(), ["build"])
        self.launch()
        if shutil.which("gio"):
            self.command("gio", "launch", str(desktop))
        count = 2 if shutil.which("gio") else 1
        for _ in range(40):
            opened = (self.base / "opens").read_text().splitlines() if (self.base / "opens").exists() else []
            if len(opened) == count:
                break
            time.sleep(0.05)
        self.assertEqual(opened, [url] * count)
        self.launch("--restart", "--no-open", "--port", "0")
        self.assertEqual(len((self.base / "builds").read_text().splitlines()), 2)
        self.launch("--stop")
        self.launch("--stop")
        # The usual launch also works if 8080 belongs to another workspace.
        default_url = self.launch("--no-open").stdout.strip()
        with HTTP.open(default_url + "api/launcher", timeout=2) as response:
            self.assertEqual(response.read().decode().splitlines()[1], str(self.repo))
        self.launch("--stop")
        self.install()  # Reinstall is idempotent.
        keep = self.repo / f"{self.STORAGE}/runs/keep.txt"
        keep.write_text("saved result")
        self.install("--uninstall")
        self.assertFalse((self.bin / self.COMMAND).exists())
        self.assertFalse(desktop.exists())
        self.assertEqual(keep.read_text(), "saved result")

    def test_foreign_service_is_not_reused_or_stopped(self):
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b"unrelated application")

            def log_message(self, *args):
                pass

        with http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
            worker = threading.Thread(target=server.serve_forever, daemon=True)
            worker.start()
            try:
                port = str(server.server_port)
                result = self.launch("--port", port, "--no-open", check=False)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((self.base / "opens").exists())
                self.launch("--stop", "--port", port)
                with HTTP.open(f"http://127.0.0.1:{port}/", timeout=2) as response:
                    self.assertEqual(response.read(), b"unrelated application")
            finally:
                server.shutdown()
                worker.join()

    def test_restart_on_different_port_stops_cached_server(self):
        old_url = self.launch("--port", "0", "--no-open").stdout.strip()
        self.addCleanup(lambda: self.shutdown_url(old_url))
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            new_port = reservation.getsockname()[1]
        new_url = self.launch("--restart", "--port", str(new_port), "--no-open").stdout.strip()
        self.assertEqual(new_url, f"http://127.0.0.1:{new_port}/")
        with self.assertRaises(urllib.error.URLError):
            HTTP.open(old_url + "api/launcher", timeout=2)
        with HTTP.open(new_url + "api/launcher", timeout=2) as response:
            self.assertEqual(response.read().decode().splitlines()[1], str(self.repo))
        self.assertEqual((self.repo / f"{self.STORAGE}/launcher/port").read_text().strip(), str(new_port))
        self.assertEqual(len((self.base / "builds").read_text().splitlines()), 2)

    def shutdown_url(self, url):
        try:
            with HTTP.open(urllib.request.Request(url + "api/shutdown", method="POST"), timeout=2):
                pass
        except urllib.error.URLError:
            pass

    def test_icon_name_does_not_collide_with_theme_icons(self):
        desktop = (self.data / f"applications/{self.COMMAND}.desktop").read_text()
        self.assertIn(f"Icon=hexaudon-{self.COMMAND}\n", desktop)
        icons = self.data / "icons/hicolor/scalable/apps"
        self.assertTrue((icons / f"hexaudon-{self.COMMAND}.svg").exists())
        # Earlier versions installed the icon under the command name; only managed files are removed.
        shutil.copyfile(icons / f"hexaudon-{self.COMMAND}.svg", icons / f"{self.COMMAND}.svg")
        self.install()
        self.assertFalse((icons / f"{self.COMMAND}.svg").exists())
        (icons / f"{self.COMMAND}.svg").write_text("<svg>theme or user icon</svg>")
        self.install("--uninstall")
        self.assertFalse((icons / f"hexaudon-{self.COMMAND}.svg").exists())
        self.assertEqual((icons / f"{self.COMMAND}.svg").read_text(), "<svg>theme or user icon</svg>")

    def test_installer_preserves_unrelated_files(self):
        launcher = self.bin / self.COMMAND
        launcher.write_text("user command\n")
        for args in [(), ("--uninstall",)]:
            self.assertNotEqual(self.install(*args, check=False).returncode, 0)
            self.assertEqual(launcher.read_text(), "user command\n")

    def test_install_migrates_legacy_names_for_this_checkout(self):
        old_launcher = self.bin / self.LEGACY_COMMAND
        old_desktop = self.data / f"applications/{self.LEGACY_COMMAND}.desktop"
        old_icon = self.data / f"icons/hicolor/scalable/apps/{self.LEGACY_COMMAND}.svg"
        for source in [self.scripts, self.repo / self.APP / "desktop"]:
            quoted = subprocess.check_output(
                ["bash", "-c", 'printf \'%q\' "$1"', "_", str(source / "launch.sh")],
                text=True)
            old_launcher.write_text(f'#!/bin/bash\n# {self.LEGACY_COMMAND}-managed-launcher-v1\nexec /bin/bash {quoted} "$@"\n')
            shutil.copyfile(self.data / f"applications/{self.COMMAND}.desktop", old_desktop)
            shutil.copyfile(self.data / f"icons/hicolor/scalable/apps/hexaudon-{self.COMMAND}.svg", old_icon)
            self.install()
            self.assertFalse(old_launcher.exists())
            self.assertFalse(old_desktop.exists())
            self.assertFalse(old_icon.exists())
            self.assertIn(self.COMMAND + ":", self.launch("--help").stdout)

    def test_install_preserves_unrelated_legacy_registration(self):
        old_launcher = self.bin / self.LEGACY_COMMAND
        for text in ["user command\n",
                     f'# {self.LEGACY_COMMAND}-managed-launcher-v1\nexec /bin/bash /another/repo/launch.sh "$@"\n']:
            old_launcher.write_text(text)
            self.install()
            self.assertEqual(old_launcher.read_text(), text)


if __name__ == "__main__":
    unittest.main(verbosity=2)
