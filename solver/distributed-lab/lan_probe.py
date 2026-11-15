#!/usr/bin/env python3
"""Bounded idle LAN echo diagnostic. No solver launch, reconfiguration or kill.

Default mode prints a plan. --execute requires an idle evaluation state and uses
the existing SSH execute() helper. Authentication is supplied only by its caller.
"""
import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import shlex
import signal
import socket
import statistics
import struct
import subprocess
import sys
import time

SIZES = (64, 3517, 6826, 11185, 32768)
REPEATS = 50
CPU = {"v1": 31, "v2": 15, "v3": 31}
PORT = 49371
PREFIX = "__LAN_PROBE__"
REMOTE_LIFETIME = 30


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def save(path, value):
    path = Path(path)
    temporary = path.with_suffix(path.suffix + ".new")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")
    temporary.replace(path)


def emit(value):
    print(PREFIX + json.dumps(value), flush=True)


def identity(pid):
    try:
        base = Path("/proc") / str(pid)
        stat = (base / "stat").read_text().rsplit(")", 1)[1].split()
        status = dict(line.split(":", 1) for line in (base / "status").read_text().splitlines() if ":" in line)
        return {"pid": pid, "startTicks": stat[19], "state": stat[0],
                "bootId": Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
                "uid": int(status["Uid"].split()[1]), "cpuAllowed": status["Cpus_allowed_list"].strip(),
                "argv": [v.decode(errors="replace") for v in (base / "cmdline").read_bytes().split(b"\0") if v]}
    except (FileNotFoundError, ProcessLookupError):
        return None


def same_process(actual, expected):
    # Linux clears argv for zombies. Keep identifying that owned PID until it
    # has actually been reaped, instead of reporting it as already disappeared.
    return (actual is not None and all(actual[key] == expected[key] for key in ("pid", "startTicks", "bootId", "uid"))
            and (actual["state"] == "Z" or actual["argv"] == expected["argv"]))


@contextmanager
def exclusive_lab_lock(path):
    with Path(path).open("a+") as handle:
        try:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise RuntimeError("another evaluation/lab operation owns lab.lock") from error
        try:
            yield
        finally:
            fcntl.flock(handle, fcntl.LOCK_UN)


def require_idle(root):
    state = json.loads((root / ".server/distributed-state.json").read_text())
    if state.get("activeEvaluation") or state.get("activeMicrobench"):
        raise RuntimeError("evaluation or microbench active: LAN probe is forbidden")


def fail_on_signal(number, _frame):
    raise TimeoutError(f"bounded process received signal {number}")


def socket_deadline(stream, deadline):
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TimeoutError("frame round-trip deadline")
    stream.settimeout(remaining)


def recv_exact(stream, size, deadline):
    data = bytearray()
    while len(data) < size:
        socket_deadline(stream, deadline)
        block = stream.recv(size - len(data))
        if not block:
            raise EOFError("peer closed before complete frame")
        data.extend(block)
    return bytes(data)


def nic_info(bind_ip, peer_ip):
    record = {"bindIp": bind_ip, "peerIp": peer_ip}
    try:
        result = subprocess.run(["ip", "-j", "route", "get", peer_ip, "from", bind_ip],
                                capture_output=True, text=True, check=True, timeout=1)
        route = json.loads(result.stdout)[0]
        record["route"] = route
        interface = route["dev"]
        if not re.fullmatch(r"[A-Za-z0-9_.:-]+", interface):
            raise ValueError("unexpected interface name")
        record["interface"] = interface
        for field in ("speed", "duplex", "mtu", "operstate", "carrier"):
            try:
                record[field] = (Path("/sys/class/net") / interface / field).read_text().strip()
            except OSError as error:
                record[field] = {"unavailable": str(error)}
        record["speedUnit"] = "Mbit/s advertised by sysfs; not measured throughput"
    except Exception as error:
        record["error"] = str(error)
    return record


def checked_job(path):
    path = Path(path).resolve()
    parent = Path.home() / "procon37-distributed-lan-probes"
    if path.name != "job.json" or path.parent.parent != parent or not re.fullmatch(r"[A-Za-z0-9_-]{1,48}", path.parent.name):
        raise ValueError("job must be in a dedicated fresh home directory")
    job = json.loads(path.read_text())
    if sha(__file__) != job["scriptSha256"] or os.getuid() != job["uid"]:
        raise ValueError("script SHA/UID mismatch")
    if job["cpu"] != CPU[job["host"]] or job["port"] != PORT or job["role"] not in ("client", "server"):
        raise ValueError("unexpected job configuration")
    return job, path.parent


