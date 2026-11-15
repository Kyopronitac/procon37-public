#!/bin/sh
exec taskset -c 2 /home/jikky/Downloads/procon37/.server/distributed/bin/pool-solver-08  "$@"
