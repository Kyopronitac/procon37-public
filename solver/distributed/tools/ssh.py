#!/usr/bin/env python3
"""実験PCへSSH/SCP。パスワードは環境変数から取得して記録しない。"""
import argparse
import os
import sys
import pexpect

HOSTS = {'v1': ('voidlinux-1', '192.168.0.11'),
         'v2': ('voidlinux-2', '192.168.0.22'),
         'v3': ('voidlinux-3', '192.168.0.33'),
         'nomlab': ('nomlab-ubuntu', '192.168.0.55')}

def execute(host, command=None, upload=None, download=None, timeout=120):
    user, address = HOSTS[host]
    options = ['-o', 'StrictHostKeyChecking=accept-new', '-o', 'ConnectTimeout=10']
    if upload:
        program, args = 'scp', options + [upload[0], f'{user}@{address}:{upload[1]}']
    elif download:
        program, args = 'scp', options + [f'{user}@{address}:{download[0]}', download[1]]
    else:
        program, args = 'ssh', options + [f'{user}@{address}', command]
    child = pexpect.spawn(program, args, encoding='utf-8', timeout=timeout)
    result = ''
    while True:
        match = child.expect(['[Pp]assword:', pexpect.EOF, pexpect.TIMEOUT])
        result += child.before
        if match == 0:
            child.setecho(False)
            child.sendline(os.environ['PROCON37_SSH_PASSWORD'])
        elif match == 1:
            child.close()
            if child.exitstatus != 0:
                raise RuntimeError(f'{host}: exit {child.exitstatus}: {result}')
            return result
        else:
            child.close(force=True)
            raise TimeoutError(f'{host}: {result}')

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('host', choices=HOSTS)
    parser.add_argument('command', nargs='?')
    parser.add_argument('--upload', nargs=2)
    parser.add_argument('--download', nargs=2)
    parser.add_argument('--timeout', type=int, default=120)
    args = parser.parse_args()
    sys.stdout.write(execute(args.host, args.command, args.upload, args.download, args.timeout))
