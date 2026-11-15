"""Exercise opt-in checkpoints against HTTP + JSONL children without real teams.

Usage: python3 tester/server-bridge/tests/check_scheduled_submissions.py target/debug/server-bridge
"""
import http.server
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from urllib.parse import urlsplit

BRIDGE = None
OFFSETS = [300, 900, 1500, 2100, 2700, 3300]  # 5,15,...,55 seconds scaled by .06


class ScheduledSubmissions(unittest.TestCase):
    def run_case(self, *, streaming=True, broken=False, reject_final=False, short=False, late_state=False, timing_headers=False, sequence=False):
        days = 2 if reject_final or sequence else 1
        phase_seconds = 2 if short else 4
        state = {"posts": [], "observations": [], "first_setting": None}
        problem = {
            "startsAt": 0, "daySeconds": [phase_seconds] * days, "daySteps": [2] * days,
            "map": {"width": 2, "height": 2, "cells": [[0, 0], [0, 0]]},
            "spots": [{"brand": 0, "pos": 3, "stocks": 1}], "agents": [0],
            "fuelLimits": 10, "players": 1, "busyThreshold": 1, "jammedThreshold": 2,
        }

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def reply(self, status, value):
                raw = json.dumps(value).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(raw)))
                self.send_header("Procon37-Match-Id", "scheduled-test")
                if timing_headers:
                    self.send_header("Procon37-Select-Started-At-Ms", str(int(state["first_setting"] * 1000)))
                    self.send_header("Procon37-Select-Deadline-Ms", str(int((state["first_setting"] + phase_seconds) * 1000)))
                self.end_headers()
                self.wfile.write(raw)

            def do_GET(self):
                now = time.time()
                if urlsplit(self.path).path == "/setting":
                    if state["first_setting"] is None:
                        state["first_setting"] = now
                        state["day_start"] = math.ceil(now + phase_seconds)
                    if timing_headers:
                        time.sleep(.18)
                    self.reply(200, problem)
                elif now < state["day_start"]:
                    self.reply(403, {"message": "match has not started"})
                else:
                    day = int((now - state["day_start"]) // phase_seconds)
                    if day >= days:
                        self.reply(403, {"message": "match has ended"})
                    else:
                        # State acquisition differs from the actual phase start. Targets must not shift.
                        delay = max(0, state["day_start"] + day * phase_seconds + (3.5 if late_state else .12) - time.time())
                        time.sleep(delay)
                        state["observations"].append((day, time.time()))
                        self.reply(200, {"endsAt": state["day_start"] + (day + 1) * phase_seconds,
                            "day": day, "agents": [{"kind": 0, "pos": 0, "fuel": 10}],
                            "others": [], "traffics": []})

            def do_POST(self):
                value = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                now = time.time()
                day = None if urlsplit(self.path).path == "/agent" else int((now - state["day_start"]) // phase_seconds)
                rejected = reject_final and urlsplit(self.path).path == "/" and value == [[-1, -1]]
                state["posts"].append((urlsplit(self.path).path, day, now, value, not rejected))
                self.reply(403 if rejected else 200, {"revision": -1 if rejected else 1})

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            solver = root / "solver.py"
            solver.write_text('''import json, sys, time
for line in sys.stdin:
    request = json.loads(line)
    with open("requests.jsonl", "a") as f: f.write(line)
    if BROKEN:
        print("not-json", flush=True)
        continue
    rid = request["requestId"]
    good = [0] if request["phase"] == "select" else [[-2]]
    if STREAMING:
        print("not-json", flush=True)
        print(json.dumps({"requestId": rid + 100, "answer": good, "final": False}), flush=True)
        if rid > 0:
            print(json.dumps({"requestId": rid - 1, "answer": good, "final": False}), flush=True)
        print(json.dumps({"requestId": rid, "answer": {"invalid": True}, "final": False}), flush=True)
        print(json.dumps({"requestId": rid, "answer": good, "final": False}), flush=True)
        if SEQUENCE:
            other = [1] if request["phase"] == "select" else [[-1,-1]]
            for delay, answer, metadata, final in [(0.45, good, 1, False), (0.6, other, 2, False),
                    (0.5, other, 3, False), (0.7, good, 4, True)]:
                time.sleep(delay)
                print(json.dumps({"requestId": rid, "answer": answer, "emissionMetadata": metadata, "final": final}), flush=True)
            continue
        time.sleep(1.7)
        final = [[-1,-1]] if REJECT and request["phase"] == "plan" else good
        print(json.dumps({"requestId": rid, "answer": final, "final": True}), flush=True)
    else:
        print(json.dumps({"requestId": rid, "answer": good}), flush=True)
'''.replace("BROKEN", repr(broken)).replace("STREAMING", repr(streaming)).replace("REJECT", repr(reject_final)).replace("SEQUENCE", repr(sequence)))
            with http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
                thread = threading.Thread(target=server.serve_forever, daemon=True)
                thread.start()
                args = [str(BRIDGE), "--server", f"http://127.0.0.1:{server.server_port}",
                    "--token", "test", "--solver", sys.executable, "--cwd", str(root),
                    "--record-dir", str(root / "records"), "--select-ms", str(phase_seconds * 1000),
                    "--submit-at-ms", ",".join(map(str, OFFSETS if streaming else [3300]))]
                if streaming:
                    args.append("--stream-responses")
                try:
                    result = subprocess.run(args + ["--", str(solver)], capture_output=True, text=True, timeout=25)
                finally:
                    server.shutdown()
                    thread.join()
            self.assertEqual(result.returncode, 0, result.stderr)
            requests = [json.loads(line) for line in (root / "requests.jsonl").read_text().splitlines()]
            self.assertEqual([r["requestId"] for r in requests], list(range(days + 1)), result.stderr)
            saved = next((root / "records").iterdir())
            metadata = json.loads((saved / "metadata.json").read_text())
            offsets = OFFSETS if streaming else [3300]
            events = [json.loads(line) for line in (saved / "http.jsonl").read_text().splitlines()]
            for phase in [None] + list(range(days)):
                posts = [p for p in state["posts"] if p[1] == phase]
                expected = offsets
                if short:
                    expected = [offset for offset in offsets if offset < 1500] + [1500]
                if late_state and phase is not None:
                    self.assertEqual(len(posts), 1, result.stderr)
                    acquired = next(at for day, at in state["observations"] if day == phase)
                    self.assertLess(posts[0][2] - acquired, .18, result.stderr)
                    self.assertIn("checkpoint_recovery", result.stderr)
                    continue
                checkpoints = expected
                request_id = 0 if phase is None else phase + 1
                checkpoints_logged = [event for event in events if event.get("requestId") == request_id
                    and event["event"] in ("scheduledSubmission", "scheduledSubmissionSkipped")]
                self.assertEqual(len(checkpoints_logged), len(checkpoints), result.stderr)
                if sequence:
                    expected = [checkpoints[i] for i in (0, 2, 4)]
                    a = [0] if phase is None else [[-2]]
                    b = [1] if phase is None else [[-1, -1]]
                    self.assertEqual([post[3] for post in posts], [a, b, a], result.stderr)
                elif reject_final and phase is not None:
                    expected = [checkpoints[0], checkpoints[3]]
                else:
                    expected = checkpoints[:1]
                self.assertEqual(len(posts), len(expected), result.stderr)
                skipped = [event for event in checkpoints_logged if event["event"] == "scheduledSubmissionSkipped"]
                self.assertEqual(len(skipped), len(checkpoints) - len(posts), result.stderr)
                self.assertTrue(all(event["reason"] == "unchanged_answer" for event in skipped))
                selection_anchor = metadata["selectionStartedAtMs"] if timing_headers else metadata["receivedAtMs"]
                anchor = selection_anchor / 1000 if phase is None else state["day_start"] + phase * phase_seconds
                for post, offset in zip(posts, expected):
                    actual = (post[2] - anchor) * 1000
                    self.assertGreaterEqual(actual, offset - 15, result.stderr)
                    self.assertLess(actual, offset + 180, result.stderr)
            max_budget = 1000 if short else 2800
            self.assertLessEqual(requests[0]["timeLimitMs"], max_budget)
            self.assertGreater(requests[0]["timeLimitMs"], max_budget - (400 if timing_headers else 200))
            if streaming and not broken:
                self.assertIn("reason=wrong_request", result.stderr)
                self.assertIn("reason=invalid_answer", result.stderr)
            if broken:
                self.assertTrue(all(post[3] == ([0] if post[0] == "/agent" else [[-2]]) for post in state["posts"]))
                self.assertTrue(all(not request.get("previousAccepted", False) for request in requests))
            if reject_final:
                self.assertEqual(requests[2]["previousAnswer"], [[-2]])
                self.assertTrue(requests[2]["previousAccepted"])
                self.assertTrue(any(not p[4] for p in state["posts"]))
            if not streaming:
                self.assertTrue(all("previousAnswer" not in request for request in requests))
            self.assertIn("solver_exited", result.stderr)
            return result

    def test_six_checkpoints_use_phase_anchor_and_skip_unchanged_bad_or_stale_lines(self):
        self.run_case()

    def test_fixed_solver_legacy_reply_uses_one_final_checkpoint(self):
        self.run_case(streaming=False)

    def test_failed_solver_submits_safe_fallback_once_and_skips_unchanged_checkpoints(self):
        self.run_case(broken=True)

    def test_rejected_final_keeps_accepted_interim_and_passes_previous_answer(self):
        self.run_case(reject_final=True)

    def test_shortened_fixed_and_streaming_phases_share_safe_final_time(self):
        self.run_case(short=True, streaming=False)
        self.run_case(short=True)

    def test_elapsed_final_checkpoint_recovers_immediately_while_phase_open(self):
        self.run_case(late_state=True, streaming=False)

    def test_exact_selection_headers_prevent_delayed_problem_from_shifting_targets(self):
        self.run_case(timing_headers=True)

    def test_answer_only_dedup_repeated_changed_reverted_and_next_day(self):
        self.run_case(sequence=True)


if __name__ == "__main__":
    BRIDGE = Path(sys.argv.pop(1)).resolve()
    unittest.main(verbosity=2)
