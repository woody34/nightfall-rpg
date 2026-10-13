#!/usr/bin/env python3
"""Turn resolved Compose JSON into a fresh, port-offset soak stack."""
import argparse
import copy
import json
import re
from pathlib import Path


def isolate(config: dict, offset: int) -> dict:
    config = copy.deepcopy(config)
    config.pop('name', None)
    for service in config['services'].values():
        for port in service.get('ports', []):
            port['published'] = str(int(port['published']) + offset)
    # `compose config` expands BOTH default networks and volumes to explicit names.
    # Remove those names so -p owns every resource, never attaching to a dev stack.
    for section in ('volumes', 'networks'):
        for resource in config.get(section, {}).values():
            if resource.get('external'):
                raise ValueError('soak requires owned resources, not external ' + section)
            resource.pop('name', None)
    return config


TMPFS_VOLUMES = ('pgdata', 'natsdata')


def tmpfs_volumes(config: dict, size: str) -> dict:
    """Back the soak's Postgres/NATS volumes with tmpfs (storage isolation, not production)."""
    if not re.fullmatch(r'[0-9]+[kmgKMG]', size):
        raise ValueError('tmpfs size must look like 8g')
    for name in TMPFS_VOLUMES:
        volume = config.setdefault('volumes', {}).setdefault(name, {}) or {}
        volume.update({'driver': 'local', 'driver_opts': {'type': 'tmpfs', 'device': 'tmpfs', 'o': 'size=' + size}})
        config['volumes'][name] = volume
    return config


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('offset', type=int)
    parser.add_argument('--database')
    parser.add_argument('--tmpfs-volumes', metavar='SIZE', help='back pgdata/natsdata with tmpfs of SIZE (e.g. 8g)')
    args = parser.parse_args()
    config = isolate(json.loads(args.source.read_text()), args.offset)
    if args.tmpfs_volumes:
        config = tmpfs_volumes(config, args.tmpfs_volumes)
    if args.database:
        if not re.fullmatch(r'nf_phase3_fixture_[a-z0-9_]+', args.database):
            parser.error('casting soak requires an isolated Phase3 fixture database')
        config['services']['postgres']['environment']['POSTGRES_DB'] = args.database
    args.output.write_text(json.dumps(config, indent=2) + '\n')
