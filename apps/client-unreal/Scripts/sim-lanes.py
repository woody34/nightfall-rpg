#!/usr/bin/env python3
"""Lanes for run-sim-ci.py --parallel N: one shared Compose stack, API and UE user directory per lane.

A lane is a fixed block of host ports (`STRIDE` consecutive ports from `base + STRIDE * index`),
a Compose project `<project base>-lane<index>`, a generated Compose file whose published ports are
remapped into that block (loopback only), an API environment file, and a UE `-UserDir`. Owned
fixture units (Phase 2/3/4 packs) pick their own free ports and projects, so a lane only adds the
UE user directory to them. The block scheme keeps lanes disjoint by construction; the host probe
refuses a block whose ports are already bound (an owned stack, a soak, another suite).
"""
import json
import socket
from pathlib import Path

PORT_NAMES = ('postgres', 'nats', 'nats_http', 'grafana', 'otlp_grpc', 'otlp_http', 'keycloak',
              'api_http', 'api_grpc')
STRIDE = 10
DEFAULT_PORT_BASE = 30000
DEFAULT_PROJECT_BASE = 'nightfall-sim-phase4'
PROJECT_PREFIX = 'nightfall-sim-'
MAX_PORT = 65535
# docker-compose.yml published ports by (service, container port) -> lane port name.
PUBLISHED = {('postgres', 5432): 'postgres', ('nats', 4222): 'nats', ('nats', 8222): 'nats_http',
             ('lgtm', 3000): 'grafana', ('lgtm', 4317): 'otlp_grpc', ('lgtm', 4318): 'otlp_http',
             ('keycloak', 8080): 'keycloak'}


def lane_ports(base, index):
    """The port block of lane `index`; raises when it leaves the TCP port range."""
    if type(base) is not int or type(index) is not int or base <= 1024 or index < 0:
        raise ValueError('lane port base must be above 1024 and the lane index non-negative')
    start = base + STRIDE * index
    if start + STRIDE - 1 > MAX_PORT:
        raise ValueError(f'lane {index} port block {start}.. leaves the TCP port range')
    return {name: start + offset for offset, name in enumerate(PORT_NAMES)}


EPHEMERAL_RANGE_FILE = Path('/proc/sys/net/ipv4/ip_local_port_range')


def ephemeral_range(path=EPHEMERAL_RANGE_FILE):
    """The kernel's outbound/`bind(0)` port range (low, high), or None when unreadable."""
    try:
        low, high = map(int, Path(path).read_text().split())
    except (OSError, ValueError):
        return None
    return (low, high) if 0 < low <= high <= MAX_PORT else None


def bound(port):
    """True when a socket listens on `port` (IPv4 any/loopback or IPv6 any).

    SO_REUSEADDR lets the probe bind over TIME_WAIT leftovers of a previous run's API while a
    live listener still refuses the bind, so only a real listener reports the block in use."""
    for family, address in ((socket.AF_INET, '0.0.0.0'), (socket.AF_INET, '127.0.0.1'),
                            (socket.AF_INET6, '::')):
        try:
            with socket.socket(family, socket.SOCK_STREAM) as probe:
                probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                probe.bind((address, port))
        except OSError as error:
            if family == socket.AF_INET6 and error.errno in (97, 99):  # EAFNOSUPPORT, EADDRNOTAVAIL
                continue
            return True
    return False


def project_name(base, index):
    if not isinstance(base, str) or not base.startswith(PROJECT_PREFIX) or len(base) <= len(PROJECT_PREFIX):
        raise ValueError(f'lane Compose projects require a {PROJECT_PREFIX}... base name')
    if any(not (c.islower() or c.isdigit() or c in '-_') for c in base):
        raise ValueError('Compose project names are lowercase letters, digits, - and _')
    return f'{base}-lane{index}'


