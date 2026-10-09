#!/usr/bin/env python3
"""Give each soak role a radial lane in ONE zone; retain every kill-scenario assertion."""
import argparse
import math
from pathlib import Path
import re
import shutil


def prepare(repo: Path, out: Path, clients: int) -> None:
    if not 1 <= clients <= 8:
        raise ValueError("clients must be between 1 and 8")
    data = out / "data"
    shutil.copytree(repo / "packages/data", data, dirs_exist_ok=True)
    source = (repo / "apps/client-unreal/Scenarios/1-kill-one-monster.nfs").read_text()
    if len(re.findall(r'^nf.ClickMove ', source, re.M)) != 3 or len(re.findall(r'^nf.WaitFor own_at ', source, re.M)) != 3:
        raise ValueError('kill scenario route changed; review soak lane adaptation')
    zone = ['id = "test_zone"', 'zone_id = 1', 'name = "Nightfall soak"',
            '[bounds]', 'min = [0, 0]', 'max = [256, 256]', '[safe_point]', 'pos = [0, 0]']
    for i in range(clients):
        # At most 8 roles: >=35 tiles between homes. This includes both NPCs'
        # maximum wander (sqrt(2)*9.375 each; domain/zone/ai.rs) plus clan-help(8).
        angle = math.radians(5 + 80 * i / max(1, clients - 1))
        home = (round(180 * math.cos(angle)), round(180 * math.sin(angle)))
        end = (round(177 * math.cos(angle)), round(177 * math.sin(angle)))
        waypoints = [(round(end[0] * n / 3), round(end[1] * n / 3)) for n in (1, 2, 3)]
        zone += ['[[spawn_slots]]', f'id = "soak_{i + 1:02}"', 'template = "keltir"',
                 f'home = [{home[0]}, {home[1]}]', 'count = 1',
                 'respawn_delay_secs = 5', 'respawn_random_secs = 0']
        moves = iter(waypoints)
        waits = iter(waypoints)
        scenario = re.sub(r'^nf.ClickMove .+$', lambda _: 'nf.ClickMove %d %d' % next(moves), source, flags=re.M)
        scenario = re.sub(r'^nf.WaitFor own_at .+$', lambda _: 'nf.WaitFor own_at %d %d 1 40' % next(waits), scenario, flags=re.M)
        scenario = '# Soak lane: movement coordinates only; all combat assertions are unchanged.\n' + scenario
        path = out / f'client-{i + 1:02}' / 'Scenarios' / '1-kill-one-monster.nfs'
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(scenario)
    (data / 'zones/test_zone.toml').write_text('\n'.join(zone) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('repo', type=Path)
    parser.add_argument('out', type=Path)
    parser.add_argument('clients', type=int)
    args = parser.parse_args()
    prepare(args.repo, args.out, args.clients)
