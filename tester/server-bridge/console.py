#!/usr/bin/env python3
"""Save complete runner output while showing only operator progress events."""
import argparse
from datetime import datetime, timezone
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import threading
import time


class Progress:
    """Translate stable bridge events, never pass arbitrary solver diagnostics through."""
    def __init__(self):
        self.phase = 'select'
        self.day = None
        self.started = None
        self.checkpoint = None

    def line(self, raw):
        line = raw.decode('utf-8', errors='replace').rstrip('\r\n')
        node = re.fullmatch(r'(v[123]): (package, token permissions and CPU allocation OK|wired TCP authentication OK \(\d+ search threads\))', line)
        if node:
            return f'[run.sh] distributed_node_access node={node[1]} result=ok'
        peer = re.fullmatch(r'dist peer=(\S+) connection=ready version=\d+ threads=\d+ offset=\d+', line)
        if peer:
            return f'[run.sh] distributed_node_access node={peer[1]} result=ok'
        peer_failure = re.fullmatch(r'dist peer=(\S+) request=\d+ (unavailable|disconnected)=.*', line)
        if peer_failure:
            return f'[run.sh] distributed_node_access node={peer_failure[1]} result={peer_failure[2]}'
        peer_rejected = re.fullmatch(r'dist peer=(\S+) rejected_configuration version=\d+ threads=\d+ offset=\d+', line)
        if peer_rejected:
            return f'[run.sh] distributed_node_access node={peer_rejected[1]} result=configuration_rejected'
        if not line.startswith('[bridge] '):
            return None
        event, _, body = line[len('[bridge] '):].partition(' ')
        fields = dict(re.findall(r'(\w+)=([^\s]+)', body))
        if event == 'connection_check':
            return '[run.sh] problem_server_access result=checking'
        if event in ('server_connection', 'connection_restored', 'connection_lost', 'authentication_failed'):
            status = {'connection_restored': 'reachable', 'connection_lost': 'unreachable',
                      'authentication_failed': 'authentication_rejected'}.get(event, fields.get('result', 'unknown'))
            return f'[run.sh] problem_server_access result={status}'
        if event == 'problem_received':
            return f"[run.sh] problem_received map={fields.get('width', '?')}x{fields.get('height', '?')} days={fields.get('days', '?')}"
        if event == 'solver_started':
            return '[run.sh] solver_started'
        if event == 'solve_start':
            self.phase = fields.get('phase', '?')
            self.day = fields.get('day')
            self.started = time.monotonic()
            self.checkpoint = None
            return None
        if event == 'day_state':
            self.day = fields.get('day')
            return f"[run.sh] day={self.day or '?'}"
        if event == 'checkpoint_submit':
            self.phase = fields.get('phase', self.phase)
            self.checkpoint = fields.get('checkpoint', '?')
            if self.phase == 'plan':
                self.day = fields.get('day', fields.get('request_id', self.day))
            try:
                elapsed = float(fields['elapsed_ms']) / 1000 if 'elapsed_ms' in fields else None
            except ValueError:
                elapsed = None
            seconds = f'{elapsed:.3f}' if elapsed is not None else 'unknown'
            return f'[run.sh] submission {self.scope()} elapsed_sec={seconds} checkpoint={self.checkpoint}'
        if event == 'submission_attempt' and self.checkpoint is None:
            # Legacy bridges do not include phase elapsed time. Mark the local estimate.
            seconds = f'{time.monotonic() - self.started:.3f}' if self.started is not None else 'unknown'
            return f'[run.sh] submission {self.scope()} elapsed_sec_approx={seconds}'
        if event == 'checkpoint_result':
            return f"[run.sh] submission_status {self.scope()} checkpoint={fields.get('checkpoint', '?')} result={fields.get('result', 'unknown')}"
        if event in ('selection_submission', 'plan_submission') and fields.get('scheduled') != 'true':
            phase = 'select' if event == 'selection_submission' else 'plan'
            day = f" day={fields['day']}" if 'day' in fields else ''
            return f"[run.sh] submission_status phase={phase}{day} result={fields.get('result', 'unknown')}"
        if event == 'solver_exited':
            return '[run.sh] solver_exited'
        return None

    def scope(self):
        return f'phase={self.phase}' + (f' day={self.day}' if self.phase == 'plan' and self.day else '')


def supervise(command, log_directory, repeat=False):
    log_directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    stamp = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    path = log_directory / f'{stamp}-{os.getpid()}.log'
    child = None
    interrupted = threading.Event()
    caught_signal = None

    def forward(signum, _frame):
        nonlocal caught_signal
        caught_signal = signum
        interrupted.set()
        if child is not None and child.poll() is None:
            try:
                os.killpg(child.pid, signum)
            except ProcessLookupError:
                pass

    previous = {sig: signal.signal(sig, forward) for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)}
    try:
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, 'wb', buffering=0) as log:
            while not interrupted.is_set():
                progress = Progress()
                child = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                         stdin=subprocess.DEVNULL, start_new_session=True)
                if interrupted.is_set():
                    forward(caught_signal or signal.SIGTERM, None)
                try:
                    for line in child.stdout:
                        log.write(line)
                        message = progress.line(line)
                        if message is not None:
                            print(message, flush=True)
                    code = child.wait()
                finally:
                    child.stdout.close()
                    if child.poll() is None:
                        # Exceptional reader/log failure still stops the owned process group.
                        os.killpg(child.pid, signal.SIGTERM)
                        child.wait()
                log.write(f'[run.sh] child_exit returncode={code}\n'.encode())
                if code != 0 and caught_signal is None:
                    print(f'[run.sh] solver_exited result=runner_stopped exit_code={code}', flush=True)
                if caught_signal is not None:
                    return 128 + caught_signal
                if not repeat or code == 2:
                    return code if code >= 0 else 128 - code
                if code != 0:
                    interrupted.wait(1)
            return 128 + caught_signal if caught_signal is not None else 0
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--log-dir', type=Path, required=True)
    parser.add_argument('--repeat-bridge', action='store_true')
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    if not command:
        parser.error('a command is required')
    try:
        return supervise(command, args.log_dir, args.repeat_bridge)
    except OSError as error:
        print(f'[run.sh] solver_exited result=runner_failed error={error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