def allocate(parallel, port_base=DEFAULT_PORT_BASE, project_base=DEFAULT_PROJECT_BASE, probe=None, ephemeral=None):
    """Port blocks and project names for every lane; fails fast on a block already in use.

    A block inside the kernel ephemeral range (`ephemeral`, default read from
    /proc/sys/net/ipv4/ip_local_port_range) is refused: owned fixtures (`bind 127.0.0.1:0`) and
    outbound connections could take a lane port between two units. The default base 30000 stays
    below the Linux default range 32768-60999 for up to 276 lanes."""
    if type(parallel) is not int or parallel < 1:
        raise ValueError('parallel must be a positive integer')
    probe = probe or bound
    ephemeral = ephemeral_range() if ephemeral is None else ephemeral
    lanes = []
    for index in range(parallel):
        ports = lane_ports(port_base, index)
        first, last = min(ports.values()), max(ports.values())
        if ephemeral and first <= ephemeral[1] and last >= ephemeral[0]:
            raise ValueError(f'lane {index} port block {first}-{last} intersects the ephemeral port range '
                             f'{ephemeral[0]}-{ephemeral[1]}; choose a lower --lane-port-base or fewer lanes')
        busy = sorted(port for port in ports.values() if probe(port))
        if busy:
            raise ValueError(f'lane {index} port block {min(ports.values())}-{max(ports.values())} is in use '
                             f'(bound: {", ".join(map(str, busy))}); choose another --lane-port-base')
        lanes.append({'index': index, 'ports': ports, 'compose_project': project_name(project_base, index)})
    return lanes


def lane_compose(config, ports):
    """Remap a resolved (`compose config --format json`) stack onto one lane's loopback ports.

    Every published port must be a known stack port; the project owns every volume and network
    (no external resources, no fixed names), as soak-compose.py does for the soak."""
    config = json.loads(json.dumps(config))
    config.pop('name', None)
    for name, service in config['services'].items():
        if 'container_name' in service:
            raise ValueError(f'service {name} has a fixed container_name; lanes need project-owned containers')
        for port in service.get('ports', []):
            key = (name, int(port['target']))
            if key not in PUBLISHED or port.get('protocol', 'tcp') != 'tcp':
                raise ValueError(f'unknown published stack port {name}:{port.get("target")}')
            port['published'] = str(ports[PUBLISHED[key]])
            port['host_ip'] = '127.0.0.1'
    for section in ('volumes', 'networks'):
        for resource in (config.get(section) or {}).values():
            if resource is None:
                continue
            if resource.get('external'):
                raise ValueError('lanes require owned resources, not external ' + section)
            resource.pop('name', None)
    return config


def api_env(ports):
    """API overrides sourced by sim-lib after the repository .env (SIM_API_ENV_FILE)."""
    return {'DATABASE_URL': f"postgres://nightfall:nightfall@127.0.0.1:{ports['postgres']}/nightfall",
            'NATS_URL': f"nats://127.0.0.1:{ports['nats']}",
            'HTTP_ADDR': f"127.0.0.1:{ports['api_http']}",
            'GRPC_ADDR': f"127.0.0.1:{ports['api_grpc']}",
            'WS_PUBLIC_URL': f"ws://127.0.0.1:{ports['api_http']}/ws",
            'OIDC_ISSUER': f"http://localhost:{ports['keycloak']}/realms/nightfall",
            'OTEL_EXPORTER_OTLP_ENDPOINT': f"http://127.0.0.1:{ports['otlp_grpc']}"}


def env_file_text(values):
    for key, value in values.items():
        if not key.isidentifier() or any(c in value for c in '\n\'"\\$`'):
            raise ValueError('environment file values must be plain shell words')
    return ''.join(f"{key}='{value}'\n" for key, value in values.items())


