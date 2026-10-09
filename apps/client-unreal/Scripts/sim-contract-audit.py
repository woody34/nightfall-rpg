#!/usr/bin/env python3
"""Cross-check one isolated run against NF_SESSIONS using runtime protobuf descriptor numbers.

Requires a fresh, dedicated stack containing only the clients in --coverage. SessionOut is
queued before socket delivery, so tail differences on close are reported, never hidden.
Close codes have no server audit record and are explicitly excluded from this comparison.
No raw frames, session credentials or decoded message contents are written to the report.
"""
import argparse
import base64
import importlib.util
import json
from pathlib import Path
import socket
import sys
import uuid
from urllib.parse import urlsplit

spec = importlib.util.spec_from_file_location('contract', Path(__file__).with_name('sim-contract.py'))
contract = importlib.util.module_from_spec(spec)
spec.loader.exec_module(contract)


def varint(data, offset):
    result = 0
    for shift in range(0, 70, 7):
        if offset >= len(data):
            raise ValueError('truncated protobuf varint')
        byte = data[offset]
        offset += 1
        result |= (byte & 127) << shift
        if byte < 128:
            return result, offset
    raise ValueError('oversize protobuf varint')


def field_items(data):
    result = []
    offset = 0
    while offset < len(data):
        key, offset = varint(data, offset)
        tag, wire = key >> 3, key & 7
        if not tag:
            raise ValueError('protobuf field zero')
        if wire == 0:
            value, offset = varint(data, offset)
        elif wire in (1, 2, 5):
            if wire == 2:
                length, offset = varint(data, offset)
            else:
                length = 8 if wire == 1 else 4
            value = data[offset:offset + length]
            if len(value) != length:
                raise ValueError('truncated protobuf field')
            offset += length
        else:
            raise ValueError(f'unsupported protobuf wire type {wire}')
        result.append((tag, value))
    return result


def fields(data):
    return dict(field_items(data))  # scalar fields use the last wire value


def selected_oneof(data, numbers):
    names = {number: name for name, number in numbers.items()}
    selected = None
    value = b''
    for tag, payload in field_items(data):
        if tag not in names:
            continue
        if not isinstance(payload, bytes):
            raise ValueError('message oneof has a non-message wire value')
        name = names[tag]
        # A different member replaces the previous one; repeats of the same message member
        # merge, as protobuf parsing does. Concatenating message bytes preserves that merge.
        value = value + payload if selected == name else payload
        selected = name
    return selected, value


def tally_frame(frame, incoming, numbers, counts):
    group = 'intents' if incoming else 'payloads'
    name, payload = selected_oneof(frame, numbers[group])
    if name is None:
        return
    counts[group][name] += 1
    if incoming:
        return
    if name == 'event':
        case, _ = selected_oneof(payload, numbers['events'])
        if case is not None:
            counts['events'][case] += 1
    elif name == 'rejected':
        reason = fields(payload).get(2, 0)
        known = next((case for case, number in numbers['reasons'].items() if number == reason), f'unknown_{reason}')
        counts['reasons'][known] = counts['reasons'].get(known, 0) + 1


class Nats:
    """Minimal request/reply client for the public JetStream JSON management protocol."""
    def __init__(self, url):
        parsed = urlsplit(url)
        if parsed.username or parsed.password or parsed.scheme != 'nats':
            raise ValueError('use a dedicated local nats://host:port without credentials')
        self.socket = socket.create_connection((parsed.hostname, parsed.port or 4222), timeout=10)
        self.reader = self.socket.makefile('rb')
        if not self.reader.readline().startswith(b'INFO '):
            raise ValueError('missing NATS INFO')
        self.inbox = '_INBOX.nightfall_contract_' + uuid.uuid4().hex
        self.socket.sendall(f'CONNECT {{"verbose":false,"pedantic":false}}\r\nSUB {self.inbox} 1\r\n'.encode())

    def request(self, subject, body):
        payload = json.dumps(body).encode()
        self.socket.sendall(f'PUB {subject} {self.inbox} {len(payload)}\r\n'.encode() + payload + b'\r\n')
        while True:
            line = self.reader.readline()
            if not line:
                raise ValueError('NATS connection closed')
            if line.startswith(b'PING'):
                self.socket.sendall(b'PONG\r\n')
            elif line.startswith(b'-ERR'):
                raise ValueError('NATS request rejected')
            elif line.startswith(b'MSG '):
                size = int(line.split()[-1])
                payload = self.reader.read(size)
                if self.reader.read(2) != b'\r\n':
                    raise ValueError('malformed NATS payload')
                result = json.loads(payload)
                if 'error' in result:
                    raise ValueError(f"JetStream error {result['error'].get('code')}: {result['error'].get('description')}")
                return result

    def close(self):
        self.reader.close()
        self.socket.close()


