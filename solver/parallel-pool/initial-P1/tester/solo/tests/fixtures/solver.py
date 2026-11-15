"""Process-boundary fixture only. The tester and GUI do not require Python."""
import json
import os
import subprocess
import sys
import threading
import time

mode = sys.argv[1]
def track_process(delta):
    import fcntl
    with open("parallel.json", "a+") as f:
        fcntl.flock(f, fcntl.LOCK_EX)
        f.seek(0)
        data = json.loads(f.read() or '{"active": 0, "peak": 0}')
        data["active"] += delta
        data["peak"] = max(data["peak"], data["active"])
        f.seek(0)
        f.truncate()
        json.dump(data, f)
        f.flush()

if mode == "cancel":
    child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
    with open("started.json", "w") as f:
        json.dump([os.getpid(), child.pid], f)
    time.sleep(30)

count = 0
for line in sys.stdin:
    req = json.loads(line)
    if mode == "exit":
        sys.exit(7)
    if mode == "late" and req["requestId"] == 0:
        time.sleep(0.35)
    if mode == "malformed":
        print("not-json", flush=True)
        continue
    if mode == "oversized":
        print("x" * (1024 * 1024 + 10), flush=True)
        continue
    if req["phase"] == "select":
        count = len(req["problem"]["agents"])
        if mode == "schedule":
            days = len(req["problem"]["daySteps"])
            track_process(1)
            time.sleep(0.2)
        answer = [9] if mode == "invalid" else [0] * count
        if mode == "valid":
            # Concurrent workers may log freely; one writer owns protocol stdout.
            workers = [threading.Thread(target=lambda: os.write(2, b"worker log\n" * 10000)) for _ in range(4)]
            for worker in workers:
                worker.start()
            for worker in workers:
                worker.join()
    else:
        answer = [[-req["steps"]] for _ in range(count)]
        if mode == "schedule" and req["requestId"] == days:
            track_process(-1)
        if mode == "invalid" and req["requestId"] == 1:
            answer[0] = [6]
        if mode == "invalid" and req["requestId"] == 2:
            assert req["previousAccepted"] is False
    print(json.dumps({"requestId": req["requestId"], "answer": answer}), flush=True)