def bot_args(existing_json, user_dir, ports=None):
    """SIM_BOT_ARGS_JSON for a unit: the caller's args, the lane UserDir, and (shared stack
    only) the lane gRPC endpoint. Owned fixture units set their own endpoint, so `ports` is None."""
    existing = json.loads(existing_json) if existing_json else []
    if not isinstance(existing, list) or any(not isinstance(a, str) for a in existing):
        raise ValueError('SIM_BOT_ARGS_JSON must be an array of strings')
    if any(a.lower().startswith('-userdir=') for a in existing):
        raise ValueError('lanes own the UE UserDir; remove -UserDir from SIM_BOT_ARGS_JSON')
    if any('grpcendpoint' in a.lower() for a in existing):
        raise ValueError('lanes own the client gRPC endpoint; remove GrpcEndpoint from SIM_BOT_ARGS_JSON')
    args = [*existing, f'-UserDir={user_dir}']
    if ports is not None:
        args.append(f"-ini:Game:[/Script/Nightfall.NetSettings]:GrpcEndpoint=127.0.0.1:{ports['api_grpc']}")
    return json.dumps(args)


def unit_env(lane, folder, isolated, environ):
    """Environment additions for one unit in `lane` (lane files live in `folder`)."""
    env = {'SIM_BOT_ARGS_JSON': bot_args(environ.get('SIM_BOT_ARGS_JSON'), str(Path(folder) / 'ue-user'),
                                         None if isolated else lane['ports'])}
    if not isolated:
        ports = lane['ports']
        env.update({'COMPOSE_PROJECT_NAME': lane['compose_project'],
                    'COMPOSE_FILE': str(Path(folder) / 'compose.json'),
                    'SIM_API_URL': f"http://localhost:{ports['api_http']}",
                    'SIM_API_ENV_FILE': str(Path(folder) / 'api.env'),
                    'NATS_URL': f"nats://127.0.0.1:{ports['nats']}"})
    return env


def compose_source(repo, folder):
    """The repository stack resolved by Compose itself (absolute bind paths, defaults applied)."""
    import subprocess
    env_file = Path(folder) / 'empty.env'
    env_file.touch()
    result = subprocess.run(['docker', 'compose', '--env-file', str(env_file), '-f', str(Path(repo) / 'docker-compose.yml'),
                             'config', '--format', 'json'], cwd=repo, text=True, capture_output=True, timeout=120)
    if result.returncode:
        raise ValueError('docker compose config failed: ' + result.stderr.strip()[-2000:])
    config = json.loads(result.stdout)
    if not isinstance(config, dict) or not isinstance(config.get('services'), dict):
        raise ValueError('docker compose config returned no services')
    return config


def write_lane(folder, lane, compose_source):
    """Materialize one lane: compose.json, api.env, empty.env and the UE user directory."""
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=False)
    if compose_source is not None:
        (folder / 'compose.json').write_text(json.dumps(lane_compose(compose_source, lane['ports']), indent=2) + '\n')
    (folder / 'api.env').write_text(env_file_text(api_env(lane['ports'])))
    (folder / 'empty.env').touch()
    (folder / 'ue-user').mkdir()
    (folder / 'lane.json').write_text(json.dumps(lane, indent=2, sort_keys=True) + '\n')


def schedule(units, lanes, run):
    """Run every unit exactly once over `lanes` worker threads, each unit whole within one lane.

    `run(lane, unit)` returns the unit's summary row. Rows come back sorted by unit name, with the
    per-lane unit lists in execution order. Exceptions propagate after every worker stops."""
    import threading
    pending = list(enumerate(units))
    lock = threading.Lock()
    rows, errors = {}, []
    for lane in lanes:
        lane['units'] = []

    def worker(lane):
        while True:
            with lock:
                if not pending or errors:
                    return
                index, unit = pending.pop(0)
            try:
                row = run(lane, unit)
                with lock:
                    rows[index] = row
                    lane['units'].append(row['unit'])
            except BaseException as error:  # a lane failure must not strand the other lanes' rows
                with lock:
                    errors.append(error)
                return

    threads = [threading.Thread(target=worker, args=(lane,), name=f"lane-{lane['index']}") for lane in lanes]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    if errors:
        raise errors[0]
    return merge_rows(rows.values())


def merge_rows(rows):
    """Deterministic suite order: by unit name, whatever the lanes' completion order."""
    result = sorted(rows, key=lambda row: row['unit'])
    if len({row['unit'] for row in result}) != len(result):
        raise ValueError('duplicate unit rows in the suite summary')
    return result
