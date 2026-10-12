-- Pinned l2j-dp @3ca488dd data/stats/initialEquipment.xml:4-109.
-- Preserve XML row order, counts, equipped flags and base creation class. No item allocation.
CREATE TEMP TABLE phase4_backfill_kits (class_id integer PRIMARY KEY, manifest_hash text NOT NULL, payload jsonb NOT NULL) ON COMMIT DROP;
INSERT INTO phase4_backfill_kits VALUES
(0,'f4b9d0c917cf67c6cb1dde7d8a985c4d13cc217d6887e1cc66656dd218ee178d','{"items":[{"count":1,"equipped":true,"template_id":2369},{"count":1,"equipped":false,"template_id":10},{"count":1,"equipped":true,"template_id":1146},{"count":1,"equipped":true,"template_id":1147},{"count":1,"equipped":false,"template_id":5588},{"count":5,"equipped":false,"template_id":10650},{"count":10,"equipped":false,"template_id":12753}],"source_class_id":0}'::jsonb),
(10,'9e8c395654f2b4350c361c2862a4cdaddc2855f491d92e7c5fea72e4eb33c9de','{"items":[{"count":1,"equipped":true,"template_id":6},{"count":1,"equipped":true,"template_id":425},{"count":1,"equipped":true,"template_id":461},{"count":1,"equipped":false,"template_id":5588},{"count":5,"equipped":false,"template_id":10650},{"count":10,"equipped":false,"template_id":12753}],"source_class_id":10}'::jsonb),
(18,'0759a4565f5e0ff922bd28f3ec52ab28e447e0ad30f22069c7bc835789ca837b','{"items":[{"count":1,"equipped":true,"template_id":2369},{"count":1,"equipped":false,"template_id":10},{"count":1,"equipped":true,"template_id":1146},{"count":1,"equipped":true,"template_id":1147},{"count":1,"equipped":false,"template_id":5588},{"count":5,"equipped":false,"template_id":10650},{"count":10,"equipped":false,"template_id":12753}],"source_class_id":18}'::jsonb),
(25,'d20eff22bcff43c232f03d49120167d5ca6b83fa29bb214e7bfbe4933ef557a4','{"items":[{"count":1,"equipped":true,"template_id":6},{"count":1,"equipped":true,"template_id":425},{"count":1,"equipped":true,"template_id":461},{"count":1,"equipped":false,"template_id":5588},{"count":5,"equipped":false,"template_id":10650},{"count":10,"equipped":false,"template_id":12753}],"source_class_id":25}'::jsonb),
(31,'84541b9bccda1baa494db25775479096ceaea870e5c37a6b2d061d11d6f439d2','{"items":[{"count":1,"equipped":true,"template_id":2369},{"count":1,"equipped":false,"template_id":10},{"count":1,"equipped":true,"template_id":1146},{"count":1,"equipped":true,"template_id":1147},{"count":1,"equipped":false,"template_id":5588},{"count":5,"equipped":false,"template_id":10650},{"count":10,"equipped":false,"template_id":12753}],"source_class_id":31}'::jsonb),
(38,'33644086080a62ce711e4113667822b22aafe471f1397c5a82a0fb4416474b78','{"items":[{"count":1,"equipped":true,"template_id":6},{"count":1,"equipped":true,"template_id":425},{"count":1,"equipped":true,"template_id":461},{"count":1,"equipped":false,"template_id":5588},{"count":5,"equipped":false,"template_id":10650},{"count":10,"equipped":false,"template_id":12753}],"source_class_id":38}'::jsonb),
(44,'11a1f1578922f31c51ffd5c61c8350cbda3deb13352a20fac7b919266e7b7331','{"items":[{"count":1,"equipped":false,"template_id":2369},{"count":1,"equipped":true,"template_id":2368},{"count":1,"equipped":true,"template_id":1146},{"count":1,"equipped":true,"template_id":1147},{"count":1,"equipped":false,"template_id":5588},{"count":5,"equipped":false,"template_id":10650},{"count":10,"equipped":false,"template_id":12753}],"source_class_id":44}'::jsonb),
(49,'5b685fcca9c2315db7d1bd900a267a514dcaf5fd7fb345298cb14f768fd00818','{"items":[{"count":1,"equipped":true,"template_id":2368},{"count":1,"equipped":true,"template_id":425},{"count":1,"equipped":true,"template_id":461},{"count":1,"equipped":false,"template_id":5588},{"count":5,"equipped":false,"template_id":10650},{"count":10,"equipped":false,"template_id":12753}],"source_class_id":49}'::jsonb),
(53,'c36987d88abb03e33888ffb2728850df4f33fc51017567d2c1f23f121042849e','{"items":[{"count":1,"equipped":true,"template_id":2370},{"count":1,"equipped":false,"template_id":10},{"count":1,"equipped":true,"template_id":1146},{"count":1,"equipped":true,"template_id":1147},{"count":1,"equipped":false,"template_id":5588},{"count":5,"equipped":false,"template_id":10650},{"count":10,"equipped":false,"template_id":12753}],"source_class_id":53}'::jsonb);
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM characters c LEFT JOIN phase4_backfill_kits k ON k.class_id=c.base_class_id WHERE k.class_id IS NULL) THEN
        RAISE EXCEPTION 'phase4 backfill: missing pinned starter kit for base class';
    END IF;
    IF EXISTS (SELECT 1 FROM pending_item_requests p JOIN characters c ON c.id=p.character_id JOIN phase4_backfill_kits k ON k.class_id=c.base_class_id WHERE p.kind IN ('starter','backfill') AND (p.manifest_version<>1 OR p.manifest_hash<>k.manifest_hash OR p.payload<>k.payload)) THEN
        RAISE EXCEPTION 'phase4 backfill: existing manifest identity mismatch';
    END IF;
END $$;
INSERT INTO pending_item_requests (request_id,character_id,kind,request_fingerprint,manifest_version,manifest_hash,payload)
SELECT md5('nightfall.phase4.backfill.1|' || c.id::text)::uuid,c.id,'backfill',
       'v1|backfill|' || c.id::text || '|' || k.manifest_hash,1,k.manifest_hash,k.payload
FROM characters c JOIN phase4_backfill_kits k ON k.class_id=c.base_class_id
ON CONFLICT DO NOTHING;
