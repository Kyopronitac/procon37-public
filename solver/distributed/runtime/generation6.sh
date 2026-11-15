#!/bin/sh
exec taskset -c 0 /home/jikky/Downloads/procon37/.server/distributed/bin/pool-solver-06  "$@"
