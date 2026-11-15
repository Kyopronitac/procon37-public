#!/bin/sh
export PROCON37_G007_FUTURE_WEIGHT=0
export PROCON37_G007_RESERVE_WEIGHT=15
export PROCON37_DISTRIBUTED_TOKEN_FILE=/home/voidlinux-1/procon37-distributed/token
exec taskset -c 0-31 /home/voidlinux-1/procon37-distributed/bin/distributed-solver --mode coordinator --threads 32 --seed 0 --peers 192.168.0.22:39370/16/31,192.168.0.33:39370/32/46 "$@"
