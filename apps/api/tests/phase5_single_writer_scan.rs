//! Story 4.5 / E31 single-writer rule (design §10.0): no API code path writes a lease-governed
//! row. The only writer of warehouse, freight, mail, auction, manor and delivery rows is the
//! zone checkpoint transaction (`infrastructure/postgres/economy.rs` `apply_delta` /
//! `checkpoint_in`). This test scans every API source file.
//!
//! c-review-7 P2-5: the scan is allowlist-based. Every `item_instances` write (insert, update,
//! delete) is a violation, except in the economy writer and in the V6 settlement statements
//! of [`CHARACTER_WRITERS`], which must carry `owner_kind='character'`. Table names lose a
//! schema prefix and quotes, and an aliased sea-orm entity (`Entity as X`) is followed.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::path::{Path, PathBuf};

const GOVERNED: &[&str] = &[
    "mail_messages",
    "item_auctions",
    "item_auction_bids",
    "manor_seed_production",
    "manor_crop_procure",
    "economy_deliveries",
];
/// c-review-7 P2-5: every `item_instances` write is checked, not only the container ones.
const ITEMS: &str = "item_instances";
/// The V6 settlement statements outside the economy writer: each one must carry
/// `owner_kind='character'`, so it can never touch a container row.
const CHARACTER_WRITERS: &[&str] = &[
    "infrastructure/postgres/item_settlement.rs",
    "infrastructure/postgres/character_repository.rs",
];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            // Migrations create and seed the schema; generated entities only declare it.
            if !path.ends_with("migrations") {
                sources(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The table name after a write verb: quotes and a schema prefix are stripped
/// (`public.mail_messages`, `"public"."mail_messages"`).
fn table_after(rest: &str) -> String {
    let token: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '"' | '\\'))
        .collect();
    token
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// One write the scanner found.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Write {
    line: usize,
    what: String,
    /// The statement up to its closing string quote, without spaces (SQL only).
    statement: Option<String>,
}

impl Write {
    fn character_owned(&self) -> bool {
        self.statement
            .as_deref()
            .is_some_and(|s| s.contains("owner_kind='character'"))
    }
}

/// Every statement that writes a lease-governed row or an `item_instances` row.
fn writes(text: &str) -> Vec<Write> {
    let mut found = Vec::new();
    let lower = text.to_lowercase();
    for verb in ["insert into", "update", "delete from"] {
        let mut from = 0;
        while let Some(at) = lower[from..].find(verb) {
            let start = from + at;
            from = start + verb.len();
            let table = table_after(&lower[from..]);
            if GOVERNED.contains(&table.as_str()) || table == ITEMS {
                let line = lower[..start].matches('\n').count() + 1;
                let statement: String = lower[start..]
                    .chars()
                    .take_while(|c| *c != '"')
                    .filter(|c| !c.is_whitespace())
                    .collect();
                found.push(Write {
                    line,
                    what: format!("{verb} {table}"),
                    statement: Some(statement),
                });
            }
        }
    }
    // sea-orm: the entity paths, and any alias of a written entity (`Entity as X`).
    let mut entities: Vec<String> = GOVERNED
        .iter()
        .chain(std::iter::once(&ITEMS))
        .map(|e| format!("{e}::Entity"))
        .collect();
    for line in text.lines() {
        for table in GOVERNED.iter().chain(std::iter::once(&ITEMS)) {
            if let Some(at) = line.find(&format!("{table}::Entity as ")) {
                let alias: String = line[at + table.len() + "::Entity as ".len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !alias.is_empty() {
                    entities.push(alias);
                }
            }
        }
    }
    for (index, line) in text.lines().enumerate() {
        for entity in &entities {
            for write in ["insert", "update", "delete"] {
                let needle = format!("{entity}::{write}");
                let hit = line.match_indices(&needle).any(|(at, _)| {
                    // A whole path segment: `Mail::update_many`, not `OtherMail::update`.
                    at == 0
                        || !line[..at]
                            .chars()
                            .next_back()
                            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                });
                if hit {
                    found.push(Write {
                        line: index + 1,
                        what: needle,
                        statement: None,
                    });
                }
            }
        }
        for table in GOVERNED.iter().chain(std::iter::once(&ITEMS)) {
            let needle = format!("{table}::ActiveModel");
            if line.contains(&needle) {
                found.push(Write {
                    line: index + 1,
                    what: needle,
                    statement: None,
                });
            }
        }
    }
    found
}

/// Line ranges of the functions allowed to write: the zone checkpoint writer only.
fn allowed_ranges(text: &str) -> Vec<(usize, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut ranges = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.contains("async fn apply_delta(") || line.contains("async fn checkpoint_in(") {
            let end = lines[index..]
                .iter()
                .position(|l| *l == "}")
                .map_or(lines.len(), |p| index + p);
            ranges.push((index + 1, end + 1));
        }
    }
    ranges
}

