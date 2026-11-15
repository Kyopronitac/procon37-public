"""Black-box match check; never reads sample implementations."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[3]


class SampleTests(unittest.TestCase):
    def test_samples_complete_all_formats_with_shared_traffic(self):
        with tempfile.TemporaryDirectory(prefix="procon37-samples-") as output_dir:
            result = subprocess.run(
                [str(REPO / "target/release/game"), "run", "3", "--no-build",
                 "--time-limit-ms", "2000", "--jobs", "1",
                 "--label", "サンプル置換・通信と共通道路の回帰確認3件",
                 "--output-dir", output_dir, "--", "37"],
                cwd=REPO, capture_output=True, text=True, timeout=180,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            manifests = list(Path(output_dir).glob("*/run.json"))
            self.assertEqual(len(manifests), 1)
            run = json.loads(manifests[0].read_text())
            self.assertEqual(run["completed"], 3)
            self.assertEqual(run["status"], "completed")
            for case in run["cases"]:
                self.assertEqual(len(case["players"]), 5)
                for player in case["players"]:
                    with self.subTest(seed=case["seed"], player=player["name"]):
                        self.assertEqual(player["status"], "AC")
                        self.assertEqual(player["warnings"], 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