def remote_worker(job_path):
    job, directory = checked_job(job_path)
    if (directory / "process.json").exists() or (directory / "result.json").exists():
        raise RuntimeError("owned process/result already recorded; no retry")
    signal.signal(signal.SIGALRM, fail_on_signal)
    signal.signal(signal.SIGTERM, fail_on_signal)
    signal.alarm(REMOTE_LIFETIME)
    os.sched_setaffinity(0, {job["cpu"]})
    record = identity(os.getpid())
    record.update(role=job["role"], startedUnix=time.time(), scriptSha256=sha(__file__))
    save(directory / "process.json", record)
    began = time.monotonic()
    result = {"role": job["role"], "scope": "idle TCP echo only", "startedUnix": time.time(),
              "hostname": os.uname().nodename, "identity": record, "sizes": list(SIZES), "repeats": REPEATS,
              "tcpNoDelay": True, "roundTripDeadlineSeconds": 1, "lifetimeSeconds": REMOTE_LIFETIME}
    try:
        if job["role"] == "server":
            # No SO_REUSEADDR/REUSEPORT: never share a listener with another run.
            with socket.socket() as listener:
                listener.bind((job["bindIp"], job["port"]))
                listener.listen(1)
                listener.settimeout(1)
                save(directory / "ready.json", {"pid": os.getpid(), "bindIp": job["bindIp"], "port": job["port"]})
                rejected = 0
                while True:
                    try:
                        connection, address = listener.accept()
                    except socket.timeout:
                        continue  # alarm bounds accept even if no client arrives.
                    if address[0] == job["allowedClientIp"]:
                        break
                    connection.close()
                    rejected += 1
                    if rejected >= 16:
                        raise RuntimeError("too many connections from disallowed addresses")
                count = total = 0
                with connection:
                    connection.settimeout(1)
                    connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    for size in SIZES:
                        for _ in range(REPEATS):
                            deadline = time.monotonic() + 1
                            received_size = struct.unpack("!I", recv_exact(connection, 4, deadline))[0]
                            if received_size != size or received_size > 32768:
                                raise ValueError("unexpected/oversized frame")
                            payload = recv_exact(connection, received_size, deadline)
                            total += received_size
                            if total > sum(SIZES) * REPEATS:
                                raise ValueError("payload byte limit exceeded")
                            socket_deadline(connection, deadline)
                            connection.sendall(struct.pack("!I", received_size) + payload)
                            count += 1
                result.update(peerIp=address[0], rejectedConnections=rejected, frames=count,
                              payloadBytesPerDirection=total, success=count == len(SIZES) * REPEATS)
        else:
            results = []
            for peer in job["peers"]:
                with socket.socket() as stream:
                    stream.settimeout(1)
                    stream.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    stream.bind((job["bindIp"], 0))
                    connected = time.perf_counter_ns()
                    stream.connect((peer["address"], job["port"]))
                    connect_ms = (time.perf_counter_ns() - connected) / 1e6
                    for size in SIZES:
                        batch_started = time.time()
                        timings = []
                        for sequence in range(REPEATS):
                            payload = struct.pack("!Q", sequence) + bytes([sequence % 251]) * (size - 8)
                            frame = struct.pack("!I", size) + payload
                            started = time.perf_counter_ns()
                            deadline = time.monotonic() + 1
                            socket_deadline(stream, deadline)
                            stream.sendall(frame)
                            received_size = struct.unpack("!I", recv_exact(stream, 4, deadline))[0]
                            if received_size != size:
                                raise ValueError("echo frame length mismatch")
                            echoed = recv_exact(stream, received_size, deadline)
                            timings.append((time.perf_counter_ns() - started) / 1e6)
                            if echoed != payload:
                                raise ValueError("echo content mismatch")
                        ordered = sorted(timings)
                        results.append({"from": job["host"], "sourceIp": job["bindIp"], "to": peer["host"],
                                        "destinationIp": peer["address"], "payloadBytes": size, "frameBytes": size + 4,
                                        "samplesMs": timings, "p50Ms": statistics.median(ordered),
                                        "p95Ms": ordered[math.ceil(0.95 * len(ordered)) - 1], "maxMs": max(ordered),
                                        "connectMs": connect_ms, "samples": len(timings),
                                        "startedUnix": batch_started, "endedUnix": time.time()})
                result["measurements"] = results
            result.update(frames=len(job["peers"]) * len(SIZES) * REPEATS, success=True)
    except Exception as error:
        result.update(success=False, error=f"{type(error).__name__}: {error}")
    finally:
        signal.alarm(0)
        result.update(endedUnix=time.time(), wallSeconds=time.monotonic() - began,
                      identityAtEnd=identity(os.getpid()))
        save(directory / "result.json", result)
    emit(result)
    return 0 if result["success"] else 1


