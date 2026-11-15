#!/usr/bin/env bash
set -Eeuo pipefail
procon37_repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)
procon37_app=server
source "$procon37_repo/common/desktop/setup.sh"
