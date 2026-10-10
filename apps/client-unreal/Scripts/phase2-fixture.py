#!/usr/bin/env python3
"""Before-admission Phase2 fixture orchestration; never resolves the development Compose file."""
import argparse
import json
import hashlib
import os
from pathlib import Path
import re
import socket
import subprocess
import sys
import uuid
from urllib.parse import unquote, urlsplit

REPO = Path(__file__).resolve().parents[3]
PHASE2 = frozenset(('phase2-transfer', 'phase2-transfer-observer', 'phase2-transfer-missing-token'))
SUPPORTED = PHASE2 | {'default', 'phase1a-social-aggro', 'phase1a-late-entry'}
OWNER = '01970000-0000-7000-8000-000000000020'
OBSERVER = '01970000-0000-7000-8000-000000000040'


def fixture(batch):
    if not batch:
        raise ValueError('fixture requires at least one scenario')
    values = []
    for path in batch:
        lines = [line for line in path.read_text().splitlines() if re.match(r'^#\s*fixture:', line)]
        if len(lines) > 1:
            raise ValueError(f'duplicate fixture header: {path.name}')
        match = re.fullmatch(r'#\s*fixture:\s*(\S+)\s*', lines[0]) if lines else None
        if lines and not match:
            raise ValueError(f'invalid fixture header: {path.name}')
        value = match[1] if match else 'default'
        if value not in SUPPORTED:
            raise ValueError(f'unsupported fixture {value}: {path.name}')
        if value in PHASE2 and lines[0] != '# fixture: ' + value:
            raise ValueError(f'Phase2 requires exact fixture header: {path.name}')
        values.append(value)
    if len(set(values)) != 1:
        raise ValueError('scenario roles must request the same fixture')
    selected = values[0]
    if selected == 'phase2-transfer-observer':
        names = sorted(path.stem for path in batch)
        if len(names) != 2 or not names[0].endswith('-a') or names[1] != names[0][:-2] + '-b':
            raise ValueError('observer fixture requires exactly matching -a/-b roles (use run-sim-multi.sh)')
    elif selected in PHASE2 and len(batch) != 1:
        raise ValueError('solo Phase2 fixture requires one scenario per fresh unit')
    return selected


def safe_bot_args(args):
    if not isinstance(args, list) or any(not isinstance(a, str) or '\0' in a for a in args):
        raise ValueError('SIM_BOT_ARGS_JSON must be an array of strings')
    for arg in args:
        if re.match(r'^-+(devtoken(?:file)?|nfgrpc|nfapi|nfws)(?:=|$)', arg, re.I):
            raise ValueError('Phase2 owns account tokens and client endpoints; conflicting bot argument')


def clean_env():
    # No DATABASE_URL, Compose settings, auth credentials, root .env, or player config inherited.
    return {key: os.environ[key] for key in ('PATH', 'HOME', 'LANG', 'LC_ALL', 'LD_LIBRARY_PATH') if key in os.environ}


def candidate_binaries(folder):
    keys = ('SIM_API_BIN', 'SIM_MIGRATE_BIN')
    missing = [key for key in keys if not os.environ.get(key)]
    defaults = {}
    if missing:
        # Build only before infrastructure exists; never start the API to migrate.
        command = ['cargo', 'build', '--quiet', '-p', 'nightfall-api']
        for key in missing:
            command += ['--bin', 'nightfall-api' if key == 'SIM_API_BIN' else 'nightfall-migrate']
        with (folder / 'binary-build.log').open('w') as log:
            subprocess.run(command, cwd=REPO, env=clean_env(), stdout=log,
                           stderr=subprocess.STDOUT, check=True, timeout=600)
        metadata = subprocess.run(['cargo', 'metadata', '--format-version', '1', '--no-deps'],
                                  cwd=REPO, env=clean_env(), text=True, capture_output=True,
                                  check=True, timeout=60)
        target = Path(json.loads(metadata.stdout)['target_directory']) / 'debug'
        defaults = {key: str(target / ('nightfall-api' if key == 'SIM_API_BIN' else 'nightfall-migrate'))
                    for key in missing}
    result = []
    for key in keys:
        path = Path(os.environ.get(key, defaults.get(key, '')))
        if not path.is_absolute() or not os.access(path, os.X_OK):
            raise ValueError(f'Phase2 requires {key}=absolute executable from the current candidate')
        result.append(str(path.resolve()))
    return result