def remote_control(action, job_path):
    job, directory = checked_job(job_path)
    record_path = directory / "process.json"
    if action == "launch":
        if record_path.exists() or (directory / "ready.json").exists() or (directory / "result.json").exists():
            raise ValueError("existing run artifacts; never restart/reuse a probe")
        if job["role"] != "server":
            raise ValueError("only an echo server may be launched in background")
        with (directory / "stdout.log").open("xb") as stdout, (directory / "stderr.log").open("xb") as stderr:
            child = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), "remote-worker", str(Path(job_path).resolve())],
                                     stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr, start_new_session=True,
                                     env={"PATH": "/usr/bin:/bin", "HOME": str(Path.home()), "LANG": "C.UTF-8"})
        launched = identity(child.pid)
        if launched is not None:
            save(directory / "launch.json", launched)
        until = time.monotonic() + 1
        while not (directory / "ready.json").exists() and child.poll() is None and time.monotonic() < until:
            time.sleep(0.01)
        if not (directory / "ready.json").exists():
            raise RuntimeError("listener did not become ready; inspect/cleanup this owned run")
    records = []
    for filename in ("process.json", "launch.json"):
        path = directory / filename
        if path.exists():
            record = json.loads(path.read_text())
            if record["uid"] != os.getuid() or str(Path(__file__).resolve()) not in record["argv"]:
                raise RuntimeError("process record owner/script mismatch")
            records.append(record)
    cleanup = []
    if action == "cleanup":
        natural_exit = time.monotonic() + 0.25
        while any(same_process(identity(r["pid"]), r) for r in records) and time.monotonic() < natural_exit:
            time.sleep(0.01)
        for expected in records:
            try:
                descriptor = os.pidfd_open(expected["pid"])
            except ProcessLookupError:
                continue
            try:
                actual = identity(expected["pid"])
                if not same_process(actual, expected):
                    if actual is not None:
                        cleanup.append({"pid": expected["pid"], "action": "identity differs; not signalled"})
                    continue
                if actual["state"] != "Z":
                    signal.pidfd_send_signal(descriptor, signal.SIGTERM)
                    cleanup.append({"pid": expected["pid"], "action": "TERM exact owned identity"})
                    until = time.monotonic() + 0.5
                    while same_process(identity(expected["pid"]), expected) and time.monotonic() < until:
                        time.sleep(0.01)
                    actual = identity(expected["pid"])
                    if same_process(actual, expected) and actual["state"] != "Z":
                        signal.pidfd_send_signal(descriptor, signal.SIGKILL)
                        cleanup.append({"pid": expected["pid"], "action": "KILL exact owned identity"})
            finally:
                os.close(descriptor)
        until = time.monotonic() + 0.5
        while any(same_process(identity(r["pid"]), r) for r in records) and time.monotonic() < until:
            time.sleep(0.01)
    residual = [identity(r["pid"]) for r in records if same_process(identity(r["pid"]), r)]
    result = json.loads((directory / "result.json").read_text()) if (directory / "result.json").exists() else None
    tails = {name: (directory / name).read_bytes()[-8192:].decode(errors="replace")
             for name in ("stdout.log", "stderr.log") if (directory / name).exists()}
    emit({"action": action, "host": job["host"], "records": records, "remainingOwnedProcesses": residual,
          "ready": (directory / "ready.json").exists(), "cleanup": cleanup, "result": result, "logTails": tails})


# This setup program is sent through SSH before copying this file. It checks
# binding availability without terminating listeners and contains no credentials.
PREFLIGHT = r'''
import json,os,pathlib,socket,sys
spec=json.loads(sys.argv[1]);home=pathlib.Path.home()
assert os.geteuid()!=0, 'normal experiment user required'
assert str(home)==spec['home'], 'configured home mismatch'
assert spec['cpu'] in os.sched_getaffinity(0), 'control CPU unavailable'
assert (home/'procon37-distributed-lan-probes'/spec['run']).exists()==False, 'run directory already exists'
with socket.socket() as listener:listener.bind((spec['address'],spec['port']))
print('__LAN_PROBE__'+json.dumps({'hostname':os.uname().nodename,'home':str(home),'uid':os.getuid(),'bindAvailable':True,'allowedCpus':sorted(os.sched_getaffinity(0))}))
'''


