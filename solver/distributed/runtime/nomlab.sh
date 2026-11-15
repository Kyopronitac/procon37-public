#!/bin/sh
export PROCON37_G007_FUTURE_WEIGHT=0
export PROCON37_G007_RESERVE_WEIGHT=15
exec taskset -c 0-15 /home/nomlab-ubuntu/procon37-distributed/bin/distributed-solver --mode parallel --threads 16 --seed 0 "$@"