def psql(folder, arguments):
    value = state(folder)
    original = os.environ.get('PGDATABASE', '')
    if original != value['env']['DATABASE_URL']:
        raise ValueError('psql refused a database outside this fixture')
    url = urlsplit(original)
    if (url.hostname != '127.0.0.1' or url.port != value['ports']['postgres']
            or url.path != '/' + value['database'] or url.query or url.fragment):
        raise ValueError('psql requires the owned fixture endpoint')
    if not url.username or not url.password or any(ord(char) < 32 or ord(char) == 127 for char in original):
        raise ValueError('psql requires a valid owned fixture connection')
    # PGDATABASE is a database name, not a URI: libpq environment defaults do not
    # expand a dbname connection string the way psql's explicit --dbname does.
    connection = {'PGHOST': '127.0.0.1', 'PGPORT': '5432', 'PGDATABASE': url.path[1:],
                  'PGUSER': unquote(url.username), 'PGPASSWORD': unquote(url.password),
                  'PGCONNECT_TIMEOUT': '3', 'PGAPPNAME': 'phase2_fixture_seed'}
    if any(any(ord(char) < 32 or ord(char) == 127 for char in value) for value in connection.values()):
        raise ValueError('psql refused control characters in connection components')
    env = clean_env() | connection
    command = compose(folder, value) + ['exec', '-T']
    for key in connection:
        command += ['--env', key]
    command += ['postgres', 'psql', *arguments]
    subprocess.run(command, cwd=folder, env=env, check=True)


def install_psql(folder):
    shim_dir = folder / 'bin'
    shim_dir.mkdir(mode=0o700)
    shim = shim_dir / 'psql'
    shim.write_text(f'#!{sys.executable}\nimport runpy,sys\nfrom pathlib import Path\n'
                    f'module=runpy.run_path({str(Path(__file__).resolve())!r})\n'
                    f'module["psql"](Path({str(folder)!r}),sys.argv[1:])\n')
    shim.chmod(0o700)
    return str(shim_dir)


def reserve_ports(count):
    sockets = []
    try:
        while len(sockets) < count:
            sock = socket.socket()
            sock.bind(('127.0.0.1', 0))
            if sock.getsockname()[1] in (3000, 50051, 5432, 4222, 8222):
                sock.close()
                continue
            sock.listen()
            sockets.append(sock)
        return sockets
    except BaseException:
        for sock in sockets:
            sock.close()
        raise


def compose_config(db, password, ports):
    pg, nats = ports[:2]
    return {'services': {
        'postgres': {'image': 'postgres:16-alpine', 'environment': {
            'POSTGRES_USER': 'nightfall', 'POSTGRES_PASSWORD': password, 'POSTGRES_DB': db},
            'ports': [f'127.0.0.1:{pg}:5432'], 'volumes': ['pgdata:/var/lib/postgresql/data'],
            'healthcheck': {'test': ['CMD-SHELL', f'pg_isready -U nightfall -d {db}'],
                            'interval': '2s', 'timeout': '3s', 'retries': 30}},
        'nats': {'image': 'nats:2.11-alpine',
                 'command': ['--jetstream', '--store_dir', '/data', '--http_port', '8222'],
                 'ports': [f'127.0.0.1:{nats}:4222'], 'volumes': ['natsdata:/data'],
                 'healthcheck': {'test': ['CMD', 'wget', '-qO-', 'http://localhost:8222/healthz'],
                                 'interval': '2s', 'timeout': '3s', 'retries': 30}}},
        'volumes': {'pgdata': {}, 'natsdata': {}}}


def private_json(path, value):
    with open(path, 'x', opener=lambda name, flags: os.open(name, flags, 0o600)) as file:
        json.dump(value, file, indent=2)
        file.write('\n')


def state(folder):
    value = json.loads((folder / 'state.json').read_text())
    if not re.fullmatch(r'nightfall-sim-phase2-[0-9a-f]{32}', value['project']):
        raise ValueError('invalid owned fixture project')
    return value


