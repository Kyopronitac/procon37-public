#!/usr/bin/env python3
"""One authorized v1 smoke invocation; root controller stays outside its cpuset.

The child monitor, probe, compiler children, and GPU threads all inherit a fresh
singleton group. Existing groups and output directories are never reused.
"""
import ctypes
import hashlib
import json
import os
from pathlib import Path
import pwd
import signal
import subprocess
import time

EXPECTED_BINARY = "91f2484496bb8577a6509fbec86b4421d9cf6168f08226b7a67488251dc5a94c"
LABEL = "procon37-gpu-producer-smoke-20261009-r1"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def group_of(pid):
    for line in Path(f"/proc/{pid}/cgroup").read_text().splitlines():
        _, controllers, group = line.split(":", 2)
        if "cpuset" in controllers.split(","):
            return group
    return None


def main():
    started = time.monotonic()
    deadline = started + 75
    if os.geteuid() != 0:
        raise RuntimeError("root controller required")
    account = pwd.getpwnam("voidlinux-1")
    lab = Path(__file__).resolve().parent.parent
    if lab != Path(account.pw_dir) / "procon37-distributed" / LABEL:
        raise RuntimeError("unexpected dedicated deployment path")
    binary = lab / "gpu-smoke"
    if digest(binary) != EXPECTED_BINARY:
        raise RuntimeError("frozen binary digest mismatch")
    manifest = json.loads((lab / "deployment-manifest.json").read_text())
    for name, expected in manifest["sha256"].items():
        path = (lab / name).resolve()
        if not path.is_relative_to(lab) or digest(path) != expected:
            raise RuntimeError(f"deployment hash mismatch: {name}")
    parent = Path("/sys/fs/cgroup/cpuset")
    group = parent / LABEL
    output = lab / "results"
    output.mkdir()  # No existing experiment result may be overwritten.
    os.chown(output, account.pw_uid, account.pw_gid)
    group.mkdir()  # No existing cgroup may be reused or deleted.
    process = None
    record = {
        "kind": "synthetic API/driver smoke; not a match or adoption evaluation",
        "label": LABEL, "host": os.uname().nodename, "cpu": 31,
        "started_unix": time.time(), "outer_limit_seconds": 75,
        "monitor_limit_seconds": 45, "group": str(group),
        "binary_sha256": digest(binary), "deployment_sha256": manifest["sha256"],
        "controller_sha256": digest(Path(__file__)), "forced_cleanup": [],
    }
    error = None
    try:
        (group / "cpuset.mems").write_text((parent / "cpuset.mems").read_text())
        (group / "cpuset.cpus").write_text("31")
        if (group / "cpuset.cpus").read_text().strip() != "31":
            raise RuntimeError("singleton cpuset configuration failed")
        libc = ctypes.CDLL(None, use_errno=True)
        if libc.prctl(36, 1, 0, 0, 0) != 0:
            raise OSError(ctypes.get_errno(), "PR_SET_CHILD_SUBREAPER")
        os.sched_setaffinity(0, {31})

        def as_user():
            (group / "tasks").write_text(str(os.getpid()))
            os.sched_setaffinity(0, {31})
            os.initgroups(account.pw_name, account.pw_gid)
            os.setgid(account.pw_gid)
            os.setuid(account.pw_uid)

        command = ["python3", str(lab / "monitor.py"), "--output", str(output / "smoke"),
                   "--timeout", "45", "--expected-cpu", "31", "--", str(binary),
                   "--allow-gpu", "--device", "NVIDIA", "--cpu", "31", "--cpuset", str(group)]
        environment = {"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": account.pw_dir,
                       "USER": account.pw_name, "LOGNAME": account.pw_name, "LANG": "C.UTF-8"}
        record["command"] = command
        with (output / "controller-child.log").open("xb") as log:
            process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=log,
                                       stderr=subprocess.STDOUT, env=environment,
                                       preexec_fn=as_user, start_new_session=True)
            record["monitor_pid"] = process.pid
            record["monitor_initial_cpuset"] = group_of(process.pid)
            record["monitor_initial_affinity"] = sorted(os.sched_getaffinity(process.pid))
            if record["monitor_initial_cpuset"] != "/" + LABEL or record["monitor_initial_affinity"] != [31]:
                raise RuntimeError("monitor initial confinement differs")
            record["monitor_exit_code"] = process.wait(timeout=max(0.1, deadline-time.monotonic()-10))
    except Exception as failure:
        error = f"{type(failure).__name__}: {failure}"
    finally:
        # Membership in this newly-created root-owned group scopes every kill.
        # No broad pkill, prior solver PID, or shared process group is touched.
        cleanup_deadline = min(deadline, time.monotonic()+8)
        seen = set()
        while True:
            tasks = (group / "tasks").read_text().split()
            for tid in tasks:
                try:
                    fields = dict(line.split(":",1) for line in Path(f"/proc/{tid}/status").read_text().splitlines() if ":" in line)
                    pid = int(fields["Tgid"])
                    if pid in seen or group_of(pid) != "/" + LABEL:
                        continue
                    fd = os.pidfd_open(pid)
                    try:
                        if group_of(pid) == "/" + LABEL:
                            signal.pidfd_send_signal(fd, signal.SIGKILL)
                            record["forced_cleanup"].append(pid)
                            seen.add(pid)
                    finally:
                        os.close(fd)
                except (FileNotFoundError, ProcessLookupError):
                    pass
            if process is not None:
                process.poll()
            while True:
                try:
                    pid, _ = os.waitpid(-1, os.WNOHANG)
                    if pid == 0:
                        break
                except ChildProcessError:
                    break
            if not tasks or time.monotonic() >= cleanup_deadline:
                break
            time.sleep(0.01)
        record["remaining_tasks"] = (group / "tasks").read_text().split()
        try:
            group.rmdir()
            record["group_removed"] = True
        except OSError as failure:
            record["group_removed"] = False
            error = error or f"cgroup removal: {failure}"
        record["error"] = error
        record["wall_seconds"] = time.monotonic()-started
        record["ended_unix"] = time.time()
        record["success"] = not error and record.get("monitor_exit_code") == 0 and not record["remaining_tasks"] and record["group_removed"] and not record["forced_cleanup"]
        (output / "controller.json").write_text(json.dumps(record, indent=2)+"\n")
        print(json.dumps(record), flush=True)
    raise SystemExit(0 if record["success"] else 1)


if __name__ == "__main__":
    main()
