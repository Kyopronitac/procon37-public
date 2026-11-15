#!/usr/bin/env python3
"""Prepare/remove only one fresh C009 v2 GPU cpuset; never launch a GPU here.

Root retains all controls and delegates only tasks (0600) to voidlinux-2.
Cleanup requires an empty group and never kills a running solver.
"""
import argparse
import json
import os
from pathlib import Path
import pwd
import re


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["prepare", "inspect", "cleanup"])
    parser.add_argument("--name", required=True)
    args = parser.parse_args()
    if os.geteuid() != 0:
        parser.error("root is required")
    if Path("/proc/sys/kernel/hostname").read_text().strip().lower() != "voidlinux-2":
        parser.error("initial C009 deployment is VOIDLINUX-2 only")
    if not re.fullmatch(r"procon37-c009-v2-[a-zA-Z0-9_-]{1,48}", args.name):
        parser.error("unexpected experiment group name")
    parent = Path("/sys/fs/cgroup/cpuset")
    group = parent / args.name
    account = pwd.getpwnam("voidlinux-2")
    if args.action == "prepare":
        group.mkdir(mode=0o755)  # Existing groups are never reused.
        try:
            (group / "cpuset.mems").write_text((parent / "cpuset.mems").read_text())
            (group / "cpuset.cpus").write_text("15")
            os.chown(group / "tasks", account.pw_uid, account.pw_gid)
            os.chmod(group / "tasks", 0o600)
        except Exception:
            if not (group / "tasks").read_text().strip():
                group.rmdir()
            raise
    report = {"action": args.action, "group": str(group), "uid": account.pw_uid,
              "cpu": (group / "cpuset.cpus").read_text().strip(),
              "mems": (group / "cpuset.mems").read_text().strip(),
              "tasks": (group / "tasks").read_text().split(),
              "tasks_uid": (group / "tasks").stat().st_uid,
              "tasks_mode": oct((group / "tasks").stat().st_mode & 0o777),
              "root_owned_controls": all(path.stat().st_uid == 0 and not path.stat().st_mode & 0o022
                                         for path in [group, group / "cpuset.cpus", group / "cpuset.mems"])}
    if report["cpu"] != "15" or report["tasks_uid"] != account.pw_uid or report["tasks_mode"] != "0o600" or not report["root_owned_controls"]:
        raise RuntimeError("delegation configuration mismatch")
    if args.action == "cleanup":
        if report["tasks"]:
            raise RuntimeError("group is not empty; process owner must stop/reap experiment first")
        group.rmdir()
        report["removed"] = not group.exists()
    print(json.dumps(report, sort_keys=True))


if __name__ == "__main__":
    main()
