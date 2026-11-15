#!/bin/sh
exec taskset -c 0-3,8-11,16-19,24-26 /home/voidlinux-3/procon37-parallel-pool/teams/P2/solver 0 0 2 0 "$@"
