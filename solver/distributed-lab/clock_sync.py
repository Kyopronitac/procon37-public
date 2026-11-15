#!/usr/bin/env python3
"""問題serverと同じローカル時計を実験LANへ配る非特権NTPサーバー。"""
import argparse
import json
import os
import socket
import statistics
import struct
import time
from pathlib import Path

NTP_EPOCH = 2208988800
ALLOWED = {'192.168.0.11', '192.168.0.22', '192.168.0.33', '192.168.0.55', '192.168.0.81', '127.0.0.1'}


def stamp(ns):
    seconds, fraction = divmod(ns, 1_000_000_000)
    return struct.pack('!II', (seconds + NTP_EPOCH) & 0xffffffff, (fraction << 32) // 1_000_000_000)


def seconds(data):
    whole, fraction = struct.unpack('!II', data)
    return whole - NTP_EPOCH + fraction / (1 << 32)


def response(request, received_ns):
    if len(request) < 48 or request[0] & 7 != 3 or (request[0] >> 3) & 7 not in (3, 4):
        return None
    packet = bytearray(48)
    version = (request[0] >> 3) & 7
    # Local free-running reference (stratum 10), not a claim of external UTC accuracy.
    struct.pack_into('!BBBbII4s', packet, 0, version << 3 | 4, 10, request[2], -20,
                     0, 66, bytes((127, 127, 1, 1)))
    packet[16:24] = stamp(received_ns - 1_000_000_000)
    packet[24:32] = request[40:48]
    packet[32:40] = stamp(received_ns)
    packet[40:48] = stamp(time.time_ns())
    return packet


def serve(bind, port, directory):
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as connection:
        connection.bind((bind, port))
        identity = {'pid': os.getpid(), 'bind': bind, 'port': port, 'startedUnix': time.time(),
                    'startTicks': Path('/proc/self/stat').read_text().rsplit(')', 1)[1].split()[19],
                    'bootId': Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
                    'clock': 'CLOCK_REALTIME / time.time_ns; same host as problem server',
                    'source': str(Path(__file__).resolve()), 'allowedClients': sorted(ALLOWED)}
        (directory / 'server-process.json').write_text(json.dumps(identity, indent=2) + '\n')
        print(json.dumps({'event': 'ready', **identity}), flush=True)
        counters = {}
        last_report = time.monotonic()
        while True:
            request, client = connection.recvfrom(512)
            received_ns = time.time_ns()
            if client[0] not in ALLOWED:
                continue
            reply = response(request, received_ns)
            if reply is None:
                continue
            connection.sendto(reply, client)
            counters[client[0]] = counters.get(client[0], 0) + 1
            if time.monotonic() - last_report > 30:
                print(json.dumps({'event': 'served', 'unix': time.time(), 'counts': counters}), flush=True)
                last_report = time.monotonic()


def probe(server, port, samples):
    values = []
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as connection:
        connection.settimeout(2)
        connection.connect((server, port))
        for _ in range(samples):
            request = bytearray(48)
            request[0] = 4 << 3 | 3
            sent_ns = time.time_ns()
            request[40:48] = stamp(sent_ns)
            connection.send(request)
            reply = connection.recv(512)
            received = time.time_ns() / 1_000_000_000
            if len(reply) < 48 or reply[0] & 7 != 4 or reply[24:32] != request[40:48]:
                raise ValueError('NTP response origin or mode mismatch')
            t1, t2, t3, t4 = sent_ns / 1_000_000_000, seconds(reply[32:40]), seconds(reply[40:48]), received
            values.append({'serverMinusClientMs': ((t2 - t1) + (t3 - t4)) * 500,
                           'roundTripMs': ((t4 - t1) - (t3 - t2)) * 1000})
            time.sleep(0.05)
    best = sorted(values, key=lambda item: item['roundTripMs'])[:max(1, samples // 2)]
    return {'server': server, 'port': port, 'samples': values,
            'medianBestHalfOffsetMs': statistics.median(x['serverMinusClientMs'] for x in best),
            'minimumRoundTripMs': min(x['roundTripMs'] for x in values), 'capturedUnix': time.time()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('serve', 'probe'))
    parser.add_argument('--address', default='192.168.0.81')
    parser.add_argument('--port', type=int, default=32123)
    parser.add_argument('--record-dir')
    parser.add_argument('--samples', type=int, default=16)
    args = parser.parse_args()
    if args.mode == 'serve':
        if not args.record_dir:
            parser.error('serve requires --record-dir')
        serve(args.address, args.port, args.record_dir)
    else:
        if not 1 <= args.samples <= 100:
            parser.error('samples must be 1..100')
        print(json.dumps(probe(args.address, args.port, args.samples)))


if __name__ == '__main__':
    main()
