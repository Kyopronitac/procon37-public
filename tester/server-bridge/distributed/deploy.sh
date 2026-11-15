#!/bin/sh
# Copy binaries built by compile.sh to all three wired PCs from VOID1.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$ROOT"
exec python3 tester/server-bridge/distributed.py deploy "$@"