def compose(folder, value):
    return ['docker', 'compose', '--env-file', str(folder / 'empty.env'), '-p', value['project'],
            '-f', str(folder / 'compose.json')]


def logged(command, folder, name, env, seconds=180):
    with (folder / name).open('w') as log:
        subprocess.run(command, cwd=folder, env=env, stdout=log, stderr=subprocess.STDOUT,
                       check=True, timeout=seconds)


def provision(folder, batch):
    selected = fixture(batch)
    if selected not in PHASE2:
        raise ValueError('provision requires a Phase2 fixture')
    safe_bot_args(json.loads(os.environ.get('SIM_BOT_ARGS_JSON', '[]')))
    folder.mkdir(mode=0o700, parents=True, exist_ok=False)
    api, migrate = candidate_binaries(folder)
    seeder = Path(os.environ.get('SIM_PHASE2_SEEDER', REPO / 'infra/scripts/seed-phase2-transfer-fixture.py')).resolve()
    # Verify the published pack interface before creating any infrastructure. Old pair-only
    # seeders must fail here, rather than provisioning the wrong solo/missing-token ledger.
    contract = subprocess.run([sys.executable, str(seeder), '--help'], env=clean_env(),
                              text=True, capture_output=True, check=True, timeout=10)
    if '--fixture' not in contract.stdout or any(pack not in contract.stdout for pack in PHASE2):
        raise ValueError('published seeder lacks the Phase2 fixture pack contract')
    identity = uuid.uuid4().hex
    db, project = 'nf_phase2_fixture_' + identity, 'nightfall-sim-phase2-' + identity
    owner, observer = str(uuid.uuid4()), str(uuid.uuid4())
    password = uuid.uuid4().hex
    reserved = reserve_ports(4)
    try:
        ports = [sock.getsockname()[1] for sock in reserved]
        pg, nats, http, grpc = ports
        env = clean_env() | {
            'DATABASE_URL': f'postgres://nightfall:{password}@127.0.0.1:{pg}/{db}',
            'NATS_URL': f'nats://127.0.0.1:{nats}', 'HTTP_ADDR': f'127.0.0.1:{http}',
            'GRPC_ADDR': f'127.0.0.1:{grpc}', 'WS_PUBLIC_URL': f'ws://127.0.0.1:{http}/ws',
            'AUTH_DEV_TOKENS': '1', 'NIGHTFALL_PHASE2_FIXTURE': '1',
            'ZONE_FILE': str(REPO / 'packages/data/zones/test_zone.toml'),
            'RULES_DIR': str(REPO / 'packages/data')}
        roles = {path.stem: {'account_id': observer if selected == 'phase2-transfer-observer' and path.stem.endswith('-b') else owner,
                            'character_id': OBSERVER if selected == 'phase2-transfer-observer' and path.stem.endswith('-b') else OWNER} for path in batch}
        manifest = {'schema_version': 1, 'fixture': selected, 'project': project, 'database': db,
                    'ports': dict(postgres=pg, nats=nats, http=http, grpc=grpc), 'roles': roles,
                    'api_binary': api, 'migrate_binary': migrate, 'seeder': str(seeder),
                    'sha256': {'api': digest(api), 'migrate': digest(migrate), 'seeder': digest(seeder)}}
        private_json(folder / 'state.json', manifest | {'env': env})
        private_json(folder / 'compose.json', compose_config(db, password, ports))
        (folder / 'empty.env').touch(mode=0o600)
        (folder / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    finally:
        for sock in reserved:
            sock.close()
    logged(compose(folder, manifest) + ['up', '-d', '--wait'], folder, 'infra-start.log', clean_env())
    logged([migrate, 'up'], folder, 'migrate.log', env)
    command = [sys.executable, str(seeder), '--fixture', selected, '--owner-account', owner]
    if selected == 'phase2-transfer-observer':
        command += ['--observer-account', observer]
    seed_env = env | {'PATH': install_psql(folder) + os.pathsep + env.get('PATH', os.defpath)}
    logged(command, folder, 'seed.log', seed_env)
    validate_seed(folder, selected, owner, observer)
    # Only a complete seed allows API execution, including after a failed migration/seed.
    (folder / 'seed-complete').touch()


def validate_seed(folder, selected, owner, observer):
    records = [json.loads(line) for line in (folder / 'seed.log').read_text().splitlines()
               if line.startswith('{')]
    if not records:
        raise ValueError('seeder did not publish its fixture manifest')
    seed = records[-1]
    missing = selected == 'phase2-transfer-missing-token'
    expected_owner = {'account_id': owner, 'character_id': OWNER, 'level': 20 if missing else 40,
                      'class_id': 0, 'sex': 'female', 'tokens': [0, 0] if missing else [1, 1],
                      'milestone_claimed_mask': 1 if missing else 3}
    if (seed.get('fixture_enabled') is not True or seed.get('dry_run') is not False
            or seed.get('fixture') != selected or seed.get('owner') != expected_owner
            or seed.get('position') != [126, 126]
            or seed.get('transfers') != ([] if missing else [1, 2])):
        raise ValueError('seeder manifest does not match the requested fixture pack')
    if selected == 'phase2-transfer-observer':
        if seed.get('observer') != {'account_id': observer, 'character_id': OBSERVER,
                                    'level': 1, 'class_id': 0, 'sex': 'male'}:
            raise ValueError('seeder manifest does not match observer role')
    elif 'observer' in seed:
        raise ValueError('solo fixture unexpectedly provisioned an observer')
    (folder / 'seed-manifest.json').write_text(json.dumps(seed, indent=2) + '\n')


def digest(path):
    with open(path, 'rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


def cleanup(folder):
    if not (folder / 'state.json').is_file():
        return
    value = state(folder)
    try:
        logged(compose(folder, value) + ['logs', '--no-color'], folder, 'compose.log', clean_env(), 60)
    finally:
        try:
            logged(compose(folder, value) + ['down', '--volumes', '--remove-orphans'],
                   folder, 'cleanup.log', clean_env(), 60)
        except BaseException:
            (folder / 'cleanup-status.json').write_text('{"completed": false}\n')
            raise
        (folder / 'cleanup-status.json').write_text('{"completed": true}\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('detect', 'provision', 'exec-api', 'endpoint', 'args', 'cleanup'))
    parser.add_argument('--folder', type=Path)
    parser.add_argument('scenarios', type=Path, nargs='*')
    args = parser.parse_intermixed_args()
    if args.command == 'detect':
        print(fixture(args.scenarios))
        return
    folder = args.folder.resolve()
    if args.command == 'provision':
        provision(folder, args.scenarios)
    elif args.command == 'cleanup':
        cleanup(folder)
    else:
        value = state(folder)
        if args.command == 'exec-api':
            if digest(value['api_binary']) != value['sha256']['api']:
                raise ValueError('API binary changed after fixture provisioning')
            if not (folder / 'seed-complete').is_file():
                raise ValueError('API startup requires completed before-admission seed')
            # Running outside the repo also prevents a binary's dotenv lookup from finding root .env.
            os.chdir(folder)
            os.execve(value['api_binary'], [value['api_binary']], value['env'])
        elif args.command == 'endpoint':
            print(f"http://127.0.0.1:{value['ports']['http']}")
            print(value['env']['NATS_URL'])
            print(f"http://127.0.0.1:{value['ports']['grpc']}")
        else:
            role = value['roles'][args.scenarios[0].stem]
            safe_bot_args(json.loads(os.environ.get('SIM_BOT_ARGS_JSON', '[]')))
            bot_args = [f"-DevToken=test:{role['account_id']}", f"-NfGrpc=127.0.0.1:{value['ports']['grpc']}"]
            sys.stdout.buffer.write(b''.join(arg.encode() + b'\0' for arg in bot_args))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, KeyError) as error:
        print(f'sim: {error}', file=sys.stderr)
        sys.exit(2)
    except (OSError, subprocess.SubprocessError):
        # Do not print commands/environments containing credentials.
        print('sim: Phase2 fixture operation failed; check fixture logs and configuration', file=sys.stderr)
        sys.exit(2)
