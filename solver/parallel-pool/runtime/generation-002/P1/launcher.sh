#!/bin/sh
export PROCON37_G007_FUTURE_WEIGHT=0
export PROCON37_G007_RESERVE_WEIGHT=15
exec taskset -c '4-7,12-15,20-23,28-30' /home/voidlinux-1/procon37-parallel-pool/teams/P1/solver 206002 "$@"
