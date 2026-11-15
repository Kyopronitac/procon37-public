#!/bin/sh
exec taskset -c 0-3,8-11,16-19,24-26 /home/voidlinux-1/procon37-parallel-pool/teams/P0/solver 0 "$@"