def compare(coverage, records):
    numbers = coverage.get('wire_numbers')
    if not isinstance(numbers, dict) or any(group not in numbers for group in ('intents', 'payloads', 'events', 'reasons')):
        raise ValueError('runtime compiled descriptor numbers required')
    counts = {group: {name: 0 for name in entries} for group, entries in coverage['counts'].items() if group != 'close_codes'}
    frames = {'client_frames': 0, 'server_frames': 0}
    sessions = set()
    for subject, record in records:
        if subject.endswith('.checkpoint'):
            continue  # application JSON save audit is not a WebSocket frame
        incoming = subject.endswith('.in')
        if not incoming and not subject.endswith('.out'):
            raise ValueError('unexpected NF_SESSIONS subject')
        wrapper = fields(record)
        sessions.add(wrapper.get(1, b'').hex())
        frame = wrapper.get(7 if incoming else 5)
        if not isinstance(frame, bytes):
            raise ValueError('missing session audit frame')
        frames['client_frames' if incoming else 'server_frames'] += 1
        tally_frame(frame, incoming, numbers, counts)
    if not sessions:
        raise ValueError('no SessionIn/SessionOut audit records: verify server wire audit adapter is enabled')
    differences = []
    for group, entries in counts.items():
        for name in sorted(set(entries) | set(coverage['counts'][group])):
            server, client = entries.get(name, 0), int(coverage['counts'][group].get(name, 0))
            if server != client:
                differences.append({'case': f'{group}.{name}', 'audit': str(server), 'client': str(client)})
    for frame, count in frames.items():
        if count != int(coverage.get(frame, 0)):
            differences.append({'case': frame, 'audit': str(count), 'client': str(coverage.get(frame, 0))})
    return {'schema_version': 1, 'source': 'NF_SESSIONS SessionIn/SessionOut', 'sessions': len(sessions),
            'audit_counts': {g: {n: str(v) for n, v in entries.items()} for g, entries in counts.items()},
            'audit_frames': {k: str(v) for k, v in frames.items()}, 'differences': differences,
            'excluded': ['close_codes: no server session audit record'],
            'delivery_semantics': 'SessionOut audits queue admission before socket delivery; disconnect tails may differ'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--coverage', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--nats', default='nats://127.0.0.1:4222')
    parser.add_argument('--fresh-stack', action='store_true', required=True)
    args = parser.parse_args()
    client = None
    try:
        coverage = contract.read(args.coverage)
        client = Nats(args.nats)
        state = client.request('$JS.API.STREAM.INFO.NF_SESSIONS', {})['state']
        records = []
        for sequence in range(state['first_seq'], state['last_seq'] + 1):
            message = client.request('$JS.API.STREAM.MSG.GET.NF_SESSIONS', {'seq': sequence})['message']
            records.append((message['subject'], base64.b64decode(message['data'], validate=True)))
        data = compare(coverage, records)
        frame_count = sum(int(count) for count in data['audit_frames'].values())
        data['stream_records'] = len(records)
        data['ignored_checkpoint_records'] = len(records) - frame_count
        args.out.write_text(json.dumps(data, indent=2, sort_keys=True) + '\n')
        print(f"contract audit: {frame_count} wire frames, {data['sessions']} sessions, {len(data['differences'])} differences ({len(records)} stream records)")
        return bool(data['differences'])
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f'contract audit failed: {error}', file=sys.stderr)
        return 1
    finally:
        if client:
            client.close()


if __name__ == '__main__':
    sys.exit(main())
