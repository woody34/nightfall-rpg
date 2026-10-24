#!/usr/bin/env python3
"""Exact migration000011 pending-kit manifests for manually seeded legacy characters."""
import hashlib
import json
from pathlib import Path
import tomllib
from uuid import UUID

REPO=Path(__file__).resolve().parents[2]


def manifest(character,class_id):
    character=str(UUID(character))
    data=tomllib.loads((REPO/'packages/data/tables-phase4/starter_kits.toml').read_text())
    kit=next(row for row in data['kits'] if row['class_id']==class_id)
    payload=dict(source_class_id=class_id,items=[{k:r[k] for k in ('template_id','count','equipped')} for r in kit['items']])
    encoded=json.dumps(payload,sort_keys=True,separators=(',',':'))
    digest=hashlib.sha256(encoded.encode()).hexdigest()
    request=str(UUID(hashlib.md5(('nightfall.phase4.backfill.1|'+character).encode()).hexdigest()))
    return dict(request_id=request,character_id=character,kind='backfill',request_fingerprint='v1|backfill|'+character+'|'+digest,
                manifest_version=1,manifest_hash=digest,payload=payload)


def seed_sql(roles):
    result=[]
    for role in roles:
        value=manifest(role['character_id'],role['base_class_id'])
        quote=lambda s:"'"+str(s).replace("'","''")+"'"
        fields=[quote(value[k]) for k in ('request_id','character_id','kind','request_fingerprint')]
        fields += ['1',quote(value['manifest_hash']),quote(json.dumps(value['payload'],sort_keys=True,separators=(',',':')))+'::jsonb']
        result.append('INSERT INTO pending_item_requests(request_id,character_id,kind,request_fingerprint,manifest_version,manifest_hash,payload) VALUES ('+','.join(fields)+');')
    return '\n'.join(result)+'\n'
