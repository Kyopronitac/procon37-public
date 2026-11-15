#!/bin/sh
export PROCON37_P3_WORKERS=14
export PROCON37_G007_FUTURE_WEIGHT=0
export PROCON37_G007_RESERVE_WEIGHT=15
exec taskset -c 4-7,12-15,20-23,28-30 /home/voidlinux-3/procon37-parallel-pool/teams/P3/solver 0 0 2 0 "$@"
