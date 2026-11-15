#!/usr/bin/env python3
"""Test double of the official procon-server HTTP API (see docs/behavior.md).

It is not the official implementation: agents never move, and only waiting
plans with the exact number of steps are accepted. It follows the same
flags, timing, authentication and response shapes, so the match server can
be tested without the distributed binary.
"""
import argparse
import json
import re
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse


def duration(text):
    match = re.fullmatch(r'(\d+(?:\.\d+)?)(ms|s)', text)
    value = float(match[1])
    return value / 1000 if match[2] == 'ms' else value


parser = argparse.ArgumentParser()
parser.add_argument('-config', required=True)
parser.add_argument('-addr', default=':8080')
parser.add_argument('-kind-deadline', default='5s')
parser.add_argument('-match-start-delay', default='5s')
args = parser.parse_args()
config = json.load(open(args.config))
problem, teams = config['problem'], config['teams']
tokens = [team['token'] for team in teams]
started = time.time()
kinds_close = started + duration(args.kind_deadline)
match_start = kinds_close + duration(args.match_start_delay)
day_ends = []
for seconds in problem['daySeconds']:
    day_ends.append((day_ends[-1] if day_ends else match_start) + seconds)
lock = threading.Lock()
revision = 0
kinds = [[0] * len(problem['agentStarts']) for _ in teams]


def current_day(now):
    return next((d for d, end in enumerate(day_ends) if now < end), len(day_ends))


class Handler(BaseHTTPRequestHandler):
    def reply(self, status, value=None):
        body = b'' if value is None else json.dumps(value).encode()
        self.send_response(status)
        if value is not None:
            self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def team(self):
        url = urlparse(self.path)
        token = self.headers.get('Procon-Token') or parse_qs(url.query).get('token', [None])[0]
        if token not in tokens:
            self.reply(401, {'code': 'AuthError', 'message': 'invalid or missing token'})
            return None, url.path
        return tokens.index(token), url.path

    def closed(self, now):
        if now < match_start:
            self.reply(403, {'code': 'AccessTimeError', 'message': 'match has not started'})
            return True
        if current_day(now) >= len(day_ends):
            self.reply(403, {'code': 'AccessTimeError', 'message': 'match has ended'})
            return True
        return False

    def do_GET(self):
        team, path = self.team()
        if team is None:
            return
        now = time.time()
        if path == '/setting':
            # Like procon-server: startsAt stays 0 until the match starts, and the problem is withdrawn after it ends.
            if current_day(now) >= len(day_ends):
                self.reply(403, {'code': 'AccessTimeError', 'message': 'match has ended'})
                return
            self.reply(200, {'startsAt': int(match_start) if now >= match_start else 0, 'daySeconds': problem['daySeconds'], 'daySteps': problem['daySteps'],
                             'map': {'height': problem['height'], 'width': problem['width'], 'cells': problem['cells']},
                             'spots': problem['spots'], 'agents': problem['agentStarts'], 'fuelLimits': problem['fuelLimits'],
                             'players': len(teams), 'busyThreshold': problem['busyThreshold'], 'jammedThreshold': problem['jammedThreshold']})
        elif path == '/' and not self.closed(now):
            agents = lambda t: [{'kind': k, 'pos': p, 'fuel': problem['fuelLimits']} for k, p in zip(kinds[t], problem['agentStarts'])]
            day = current_day(now)
            self.reply(200, {'endsAt': int(day_ends[day]), 'day': day, 'agents': agents(team),
                             'others': [{'id': t, 'agents': agents(t)} for t in range(len(teams)) if t != team], 'traffics': []})
        elif path != '/':
            self.reply(404, {'message': 'not found'})

    def do_POST(self):
        global revision
        team, path = self.team()
        if team is None:
            return
        body = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))) or b'null')
        now = time.time()
        if path == '/agent':
            if now >= kinds_close:
                self.reply(403, {'code': 'AccessTimeError', 'message': 'agent kind submission is closed'})
            elif not isinstance(body, list) or len(body) != len(kinds[team]) or any(k not in (0, 1) for k in body):
                self.reply(400)
            else:
                kinds[team] = body
                self.reply(200)
        elif path == '/' and not self.closed(now):
            steps = problem['daySteps'][current_day(now)]
            valid = isinstance(body, list) and len(body) == len(kinds[team]) and all(
                isinstance(row, list) and all(isinstance(v, int) and v < 0 for v in row) and -sum(row) == steps for row in body)
            with lock:
                if valid:
                    revision += 1
                self.reply(200, {'revision': revision if valid else -1})

    def log_message(self, format, *values):
        print(format % values, flush=True)


host, port = args.addr.rsplit(':', 1)
print(f'INFO listening addr={args.addr}', flush=True)
ThreadingHTTPServer((host or '0.0.0.0', int(port)), Handler).serve_forever()
