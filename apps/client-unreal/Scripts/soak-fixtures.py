#!/usr/bin/env python3
"""Give each real soak client a lane, with attack or received-state casting assertions."""
import argparse
import importlib.util
import json
import os
import uuid
from pathlib import Path
import re
import shutil


def casting_scenario(waypoints):
    lines = [
        '# covers: 3 E2.1, 3 E2.2, 3 E2.4, 3 E2.7, 3 E6.7',
        '# Soak casting role: fixture coordinates; received state only.',
        '# Pinned dp3ca488dd skills/01100-01199.xml:1645 and01000-01099.xml:1560:',
        '# Base cast4000ms *3=12s; A1 state10ticks *100ms *3=3s.',
        '# Seed at lane end (wp3); BotLoop retains that endpoint for later iterations.',
        '# Fixed top/bottom edge lanes: end -> wp2 -> wp1 -> wp2 -> wp3, each leg60tiles.',
        '# Homes/end >=93tiles from peace (Chebyshev); homes >=60tiles apart.',
        '# Peace: packages/data/zones/test_zone_phase3.toml:7-11 (inclusive square).',
        '# Wander: domain/zone/ai.rs:24-30, <=9.375tiles/axis (sqrt(2)*9.375 <13.3tiles).',
        '# Home spacing >2*13.3+8tiles clan-help (npcs/keltir.toml:26); combat stays clear.',
        '# For1..8 clients, legs <=60tiles; +1tile own_at tolerance <=61 <64tile move cap.',
        '# Move cap: domain/zone/state.rs:39,1376; tolerance: Bot/BotPredicates.cpp:681-691.',
        '# Move waits: (60+1)tiles /3.75tiles/s *3=48.8s <=54s per leg.',
        '# Speed: professions/human_mystic.toml:22; domain/zone/phase2.rs:110-114.',
        '# Global waits: 4*54s moves +99s other waits=315s; +30s grace=345s <=360s.',
        '# Long Might is1200s (pinned dp skills/01000-01099.xml:1553).',
        'nf.Login', 'nf.WaitFor connected 15', 'nf.EnterWorld', 'nf.WaitFor in_world 15',
        'nf.WaitFor learned_rank 1177 == 1 3',
        'nf.WaitFor skill_ready 1068 12', 'nf.Cast 1068 self', 'nf.WaitFor buff_active 1068 12']
    wp1, wp2, wp3 = waypoints
    for x, y in (wp2, wp1, wp2, wp3):
        lines += [f'nf.ClickMove {x} {y}', f'nf.WaitFor own_at {x} {y} 1 54']
    lines += [
        'nf.WaitFor proxies >= 1 3', 'nf.Target nearest_attackable', 'nf.WaitFor target != none 3',
        'nf.WaitFor skill_ready 1177 12', 'nf.Cast 1177',
        'nf.WaitFor cast_damage_results >= 1 12', 'nf.WaitFor target_hp == 0 3',
        'nf.WaitFor cast_state == idle 3', 'nf.Expect target == none',
        'nf.Expect buff_active 1068', 'nf.Expect cast_results >= 1',
        'nf.WaitFor xp_known 3', 'nf.WaitFor kill_xp_matches_fixture 3',
        'nf.Expect own_xp > 0', 'nf.Expect own_level_matches_xp', 'nf.Within 360']
    # Short reuse may already be ready when Result arrives. The start/reuse history
    # is verified in the focused cast scenario; soak never requires seeing a subsecond window.
    return '\n'.join(lines) + '\n'


def prepare(repo: Path, out: Path, clients: int, casting: bool = False) -> None:
    if not 1 <= clients <= 8:
        raise ValueError("clients must be between 1 and 8")
    data = out / "data"
    shutil.copytree(repo / "packages/data", data, dirs_exist_ok=True)
    source = (repo / "apps/client-unreal/Scenarios/1-kill-one-monster.nfs").read_text()
    if len(re.findall(r'^nf.ClickMove ', source, re.M)) != 3 or len(re.findall(r'^nf.WaitFor own_at ', source, re.M)) != 3:
        raise ValueError('kill scenario route changed; review soak lane adaptation')
    zone = ['id = "test_zone"', 'zone_id = 1', 'name = "Nightfall soak"',
            '[bounds]', 'min = [0, 0]', 'max = [256, 256]', '[safe_point]', 'pos = [0, 0]']
    roles = []
    spec = importlib.util.spec_from_file_location('phase3_soak_inputs', repo / 'apps/client-unreal/Scripts/phase3-fixtures.py')
    contracts = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(contracts)
    for i in range(clients):
        # At most 8 roles: >=35 tiles between homes. This includes both NPCs'
        # maximum wander (sqrt(2)*9.375 each; domain/zone/ai.rs) plus clan-help(8).
        home, waypoints = contracts.soak_lane(i, clients, casting=casting)
        zone += ['[[spawn_slots]]', f'id = "soak_{i + 1:02}"', 'template = "keltir"',
                 f'home = [{home[0]}, {home[1]}]', 'count = 1',
                 'respawn_delay_secs = 5', 'respawn_random_secs = 0']
        moves = iter(waypoints)
        waits = iter(waypoints)
        scenario = re.sub(r'^nf.ClickMove .+$', lambda _: 'nf.ClickMove %d %d' % next(moves), source, flags=re.M)
        scenario = re.sub(r'^nf.WaitFor own_at .+$', lambda _: 'nf.WaitFor own_at %d %d 1 40' % next(waits), scenario, flags=re.M)
        if casting:
            # Every role is pre-admission seeded; no repeated CreateClass/slot exhaustion.
            account, character = str(uuid.uuid4()), str(uuid.uuid4())
            seed = contracts.soak_profile(repo, account, character, i, clients)
            role_dir = out / f'client-{i + 1:02}'
            role_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
            token = role_dir / 'dev.token'
            with open(token, 'x', opener=lambda path, flags: os.open(path, flags, 0o600)) as file:
                file.write(f'test:{account}\n')
            roles.append({'name': f'client-{i + 1:02}', 'seed': seed, 'token_file': str(token.resolve()),
                          'scenario': str((role_dir / 'Scenarios/1-kill-one-monster.nfs').resolve())})
            scenario = casting_scenario(waypoints)
        path = out / f'client-{i + 1:02}' / 'Scenarios' / '1-kill-one-monster.nfs'
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(scenario)
    if casting:
        # Test input HP1 permits one real cast kill without calculating damage.
        npc = data / 'npcs/keltir.toml'
        npc.write_text(npc.read_text().replace('max_hp = 44', 'max_hp = 1').replace('aggressive = true', 'aggressive = false'))
        (out / 'roles.json').write_text(json.dumps({'schema_version': 1, 'casting': True, 'roles': roles}, indent=2) + '\n')
    (data / 'zones/test_zone.toml').write_text('\n'.join(zone) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('repo', type=Path)
    parser.add_argument('out', type=Path)
    parser.add_argument('clients', type=int)
    parser.add_argument('--casting', action='store_true')
    args = parser.parse_args()
    prepare(args.repo, args.out, args.clients, args.casting)
