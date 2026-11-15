#!/usr/bin/env python3
"""One finite idle v2 owner probe. No existing solver process is touched.

Requires a fresh deployment with deployment-manifest.json mapping every file to
SHA-256, including owner-probe and the C009b source/setup script.
C009b is currently unbuilt/unverified; prior C009 results do not validate it.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import pwd
import re
import signal
import subprocess
import sys
import time


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def cpuset(pid, tid=None):
    base = Path(f"/proc/{pid}") if tid is None else Path(f"/proc/{pid}/task/{tid}")
    for line in (base / "cgroup").read_text().splitlines():
        _, controllers, group = line.split(":", 2)
        if "cpuset" in controllers.split(","):
            return group
    return None


def inspect_process(pid):
    base = Path(f"/proc/{pid}")
    try:
        threads, children = [], set()
        for task in base.joinpath("task").iterdir():
            try:
                fields = dict(line.split(":", 1) for line in task.joinpath("status").read_text().splitlines() if ":" in line)
                stat = task.joinpath("stat").read_text().rsplit(")", 1)[1].split()
                tid = int(task.name)
                threads.append({"tid": tid, "name": fields["Name"].strip(), "allowed": fields["Cpus_allowed_list"].strip(),
                                "uid": int(fields["Uid"].split()[1]), "cpuset": cpuset(pid, tid), "ticks": int(stat[11])+int(stat[12])})
                children.update(map(int, task.joinpath("children").read_text().split()))
            except (FileNotFoundError, ProcessLookupError):
                pass
        return threads, children
    except (FileNotFoundError, ProcessLookupError):
        return [], set()


def owned_children():
    children = set()
    for task in Path("/proc/self/task").iterdir():
        children.update(map(int, task.joinpath("children").read_text().split()))
    return children


def kill_adopted(pid):
    try:
        descriptor = os.pidfd_open(pid)
    except ProcessLookupError:
        return False
    try:
        fields = dict(line.split(":", 1) for line in Path(f"/proc/{pid}/status").read_text().splitlines() if ":" in line)
        if int(fields["PPid"]) != os.getpid() or fields["State"].strip().startswith("Z"):
            return False
        signal.pidfd_send_signal(descriptor, signal.SIGKILL)
        return True
    except (FileNotFoundError, ProcessLookupError):
        return False
    finally:
        os.close(descriptor)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--deployment", type=Path, required=True)
    parser.add_argument("--label", required=True)
    args = parser.parse_args()
    if os.geteuid() != 0 or Path("/proc/sys/kernel/hostname").read_text().strip().lower() != "voidlinux-2":
        parser.error("root on VOIDLINUX-2 required")
    if not re.fullmatch(r"[a-zA-Z0-9_-]{1,32}", args.label):
        parser.error("invalid fresh label")
    account = pwd.getpwnam("voidlinux-2")
    deployment = args.deployment.resolve()
    if deployment.parent != Path(account.pw_dir)/"procon37-distributed" or not deployment.name.startswith("c009b-owner-probe-"):
        parser.error("dedicated deployment path required")
    manifest = json.loads(deployment.joinpath("deployment-manifest.json").read_text())
    for name, expected in manifest["sha256"].items():
        path = (deployment/name).resolve()
        if not path.is_relative_to(deployment) or digest(path) != expected:
            raise RuntimeError(f"deployment digest mismatch: {name}")
    binary = deployment / "owner-probe"
    setup = deployment / "solver/distributed-candidates/c009b-gpu-owner/tools/gpu_cpuset.py"
    if not all(str(path.relative_to(deployment)) in manifest["sha256"] for path in [binary, setup]):
        raise RuntimeError("executable/setup must be explicitly hashed")
    output = deployment / ("results-"+args.label)
    output.mkdir()  # Never overwrite a prior run.
    name = "procon37-c009-v2-"+args.label
    group = Path("/sys/fs/cgroup/cpuset")/name
    began = time.monotonic()
    record = {"label": args.label, "host": os.uname().nodename, "scope": "idle owner builder, not active search or match",
              "startedUnix": time.time(), "cpuClockTicksPerSecond": os.sysconf("SC_CLK_TCK"),
              "binarySha256": digest(binary), "controllerSha256": digest(Path(__file__)), "group": str(group),
              "monitorDeadlineSeconds": 45, "outerDeadlineSeconds": 75, "violations": [], "forcedCleanup": [],
              "sampleCount": 0, "maxThreads": 0, "sampledDriverCpuTicks": {}, "groupCreated": False}
    process, error = None, None
    def interrupted(signum, _frame):
        raise TimeoutError(f"external controller deadline/signal {signum}")
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        prepared = subprocess.run([sys.executable, str(setup), "prepare", "--name", name], capture_output=True, text=True, check=True, timeout=5)
        record["delegation"] = json.loads(prepared.stdout)
        record["groupCreated"] = True
        libc = ctypes.CDLL(None, use_errno=True)
        if libc.prctl(36, 1, 0, 0, 0) != 0:
            raise OSError(ctypes.get_errno(), "PR_SET_CHILD_SUBREAPER")
        os.sched_setaffinity(0, {0})

        def as_user():
            # Preserve main/search CPU set; only the producer gate joins group.
            os.sched_setaffinity(0, set(range(16)))
            os.initgroups(account.pw_name, account.pw_gid)
            os.setgid(account.pw_gid)
            os.setuid(account.pw_uid)

        environment = {"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": account.pw_dir,
                       "USER": account.pw_name, "LOGNAME": account.pw_name, "LANG": "C.UTF-8",
                       "PROCON37_C009_GPU": "opencl", "PROCON37_C009_GPU_CPUSET": str(group),
                       "PROCON37_C009_GPU_DEVICE": "RTX 5060 Laptop GPU", "PROCON37_C009_GPU_CAP_MS": "250",
                       "PROCON37_C009_GPU_TEMP_MIB": "64"}
        with output.joinpath("probe.jsonl").open("xb") as stdout, output.joinpath("probe.stderr").open("xb") as stderr:
            process = subprocess.Popen([str(binary), "--allow-gpu"], stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr,
                                       env=environment, preexec_fn=as_user, start_new_session=True)
            record["probePid"] = process.pid
            baseline = cpuset(process.pid)
            record["mainInitialCpuset"] = baseline
            entered = set()
            while process.poll() is None:
                if time.monotonic()-began > 45:
                    raise TimeoutError("probe exceeded 45 second monitor bound")
                pending, seen, count = [process.pid], set(), 0
                while pending:
                    pid = pending.pop()
                    if pid in seen:
                        continue
                    seen.add(pid)
                    threads, children = inspect_process(pid)
                    pending.extend(children)
                    count += len(threads)
                    for item in threads:
                        main_or_search = pid == process.pid and (item["tid"] == pid or item["name"].startswith("probe-search-"))
                        # One producer may be observed before its gate joins.
                        before_gate = pid == process.pid and item["name"] == "gpu-producer" and item["tid"] not in entered and item["cpuset"] == baseline
                        if main_or_search or before_gate:
                            valid = item["allowed"] == "0-15" and item["cpuset"] == baseline
                        else:
                            valid = item["allowed"] == "15" and item["cpuset"] == "/"+name
                        valid = valid and item["uid"] == account.pw_uid
                        if item["cpuset"] == "/"+name:
                            entered.add(item["tid"])
                            key = f"{pid}/{item['tid']}"
                            record["sampledDriverCpuTicks"][key] = max(record["sampledDriverCpuTicks"].get(key, 0), item["ticks"])
                        if not valid and len(record["violations"]) < 32:
                            record["violations"].append({"pid": pid, **item})
                record["sampleCount"] += 1
                record["maxThreads"] = max(record["maxThreads"], count)
                time.sleep(0.01)
            record["exitCode"] = process.wait()
            record["joinedProducerObserved"] = bool(entered)
        events = [json.loads(line) for line in output.joinpath("probe.jsonl").read_text().splitlines()]
        pairs = [row for row in events if row.get("event") == "pair"]
        endings = [row for row in events if row.get("event") == "probe_end"]
        record["probeResultsValid"] = (len(pairs) == 6 and len(endings) == 1
            and {(row["seed"], row["repetition"]) for row in pairs} == {(s, r) for s in range(3) for r in range(2)}
            and all(row.get("exactGraphMatch") and not row.get("fallback") for row in pairs)
            and endings[0].get("success") and endings[0].get("normalShutdown") and endings[0].get("placementUnchanged"))
    except Exception as failure:
        error = f"{type(failure).__name__}: {failure}"
    finally:
        if process is not None and process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            record["forcedCleanup"].append({"pid": process.pid, "kind": "owned process group"})
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                error = error or "probe did not exit after SIGKILL"
        cleanup_deadline = min(began+70, time.monotonic()+8)
        while owned_children() and time.monotonic() < cleanup_deadline:
            for pid in owned_children():
                if kill_adopted(pid):
                    record["forcedCleanup"].append({"pid": pid, "kind": "adopted owned descendant"})
            while True:
                try:
                    pid, _ = os.waitpid(-1, os.WNOHANG)
                    if pid == 0:
                        break
                except ChildProcessError:
                    break
            time.sleep(0.01)
        record["remainingOwnedChildren"] = sorted(owned_children())
        if record["groupCreated"]:
            record["remainingGroupTasks"] = group.joinpath("tasks").read_text().split()
            if not record["remainingGroupTasks"]:
                try:
                    cleaned = subprocess.run([sys.executable, str(setup), "cleanup", "--name", name], capture_output=True, text=True, check=True, timeout=5)
                    record["cleanup"] = json.loads(cleaned.stdout)
                except Exception as failure:
                    error = error or f"cleanup: {failure}"
        record["groupExistsAfter"] = group.exists()
        record["error"] = error
        record["wallSeconds"] = time.monotonic()-began
        record["endedUnix"] = time.time()
        record["success"] = bool(not error and record.get("exitCode") == 0 and record.get("probeResultsValid", False) and record.get("joinedProducerObserved", False) and not record["violations"] and not record["forcedCleanup"] and not record["remainingOwnedChildren"] and not record["groupExistsAfter"])
        output.joinpath("controller.json").write_text(json.dumps(record, indent=2)+"\n")
        print(json.dumps(record), flush=True)
    raise SystemExit(0 if record["success"] else 1)


if __name__ == "__main__":
    main()
