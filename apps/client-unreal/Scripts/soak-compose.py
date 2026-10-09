#!/usr/bin/env python3
"""Turn resolved Compose JSON into a fresh, port-offset soak stack."""
import argparse
import copy
import json
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


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('offset', type=int)
    args = parser.parse_args()
    args.output.write_text(json.dumps(isolate(json.loads(args.source.read_text()), args.offset), indent=2) + '\n')
