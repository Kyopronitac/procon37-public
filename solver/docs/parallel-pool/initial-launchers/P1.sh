#!/bin/sh
export PROCON37_P1_THREADS=15
exec taskset -c 4-7,12-15,20-23,28-30 /home/voidlinux-1/procon37-parallel-pool/teams/P1/solver 1701 0 2 0 "$@"
