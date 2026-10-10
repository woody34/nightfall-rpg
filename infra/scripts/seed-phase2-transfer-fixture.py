#!/usr/bin/env python3
"""Seed the isolated native transfer scenario BEFORE starting the API or issuing tickets.

Requires a freshly migrated, otherwise empty database named nf_phase2_fixture_* on a
non-default port. Never modifies an admitted character. The API must also be started
with AUTH_DEV_TOKENS=1. Use test:<owner-account> and test:<observer-account> to log in,
then select the fixed character IDs printed below. No mutable development RPC exists.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib
from urllib.parse import urlparse
from uuid import UUID

OWNER = "01970000-0000-7000-8000-000000000020"
OBSERVER = "01970000-0000-7000-8000-000000000040"
PROFILES = {
    "phase2-transfer": (40, (1, 1), 3, None),
    "phase2-transfer-observer": (40, (1, 1), 3, None),
    "phase2-transfer-missing-token": (20, (0, 0), 1, None),
    "phase2-token-level20": (19, (0, 0), 0, 20),
    "phase2-token-level40": (39, (1, 0), 1, 40),
    "phase2-token-backfill": (40, (0, 0), 0, None),
    "phase2-token-jump19-40": (19, (0, 0), 0, 20),
}

# Independent pinned source XP/death oracle agreed with the native fixture owner.
# Rewards belong to the client-owned temporary NPC overlay, never production data.
CROSSING_ORACLES = {
    "phase2-token-level20": (10235, 10746, 20),
    "phase2-token-level40": (62751, 65888, 40),
    "phase2-token-jump19-40": (13892446, 14587068, 40),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--owner-account", required=True, type=UUID)
    parser.add_argument("--observer-account", type=UUID)
    parser.add_argument("--fixture", choices=tuple(PROFILES), default="phase2-transfer-observer")
    parser.add_argument("--dry-run", action="store_true", help="validate guards and print SQL without connecting")
    args = parser.parse_args()
    if os.environ.get("AUTH_DEV_TOKENS") != "1" or os.environ.get("NIGHTFALL_PHASE2_FIXTURE") != "1":
        parser.error("fixture refused: AUTH_DEV_TOKENS=1 and NIGHTFALL_PHASE2_FIXTURE=1 required")
    with_observer = args.fixture == "phase2-transfer-observer"
    if with_observer and args.observer_account is None:
        parser.error("observer fixture requires --observer-account")
    if args.owner_account == args.observer_account:
        parser.error("owner and observer require distinct accounts")
    url = os.environ.get("DATABASE_URL", "")
    parsed = urlparse(url)
    if (parsed.scheme not in ("postgres", "postgresql") or parsed.port in (None, 5432)
            or not re.fullmatch(r"/nf_phase2_fixture_[a-z0-9_]+", parsed.path)
            or parsed.query or parsed.fragment):
        parser.error("fixture refused: explicit non-5432 port and nf_phase2_fixture_* database required; no URL options")
    root = Path(__file__).resolve().parents[2]
    xp_rows = tomllib.loads((root / "packages/data/tables/experience.toml").read_text())["to_level"]
    level, tokens, mask, crossing = PROFILES[args.fixture]
    # The shipped progression table is the seed XP authority. Earned tokens are
    # supplied only by the runtime after a real kill of the owned overlay NPC.
    xp_level = crossing if crossing is not None else level
    threshold = next(row["xp"] for row in xp_rows if row["level"] == xp_level)
    xp = threshold - 1 if crossing is not None else threshold
    accounts = f"('{args.owner_account}')"
    rows = f"('{OWNER}','{args.owner_account}','ClassOwner','classowner','human',{level},40,30,43,21,11,25,126,126,{xp},NULL,NULL,'human_fighter',true,0,0,0,{tokens[0]},{tokens[1]},'female',{mask})"
    if with_observer:
        accounts += f", ('{args.observer_account}')"
        rows += f", ('{OBSERVER}','{args.observer_account}','ClassObserver','classobserver','human',1,40,30,43,21,11,25,126,126,0,NULL,NULL,'human_fighter',true,0,0,0,0,0,'male',0)"
    sql = f"""BEGIN;
SET LOCAL lock_timeout = '3s';
LOCK TABLE characters, account_sessions, play_tickets, zone_epochs IN ACCESS EXCLUSIVE MODE;
DO $$ BEGIN
 IF EXISTS (SELECT 1 FROM characters) OR EXISTS (SELECT 1 FROM account_sessions)
 OR EXISTS (SELECT 1 FROM play_tickets) OR EXISTS (SELECT 1 FROM zone_epochs)
 OR EXISTS (SELECT 1 FROM zone_snapshots) OR EXISTS (SELECT 1 FROM idempotency_keys)
 OR EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid())
 THEN RAISE EXCEPTION 'fixture requires a fresh empty database, before API startup and admission'; END IF;
END $$;
INSERT INTO accounts(id) VALUES {accounts} ON CONFLICT(id) DO NOTHING;
INSERT INTO characters(id,account_id,name,name_normalized,race,level,str,dex,con,\"int\",wit,men,
 pos_x,pos_y,xp,hp,mp,class_profile,alive,revision,base_class_id,current_class_id,
 token_tier_1_count,token_tier_2_count,sex,milestone_claimed_mask)
VALUES {rows};
INSERT INTO character_class_slots(character_id,slot,class_id,level,exp,sp)
SELECT id,0,current_class_id,level,xp,sp FROM characters;
INSERT INTO character_learned_skills(character_id,slot,skill_key,skill_level)
SELECT id,0,'racial.adaptable',1 FROM characters;
COMMIT;
"""
    print(f"PHASE2_FIXTURE_ENABLED pack={args.fixture} before-admission isolated fixture", file=sys.stderr)
    if args.dry_run:
        print(sql)
    else:
        # Connection data stays in the child environment, never the command line or logs.
        env = dict(os.environ, PGDATABASE=url, PGCONNECT_TIMEOUT="3", PGAPPNAME="phase2_fixture_seed")
        subprocess.run(["psql", "-X", "--set=ON_ERROR_STOP=1", "--quiet"], input=sql, text=True, env=env, check=True)
    manifest = {"fixture_enabled": True, "fixture": args.fixture, "dry_run": args.dry_run,
        "owner": {"account_id": str(args.owner_account), "character_id": OWNER, "level": level,
                  "class_id": 0, "sex": "female", "tokens": list(tokens), "milestone_claimed_mask": mask},
        "position": [126, 126], "transfers": [1, 2] if args.fixture in ("phase2-transfer", "phase2-transfer-observer") else []}
    if args.fixture.startswith("phase2-token-"):
        # New profiles publish the exact persisted seed, including NULL resources
        # (full derived maxima on admission). Legacy profile manifests stay stable.
        manifest["owner"].update(race="human", xp=xp, hp=None, mp=None, cp=0, sp=0,
                                 position=[126, 126])
        if crossing is not None:
            raw_xp, kill_xp, destination = CROSSING_ORACLES[args.fixture]
            manifest["crossing"] = {"level": destination, "threshold_xp": next(
                row["xp"] for row in xp_rows if row["level"] == destination),
                "seed_threshold_xp": threshold, "xp_margin": 1,
                "npc_raw_xp": raw_xp, "kill_xp": kill_xp,
                "expected_level": destination, "expected_xp": xp + kill_xp}
    if with_observer:
        manifest["observer"] = {"account_id": str(args.observer_account), "character_id": OBSERVER,
                                "level": 1, "class_id": 0, "sex": "male"}
    print(json.dumps(manifest, sort_keys=True))


if __name__ == "__main__":
    main()