def local_main():
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--config", type=Path, default=root / ".server/distributed/lab-config.json")
    parser.add_argument("--execute", action="store_true")
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,48}", args.run_id):
        parser.error("run ID must be a fresh 1..48 character ASCII identifier")
    spec = importlib.util.spec_from_file_location("procon37_lan_ssh", root / "solver/distributed/tools/ssh.py")
    ssh = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ssh)
    def read_nodes():
        nodes = json.loads(args.config.read_text())["nodes"]
        for host in ("v1", "v2", "v3", "nomlab"):
            if (nodes[host]["user"], nodes[host]["address"]) != ssh.HOSTS[host]:
                raise ValueError(f"configured {host} differs from SSH allowlist")
        return nodes
    summary = {"scope": "idle dedicated TCP echo; no active search, no GPU, no match scoring",
               "runId": args.run_id, "scriptSha256": sha(__file__), "sizes": list(SIZES), "repeats": REPEATS,
               "port": PORT, "cpu": CPU, "nomlab": "config checked only; never contacted",
               "setupBudgetSeconds": 30, "measurementBudgetSeconds": 30, "cleanupBudgetSeconds": 12,
               "roundTripDeadlineSeconds": 1, "remoteLifetimeSeconds": REMOTE_LIFETIME,
               "limitations": ["each frame has a shared one-second deadline; process alarm bounds the whole workload",
                   "Python framed echo is not D0/C010 2ms control-loop or JSON serialization cost",
                   "samples include first request; TCP connect is recorded separately",
                   "measured round trip is not one-way delay, offer-to-import latency or saturated bandwidth",
                   "control CPU placement does not establish CPU-search coload performance"]}
    if not args.execute:
        read_nodes()
        emit(summary)
        return 0
    # Match manage.py's lock. Acquire before checking state and hold it across
    # preflight, setup, all remote activity and the last cleanup operation.
    with exclusive_lab_lock(root / ".server/distributed/lab.lock"):
        require_idle(root)
        nodes = read_nodes()
        return run_controller(root, args, nodes, ssh, summary)


