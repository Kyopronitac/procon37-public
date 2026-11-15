#!/bin/sh
# Build adopted D0 with the installed Rust environment and cached crates on VOID1.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT"
exec python3 tester/server-bridge/distributed.py compile "$@"
