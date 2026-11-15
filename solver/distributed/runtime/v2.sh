#!/bin/sh
export PROCON37_G007_FUTURE_WEIGHT=0
export PROCON37_G007_RESERVE_WEIGHT=15
export PROCON37_DISTRIBUTED_TOKEN_FILE=/home/voidlinux-2/procon37-distributed/token
exec taskset -c 0-15 /home/voidlinux-2/procon37-distributed/bin/distributed-solver --mode worker --threads 16 --worker-offset 31 --listen 192.168.0.22:39370 "$@"
