#!/usr/bin/env python3
"""run-sim-ci.py --parallel lanes: offsets, Compose remap, scheduler and summary merge, without a stack."""
import contextlib
import importlib.util
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

SCRIPTS = Path(__file__).resolve().parents[1]


def module(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_').replace('.py', ''), SCRIPTS / name)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


lanes = module('sim-lanes.py')
ci = module('run-sim-ci.py')

STACK = {'name': 'nightfall-rpg', 'services': {
    'postgres': {'image': 'postgres:16-alpine', 'ports': [{'mode': 'ingress', 'target': 5432, 'published': '5432', 'protocol': 'tcp'}],
                 'volumes': [{'type': 'volume', 'source': 'pgdata', 'target': '/var/lib/postgresql/data'}]},
    'nats': {'image': 'nats:2.11-alpine', 'ports': [{'mode': 'ingress', 'target': 4222, 'published': '4222', 'protocol': 'tcp'},
                                                    {'mode': 'ingress', 'target': 8222, 'published': '8222', 'protocol': 'tcp'}]},
    'lgtm': {'image': 'grafana/otel-lgtm:latest', 'ports': [{'mode': 'ingress', 'target': 3000, 'published': '3300', 'protocol': 'tcp'},
                                                            {'mode': 'ingress', 'target': 4317, 'published': '4317', 'protocol': 'tcp'},
                                                            {'mode': 'ingress', 'target': 4318, 'published': '4318', 'protocol': 'tcp'}]},
    'keycloak': {'image': 'quay.io/keycloak/keycloak:26.8.0', 'ports': [{'mode': 'ingress', 'target': 8080, 'published': '8080', 'protocol': 'tcp'}]}},
    'networks': {'default': {'name': 'nightfall-rpg_default', 'ipam': {}}},
    'volumes': {'pgdata': {'name': 'nightfall-rpg_pgdata'}, 'natsdata': {'name': 'nightfall-rpg_natsdata'}, 'lgtmdata': {'name': 'nightfall-rpg_lgtmdata'}}}


class OffsetTests(unittest.TestCase):
    def test_lane_blocks_are_disjoint_in_range_and_cover_every_stack_port(self):
        plan = lanes.allocate(8, probe=lambda port: False)
        seen = set()
        for lane in plan:
            ports = lane['ports']
            self.assertEqual(set(ports), set(lanes.PORT_NAMES))
            self.assertEqual(min(ports.values()), lanes.DEFAULT_PORT_BASE + lanes.STRIDE * lane['index'])
            self.assertTrue(set(ports.values()).isdisjoint(seen))
            seen |= set(ports.values())
            self.assertEqual(lane['compose_project'], f"nightfall-sim-phase4-lane{lane['index']}")
        self.assertTrue(all(1024 < port <= 65535 for port in seen))
        # Owned stacks (27432/27422/28080), the integration override (25432..28222) and the soak (+10000) stay clear.
        self.assertTrue(seen.isdisjoint({27432, 27422, 28080, 25432, 24222, 28222, 23300, 24317, 24318, 28081,
                                         15432, 14222, 18222, 13300, 14317, 14318, 18080, 13000, 15051}))

    def free_port_below_ephemeral(self):
        """A free port below the kernel ephemeral range, aligned to a lane block."""
        for port in range(29990, 20000, -lanes.STRIDE):
            if not any(lanes.bound(p) for p in range(port, port + lanes.STRIDE)):
                return port
        self.skipTest('no free lane block below the ephemeral range')

    def test_allocation_fails_fast_on_a_bound_port_and_out_of_range_blocks(self):
        base = self.free_port_below_ephemeral()
        port = base + 7  # api_http
        with socket.socket() as held:
            held.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            held.bind(('127.0.0.1', port))
            held.listen()
            with self.assertRaises(ValueError) as refused:
                lanes.allocate(1, port_base=base, ephemeral=(32768, 60999))
            self.assertIn(str(port), str(refused.exception))
            self.assertIn('in use', str(refused.exception))
            self.assertTrue(lanes.bound(port))
        self.assertFalse(lanes.bound(port))
        self.assertEqual(len(lanes.allocate(1, port_base=base, ephemeral=(32768, 60999))), 1)
        with self.assertRaises(ValueError):
            lanes.allocate(2, port_base=65535 - lanes.STRIDE, probe=lambda port: False, ephemeral=None)
        for bad in (0, -1, '2'):
            with self.assertRaises(ValueError):
                lanes.allocate(bad, probe=lambda port: False)
        with self.assertRaises(ValueError):
            lanes.allocate(1, project_base='dev-stack', probe=lambda port: False)
        with self.assertRaises(ValueError):
            lanes.allocate(1, project_base='nightfall-sim-Upper', probe=lambda port: False)
        self.assertEqual(lanes.allocate(1, project_base='nightfall-sim-ci-7', probe=lambda port: False)[0]['compose_project'],
                         'nightfall-sim-ci-7-lane0')

    def test_time_wait_port_is_not_in_use_but_a_listener_is(self):
        # Real listeners (Rust std/tokio, docker-proxy) set SO_REUSEADDR; Linux lets a reusing probe
        # bind over their TIME_WAIT leftovers only when the old socket reused too.
        with socket.socket() as server:
            server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            server.bind(('127.0.0.1', 0))
            server.listen()
            port = server.getsockname()[1]
            self.assertTrue(lanes.bound(port))
            with socket.create_connection(('127.0.0.1', port)) as client:
                accepted, _ = server.accept()
                accepted.close()  # server-side active close: the port enters TIME_WAIT
                client.recv(1)
        self.assertFalse(lanes.bound(port))  # TIME_WAIT leftovers do not refuse a block
        with socket.socket() as again:
            again.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            again.bind(('127.0.0.1', port))
            again.listen()
            self.assertTrue(lanes.bound(port))

    def test_blocks_inside_the_ephemeral_range_are_refused(self):
        free = lambda port: False
        with self.assertRaises(ValueError) as refused:
            lanes.allocate(1, port_base=40000, probe=free, ephemeral=(32768, 60999))
        self.assertIn('ephemeral', str(refused.exception))
        with self.assertRaises(ValueError):  # the last lane's block crosses the lower bound
            lanes.allocate(277, port_base=30000, probe=free, ephemeral=(32768, 60999))
        self.assertEqual(len(lanes.allocate(276, port_base=30000, probe=free, ephemeral=(32768, 60999))), 276)
        self.assertEqual(len(lanes.allocate(1, port_base=61000, probe=free, ephemeral=(32768, 60999))), 1)
        self.assertEqual(len(lanes.allocate(1, port_base=40000, probe=free, ephemeral=False)), 1)
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'range'
            path.write_text('32768\t60999\n')
            self.assertEqual(lanes.ephemeral_range(path), (32768, 60999))
            path.write_text('garbage\n')
            self.assertIsNone(lanes.ephemeral_range(path))
            self.assertIsNone(lanes.ephemeral_range(Path(tmp) / 'missing'))
        self.assertIsNotNone(lanes.ephemeral_range())  # this host's /proc range reads


class ComposeTests(unittest.TestCase):
    def test_published_ports_remap_to_loopback_block_and_names_become_owned(self):
        ports = lanes.lane_ports(30000, 2)
        config = lanes.lane_compose(STACK, ports)
        self.assertNotIn('name', config)
        published = {(name, port['target']): (port['host_ip'], int(port['published']))
                     for name, service in config['services'].items() for port in service['ports']}
        self.assertEqual(published, {('postgres', 5432): ('127.0.0.1', ports['postgres']), ('nats', 4222): ('127.0.0.1', ports['nats']),
                                     ('nats', 8222): ('127.0.0.1', ports['nats_http']), ('lgtm', 3000): ('127.0.0.1', ports['grafana']),
                                     ('lgtm', 4317): ('127.0.0.1', ports['otlp_grpc']), ('lgtm', 4318): ('127.0.0.1', ports['otlp_http']),
                                     ('keycloak', 8080): ('127.0.0.1', ports['keycloak'])})
        self.assertTrue(all('name' not in v for v in config['volumes'].values()))
        self.assertNotIn('name', config['networks']['default'])
        self.assertEqual(STACK['volumes']['pgdata'], {'name': 'nightfall-rpg_pgdata'})  # input untouched

    def test_unknown_ports_and_external_resources_are_refused(self):
        extra = json.loads(json.dumps(STACK))
        extra['services']['nats']['ports'].append({'target': 6222, 'published': '6222', 'protocol': 'tcp'})
        with self.assertRaises(ValueError):
            lanes.lane_compose(extra, lanes.lane_ports(30000, 0))
        external = json.loads(json.dumps(STACK))
        external['volumes']['pgdata'] = {'external': True, 'name': 'shared'}
        with self.assertRaises(ValueError):
            lanes.lane_compose(external, lanes.lane_ports(30000, 0))
        named = json.loads(json.dumps(STACK))
        named['services']['postgres']['container_name'] = 'nightfall-postgres'
        with self.assertRaises(ValueError):
            lanes.lane_compose(named, lanes.lane_ports(30000, 0))

    def test_api_env_and_unit_env_follow_the_lane_block(self):
        lane = lanes.allocate(2, probe=lambda port: False)[1]
        ports = lane['ports']
        env = lanes.api_env(ports)
        self.assertEqual(env['HTTP_ADDR'], f"127.0.0.1:{ports['api_http']}")
        self.assertEqual(env['GRPC_ADDR'], f"127.0.0.1:{ports['api_grpc']}")
        self.assertIn(f":{ports['postgres']}/nightfall", env['DATABASE_URL'])
        self.assertEqual(env['NATS_URL'], f"nats://127.0.0.1:{ports['nats']}")
        self.assertIn(f":{ports['api_http']}/ws", env['WS_PUBLIC_URL'])
        self.assertIn(f":{ports['keycloak']}/realms/nightfall", env['OIDC_ISSUER'])
        text = lanes.env_file_text(env)
        self.assertEqual(subprocess.run(['bash', '-c', f'set -a; eval "$1"; echo "$HTTP_ADDR"', '_', text],
                                        text=True, capture_output=True).stdout.strip(), env['HTTP_ADDR'])
        with self.assertRaises(ValueError):
            lanes.env_file_text({'X': "a'b"})
        shared = lanes.unit_env(lane, '/lanes/lane-1', False, {'SIM_BOT_ARGS_JSON': '["-ResX=1280"]'})
        self.assertEqual(shared['COMPOSE_PROJECT_NAME'], lane['compose_project'])
        self.assertEqual(shared['COMPOSE_FILE'], '/lanes/lane-1/compose.json')
        self.assertEqual(shared['SIM_API_URL'], f"http://localhost:{ports['api_http']}")
        self.assertEqual(shared['SIM_API_ENV_FILE'], '/lanes/lane-1/api.env')
        self.assertEqual(shared['NATS_URL'], f"nats://127.0.0.1:{ports['nats']}")
        self.assertEqual(json.loads(shared['SIM_BOT_ARGS_JSON']),
                         ['-ResX=1280', '-UserDir=/lanes/lane-1/ue-user',
                          f"-ini:Game:[/Script/Nightfall.NetSettings]:GrpcEndpoint=127.0.0.1:{ports['api_grpc']}"])
        # Owned fixture units choose their own stack and endpoint; the lane only isolates the UE user directory.
        isolated = lanes.unit_env(lane, '/lanes/lane-1', True, {})
        self.assertEqual(set(isolated), {'SIM_BOT_ARGS_JSON'})
        self.assertEqual(json.loads(isolated['SIM_BOT_ARGS_JSON']), ['-UserDir=/lanes/lane-1/ue-user'])
        self.assertFalse(any('grpcendpoint' in a.lower() for a in json.loads(isolated['SIM_BOT_ARGS_JSON'])))
        with self.assertRaises(ValueError):
            lanes.bot_args('["-UserDir=/elsewhere"]', '/x')
        with self.assertRaises(ValueError):
            lanes.bot_args('["-ini:Game:[/Script/Nightfall.NetSettings]:GrpcEndpoint=127.0.0.1:1"]', '/x')
        with self.assertRaises(ValueError):
            lanes.bot_args('{"a": 1}', '/x')

    def test_write_lane_materializes_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            lane = lanes.allocate(1, probe=lambda port: False)[0]
            folder = Path(tmp) / 'lane-0'
            lanes.write_lane(folder, lane, STACK)
            self.assertEqual(sorted(p.name for p in folder.iterdir()), ['api.env', 'compose.json', 'empty.env', 'lane.json', 'ue-user'])
            compose = json.loads((folder / 'compose.json').read_text())
            self.assertEqual(compose['services']['postgres']['ports'][0]['published'], str(lane['ports']['postgres']))
            with self.assertRaises(FileExistsError):
                lanes.write_lane(folder, lane, STACK)
            lanes.write_lane(Path(tmp) / 'lane-1', lane, None)
            self.assertFalse((Path(tmp) / 'lane-1/compose.json').exists())


class SchedulerTests(unittest.TestCase):
    def test_every_unit_runs_once_whole_in_one_lane_and_rows_merge_sorted(self):
        plan = lanes.allocate(3, probe=lambda port: False)
        units = ['0b-click-move', '3-pvp-flag', '1-target-attack', '4-potion', '2-class-transfer', '4-shield-block']
        seen, lock = [], threading.Lock()

        def run(lane, unit):
            with lock:
                seen.append((lane['index'], unit, threading.current_thread().name))
            time.sleep(0.05 if unit.startswith('0b') else 0.01)
            return {'unit': unit, 'lane': lane['index']}

        rows = lanes.schedule(units, plan, run)
        self.assertEqual([row['unit'] for row in rows], sorted(units))
        self.assertEqual(sorted(unit for _, unit, _ in seen), sorted(units))
        self.assertEqual(len(seen), len(units))
        for index, unit, thread in seen:
            self.assertEqual(thread, f'lane-{index}')
        self.assertEqual(sorted(unit for lane in plan for unit in lane['units']), sorted(units))
        self.assertEqual({row['unit']: row['lane'] for row in rows},
                         {unit: lane['index'] for lane in plan for unit in lane['units']})
        # Three lanes started three units at once: the first three queue entries went to distinct lanes.
        self.assertEqual(len({index for index, _, _ in seen[:3]}), 3)

    def test_lane_exception_propagates_after_other_lanes_stop(self):
        plan = lanes.allocate(2, probe=lambda port: False)

        def run(lane, unit):
            if unit == 'boom':
                raise RuntimeError('lane crashed')
            time.sleep(0.01)
            return {'unit': unit}

        with self.assertRaises(RuntimeError):
            lanes.schedule(['boom', 'a', 'b', 'c', 'd'], plan, run)

    def test_merge_rejects_duplicate_units(self):
        self.assertEqual([r['unit'] for r in lanes.merge_rows([{'unit': 'b'}, {'unit': 'a'}])], ['a', 'b'])
        with self.assertRaises(ValueError):
            lanes.merge_rows([{'unit': 'a'}, {'unit': 'a'}])


class SuiteTests(unittest.TestCase):
    """main() with --parallel 2: patched wrappers, no Docker, no Unreal."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.scenarios = self.root / 'scenarios'
        self.scenarios.mkdir()
        for name in ('0b-click-move', '1-target-attack', '3-pvp-flag-a', '3-pvp-flag-b', 'z-last'):
            (self.scenarios / f'{name}.nfs').write_text('nf.Within 30\n')
        (self.scenarios / '4-potion.nfs').write_text('# fixture: phase4-consumables\nnf.Within 30\n')
        (self.root / 'quarantine.json').write_text('{"schema_version":1,"scenarios":[]}')
        (self.root / 'baseline.json').write_text('{}')

    def args(self, *extra):
        return ['run-sim-ci.py', '--scenarios', str(self.scenarios), '--quarantine', str(self.root / 'quarantine.json'),
                '--artifacts', str(self.root / 'suite'), '--contract-baseline', str(self.root / 'baseline.json'),
                '--fresh-stack', *extra]

    def run_main(self, *extra, environ=None, probe=lambda port: False):
        seen, calls, lock = [], [], threading.Lock()

        def clients(command, log, seconds, env):
            with lock:
                seen.append((command, env))
            time.sleep(0.02)
            return 2

        def run(command, *a, **k):
            with lock:
                calls.append((command, k.get('env')))
            return subprocess.CompletedProcess(command, 0, stdout='')

        with patch('sys.argv', self.args(*extra)), patch.dict(os.environ, environ or {}, clear=False), \
                patch.object(ci, 'run_clients', clients), patch.object(ci.subprocess, 'run', run), \
                patch.object(ci.lanes, 'bound', probe), patch.object(ci.lanes, 'compose_source', lambda repo, folder: STACK), \
                patch.object(ci, 'suite_coverage', lambda *a: {'infrastructure_errors': [], 'transitions': None, 'contract': None, 'summary_lines': []}):
            for key in ('COMPOSE_PROJECT_NAME', 'SIM_BOT_ARGS_JSON'):
                if not (environ or {}).get(key):
                    os.environ.pop(key, None)
            with open(os.devnull, 'w') as sink, contextlib.redirect_stdout(sink):
                failed = ci.main()
        return failed, seen, calls, json.loads((self.root / 'suite/summary.json').read_text())

    def test_parallel_lanes_isolate_stack_api_and_user_dir_and_merge_deterministically(self):
        failed, seen, calls, summary = self.run_main('--parallel', '2', '--lane-port-base', '31000')
        self.assertTrue(failed)  # every patched wrapper exits 2
        self.assertEqual([row['unit'] for row in summary['units']], ['0b-click-move', '1-target-attack', '3-pvp-flag', '4-potion', 'z-last'])
        self.assertEqual(summary['parallel'], 2)
        self.assertEqual([lane['index'] for lane in summary['lanes']], [0, 1])
        self.assertEqual(sorted(u for lane in summary['lanes'] for u in lane['units']), [row['unit'] for row in summary['units']])
        by_unit = {}
        for command, env in seen:
            name = Path(command[command.index('--artifacts') + 1]).name
            by_unit[name] = env
        self.assertEqual(set(by_unit), {'0b-click-move', '1-target-attack', '3-pvp-flag', '4-potion', 'z-last'})
        for row in summary['units']:
            env = by_unit[row['unit']]
            lane = summary['lanes'][row['lane']]
            self.assertEqual(row['lane_ports'], lane['ports'])
            self.assertEqual(row['compose_project'], f"nightfall-sim-phase4-lane{row['lane']}")
            args = json.loads(env['SIM_BOT_ARGS_JSON'])
            self.assertIn(f"-UserDir={self.root / 'suite/.lanes' / f"lane-{row['lane']}" / 'ue-user'}", args)
            self.assertEqual(env['SIM_REQUIRE_OWNED_API'], '1')
            self.assertEqual(env['SIM_API_WAIT'], str(ci.API_WAIT))
            if row['unit'] == '4-potion':  # owned fixture unit: own ports, no lane stack
                self.assertNotIn('SIM_API_URL', env)
                self.assertNotIn('COMPOSE_FILE', env)
                self.assertFalse(any('GrpcEndpoint' in a for a in args))
            else:
                self.assertEqual(env['COMPOSE_PROJECT_NAME'], row['compose_project'])
                self.assertEqual(env['COMPOSE_FILE'], str(self.root / 'suite/.lanes' / f"lane-{row['lane']}" / 'compose.json'))
                self.assertEqual(env['SIM_API_URL'], f"http://localhost:{lane['ports']['api_http']}")
                self.assertEqual(env['NATS_URL'], f"nats://127.0.0.1:{lane['ports']['nats']}")
                self.assertTrue(any(a.endswith(f":{lane['ports']['api_grpc']}") for a in args))
                self.assertTrue(Path(env['SIM_API_ENV_FILE']).is_file())
        # The pair is one unit in one lane; the multi wrapper got both roles.
        pair = next(c for c, _ in seen if '3-pvp-flag-a.nfs' in ' '.join(c))
        self.assertIn('run-sim-multi.sh', pair[1])
        self.assertTrue(any('3-pvp-flag-b.nfs' in a for a in pair))
        # Compose down/logs per shared unit ran under the lane project; each lane that ran a shared unit tore down once.
        downs = [(c, env) for c, env in calls if c[:3] == ['docker', 'compose', 'down']]
        shared_lanes = {row['lane'] for row in summary['units'] if row['unit'] != '4-potion'}
        self.assertEqual(sum(1 for c, env in downs if env['COMPOSE_FILE'].endswith('compose.json')), 4 + len(shared_lanes))
        for lane in summary['lanes']:
            self.assertEqual(lane['teardown_exit'], 0 if lane['index'] in shared_lanes else None)
            self.assertTrue((self.root / 'suite/.lanes' / f"lane-{lane['index']}" / 'api.env').is_file())
        suite = ET.parse(self.root / 'suite/suite.xml').getroot()
        self.assertEqual([case.get('name') for case in suite.findall('testcase')], [row['unit'] for row in summary['units']])
        self.assertTrue(all(case.find('failure') is not None for case in suite.findall('testcase')))

    def test_serial_default_records_lane_zero_without_lane_files(self):
        failed, seen, calls, summary = self.run_main(environ={'COMPOSE_PROJECT_NAME': 'nightfall-sim-test',
                                                               'SIM_API_ENV_FILE': '/operator/stale.env'})
        self.assertEqual(summary['parallel'], 1)
        self.assertEqual([row['lane'] for row in summary['units']], [0] * 5)
        self.assertTrue(all(row['lane_ports'] is None for row in summary['units']))
        self.assertEqual(summary['lanes'][0]['compose_project'], 'nightfall-sim-test')
        self.assertFalse((self.root / 'suite/.lanes').exists())
        for command, env in seen:
            self.assertNotIn('SIM_API_ENV_FILE', env)
            self.assertNotIn('SIM_BOT_ARGS_JSON', env)
            self.assertEqual(env['COMPOSE_PROJECT_NAME'], 'nightfall-sim-test')

    def test_parallel_refuses_attach_mode_and_busy_blocks(self):
        with open(os.devnull, 'w') as sink, contextlib.redirect_stderr(sink):
            with patch('sys.argv', [a for a in self.args('--parallel', '2') if a != '--fresh-stack']), \
                    self.assertRaises(SystemExit):
                ci.main()
            with self.assertRaises(SystemExit):
                self.run_main('--parallel', '2', probe=lambda port: port % lanes.STRIDE == 3)
        self.assertFalse((self.root / 'suite').exists())  # failed fast, before any artifact


if __name__ == '__main__':
    unittest.main()