/// The violations of one file (`rel` is its path under `src/`).
fn violations(rel: &str, text: &str) -> (Vec<String>, usize) {
    let hits = writes(text);
    let mut out = Vec::new();
    let mut writer_hits = 0;
    if rel == "infrastructure/postgres/economy.rs" {
        let ranges = allowed_ranges(text);
        assert_eq!(ranges.len(), 2, "apply_delta and checkpoint_in found");
        for hit in hits {
            writer_hits += 1;
            if !ranges.iter().any(|(a, b)| (*a..=*b).contains(&hit.line)) {
                out.push(format!("{rel}:{}: {} outside the checkpoint writer", hit.line, hit.what));
            }
        }
    } else {
        for hit in hits {
            let v6 = CHARACTER_WRITERS.contains(&rel)
                && hit.what.ends_with(ITEMS)
                && hit.character_owned();
            if !v6 {
                out.push(format!("{rel}:{}: {}", hit.line, hit.what));
            }
        }
    }
    (out, writer_hits)
}

#[test]
fn the_scanner_flags_a_synthetic_writer() {
    let bad = "fn x() { db.execute(\"UPDATE mail_messages SET read=true WHERE id=$1\"); }\nfn y() { q(\"INSERT INTO item_instances(id,location) VALUES($1,'warehouse')\"); }\nlet m = mail_messages::ActiveModel { ..Default::default() };";
    assert_eq!(writes(bad).len(), 3, "{:?}", writes(bad));
    assert!(
        writes("SELECT * FROM mail_messages WHERE id=$1").is_empty(),
        "reads are allowed"
    );
    // c-review-7 P2-5: the reviewer's probe shapes, each one flagged outside the allowlist.
    for probe in [
        "q(\"DELETE FROM item_instances WHERE owner_kind='warehouse' AND id=$1\")",
        "q(\"UPDATE item_instances SET owner_kind=$2, location=$3 WHERE id=$1\")",
        "q(\"UPDATE public.mail_messages SET read=true WHERE id=$1\")",
        "q(\"UPDATE \\\"public\\\".\\\"economy_deliveries\\\" SET status='delivered'\")",
        "use crate::infrastructure::postgres::entities::mail_messages::Entity as Mail;\nfn z() { Mail::update_many().exec(db); }",
        "fn w() { item_instances::Entity::update_many().exec(db); }",
        "fn v() { item_instances::Entity::delete_by_id(1).exec(db); }",
        "let a = item_instances::ActiveModel { ..Default::default() };",
        "q(\"UPDATE item_instances SET location='inventory' WHERE character_id=$1\")",
    ] {
        let (found, _) = violations("application/zz_probe.rs", probe);
        assert!(!found.is_empty(), "not flagged: {probe}");
    }
    // An alias that only reads is not a write.
    assert!(
        writes("use x::mail_messages::Entity as Mail;\nlet m = Mail::find().all(db);").is_empty()
    );
    // A V6 settlement statement carries owner_kind='character' in an allowlisted file only.
    let v6 = "q(\"DELETE FROM item_instances WHERE character_id=$1 AND id=$2 AND owner_kind='character'\")";
    assert!(violations("infrastructure/postgres/item_settlement.rs", v6)
        .0
        .is_empty());
    assert!(!violations("application/zz_probe.rs", v6).0.is_empty());
    let unguarded = "q(\"UPDATE item_instances SET location='inventory' WHERE character_id=$1\")";
    assert!(!violations("infrastructure/postgres/item_settlement.rs", unguarded)
        .0
        .is_empty());
}

#[test]
fn no_api_code_path_writes_a_lease_governed_row() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);
    assert!(files.len() > 100, "scanned {} files", files.len());
    let mut found = Vec::new();
    let mut writer_hits = 0;
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let rel = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let (bad, hits) = violations(&rel, &text);
        found.extend(bad);
        writer_hits += hits;
    }
    assert!(
        writer_hits >= 10,
        "the checkpoint writer itself was scanned ({writer_hits} writes)"
    );
    assert!(found.is_empty(), "E31 single-writer violations:\n{}", found.join("\n"));
}