def run_controller(root, args, nodes, ssh, summary):
    directory = root / ".server/distributed/lan-probes" / args.run_id
    directory.mkdir(parents=True, exist_ok=False)
    summary.update(startedUnix=time.time(), hosts={}, success=False)
    save(directory / "record.json", summary)
    deployed = []
    previous_signals = {number: signal.signal(number, fail_on_signal) for number in (signal.SIGTERM, signal.SIGINT)}

    def execute(host, deadline, command=None, upload=None):
        timeout = min(8.0, deadline - time.monotonic())
        if timeout <= 0:
            raise TimeoutError("controller phase deadline")
        # The original helper has per-read timeouts. An absolute timer plus
        # captured pexpect children bounds authentication too, without changing it.
        spawned = []
        original_spawn = ssh.pexpect.spawn
        previous = signal.signal(signal.SIGALRM, fail_on_signal)
        def capture(*positional, **keywords):
            child = original_spawn(*positional, **keywords)
            spawned.append(child)
            return child
        ssh.pexpect.spawn = capture
        signal.setitimer(signal.ITIMER_REAL, timeout)
        try:
            output = ssh.execute(host, command=command, upload=upload, timeout=timeout)
        finally:
            signal.setitimer(signal.ITIMER_REAL, 0)
            ssh.pexpect.spawn = original_spawn
            signal.signal(signal.SIGALRM, previous)
            for child in spawned:
                if child.isalive():
                    child.close(force=True)
        if upload is not None:
            return None
        rows = [line.split(PREFIX, 1)[1] for line in output.splitlines() if PREFIX in line]
        if len(rows) != 1:
            raise RuntimeError("remote result missing or ambiguous")
        return json.loads(rows[0])

    def command(host, action):
        remote = nodes[host]["home"] + "/procon37-distributed-lan-probes/" + args.run_id
        return shlex.join(["python3", remote + "/lan_probe.py", "remote-control", action, remote + "/job.json"])

    try:
        setup_deadline = time.monotonic() + 30
        # All hosts/ports are checked before any measurement process is launched.
        for host in CPU:
            node = nodes[host]
            request = {"home": node["home"], "cpu": CPU[host], "run": args.run_id, "address": node["address"], "port": PORT}
            report = execute(host, setup_deadline, command=shlex.join(["python3", "-c", PREFLIGHT, json.dumps(request)]))
            summary["hosts"][host] = {"preflight": report}
        for host in CPU:
            node = nodes[host]
            remote = node["home"] + "/procon37-distributed-lan-probes/" + args.run_id
            job = {"host": host, "uid": summary["hosts"][host]["preflight"]["uid"], "cpu": CPU[host], "port": PORT,
                   "role": "client" if host == "v1" else "server", "bindIp": node["address"],
                   "allowedClientIp": nodes["v1"]["address"], "scriptSha256": summary["scriptSha256"],
                   "peers": [{"host": peer, "address": nodes[peer]["address"]} for peer in ("v2", "v3")]}
            local_job = directory / (host + "-job.json")
            save(local_job, job)
            setup = "import pathlib,sys; p=pathlib.Path(sys.argv[1]); p.parent.mkdir(exist_ok=True); p.mkdir(mode=0o700); print('__LAN_PROBE__{}')"
            execute(host, setup_deadline, command=shlex.join(["python3", "-c", setup, remote]))
            execute(host, setup_deadline, upload=(str(Path(__file__).resolve()), remote + "/lan_probe.py"))
            execute(host, setup_deadline, upload=(str(local_job), remote + "/job.json"))
            deployed.append(host)
            report = execute(host, setup_deadline, command=shlex.join(["python3", remote + "/lan_probe.py", "remote-nic", remote + "/job.json"]))
            summary["hosts"][host]["nic"] = report
        summary["setupSeconds"] = time.time() - summary["startedUnix"]
        measurement_deadline = time.monotonic() + 30
        for host in ("v2", "v3"):
            summary["hosts"][host]["launch"] = execute(host, measurement_deadline, command=command(host, "launch"))
        remote = nodes["v1"]["home"] + "/procon37-distributed-lan-probes/" + args.run_id
        # Client is foreground over SSH; it also saves its own process identity
        # before connecting, so timeout cleanup does not depend on SSH return.
        summary["hosts"]["v1"]["measurement"] = execute("v1", measurement_deadline,
            command=shlex.join(["python3", remote + "/lan_probe.py", "remote-worker", remote + "/job.json"]))
    except Exception as error:
        summary["error"] = f"{type(error).__name__}: {error}"
    finally:
        cleanup_deadline = time.monotonic() + 12
        for host in deployed:
            try:
                report = execute(host, cleanup_deadline, command=command(host, "cleanup"))
                summary["hosts"][host]["cleanup"] = report
                save(directory / (host + "-result.json"), report)
            except Exception as error:
                summary["hosts"][host]["cleanupError"] = str(error)
        summary["success"] = ("error" not in summary and len(deployed) == 3
            and all((summary["hosts"][host].get("cleanup", {}).get("result") or {}).get("success")
                and not summary["hosts"][host]["cleanup"]["remainingOwnedProcesses"]
                and not summary["hosts"][host]["cleanup"]["cleanup"]
                and summary["hosts"][host]["cleanup"]["result"]["frames"] == (500 if host == "v1" else 250)
                and summary["hosts"][host]["cleanup"]["result"]["identityAtEnd"]["cpuAllowed"] == str(CPU[host])
                and summary["hosts"][host]["cleanup"]["result"]["identityAtEnd"]["uid"] == summary["hosts"][host]["preflight"]["uid"]
                for host in CPU))
        summary["endedUnix"] = time.time()
        save(directory / "record.json", summary)
    emit(summary)
    for number, previous in previous_signals.items():
        signal.signal(number, previous)
    return 0 if summary["success"] else 1


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "remote-worker":
        raise SystemExit(remote_worker(sys.argv[2]))
    if len(sys.argv) == 4 and sys.argv[1] == "remote-control":
        if sys.argv[2] not in ("launch", "inspect", "cleanup"):
            raise SystemExit("invalid remote control action")
        remote_control(sys.argv[2], sys.argv[3])
    elif len(sys.argv) == 3 and sys.argv[1] == "remote-nic":
        job, _ = checked_job(sys.argv[2])
        peers = job["peers"] if job["role"] == "client" else [{"host": "v1", "address": job["allowedClientIp"]}]
        emit({peer["host"]: nic_info(job["bindIp"], peer["address"]) for peer in peers})
    else:
        raise SystemExit(local_main())
