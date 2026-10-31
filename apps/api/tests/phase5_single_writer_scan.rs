//! Story 4.5 / E31 single-writer rule (design §10.0): no API code path writes a lease-governed
//! row. The only writer of warehouse, freight, mail, auction, manor and delivery rows is the
//! zone checkpoint transaction (`infrastructure/postgres/economy.rs` `apply_delta` /
//! `checkpoint_in`). This test scans every API source file.
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
const CONTAINER_LOCATIONS: &[&str] = &["warehouse", "freight", "mail", "delivery", "auction_bid"];

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

/// `(line, text)` of every statement writing a lease-governed row.
fn writes(text: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let lower = text.to_lowercase();
    for verb in ["insert into", "update", "delete from"] {
        let mut from = 0;
        while let Some(at) = lower[from..].find(verb) {
            let start = from + at;
            from = start + verb.len();
            let rest = lower[from..].trim_start();
            let table: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            let governed = GOVERNED.contains(&table.as_str());
            let container_item = table == "item_instances" && verb != "delete from" && {
                let statement: String = lower[start..].chars().take(400).collect();
                CONTAINER_LOCATIONS
                    .iter()
                    .any(|loc| statement.contains(&format!("'{loc}'")))
            };
            if governed || container_item {
                let line = lower[..start].matches('\n').count() + 1;
                found.push((line, format!("{verb} {table}")));
            }
        }
    }
    for entity in GOVERNED {
        for write in [
            "ActiveModel",
            "Entity::insert",
            "Entity::update",
            "Entity::delete",
        ] {
            let needle = format!("{entity}::{write}");
            for (index, line) in text.lines().enumerate() {
                if line.contains(&needle) {
                    found.push((index + 1, needle.clone()));
                }
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

#[test]
fn the_scanner_flags_a_synthetic_writer() {
    let bad = "fn x() { db.execute(\"UPDATE mail_messages SET read=true WHERE id=$1\"); }\nfn y() { q(\"INSERT INTO item_instances(id,location) VALUES($1,'warehouse')\"); }\nlet m = mail_messages::ActiveModel { ..Default::default() };";
    let found = writes(bad);
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(
        writes("SELECT * FROM mail_messages WHERE id=$1").is_empty(),
        "reads are allowed"
    );
    assert!(
        writes("UPDATE item_instances SET location='inventory' WHERE character_id=$1").is_empty(),
        "V6 inventory writes are not container writes"
    );
}

#[test]
fn no_api_code_path_writes_a_lease_governed_row() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);
    assert!(files.len() > 100, "scanned {} files", files.len());
    let writer = root.join("infrastructure/postgres/economy.rs");
    let mut violations = Vec::new();
    let mut writer_hits = 0;
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let hits = writes(&text);
        if *file == writer {
            let ranges = allowed_ranges(&text);
            assert_eq!(ranges.len(), 2, "apply_delta and checkpoint_in found");
            for (line, what) in hits {
                writer_hits += 1;
                if !ranges.iter().any(|(a, b)| (*a..=*b).contains(&line)) {
                    violations.push(format!(
                        "{}:{line}: {what} outside the checkpoint writer",
                        file.display()
                    ));
                }
            }
        } else {
            for (line, what) in hits {
                violations.push(format!("{}:{line}: {what}", file.display()));
            }
        }
    }
    assert!(
        writer_hits >= 10,
        "the checkpoint writer itself was scanned ({writer_hits} writes)"
    );
    assert!(
        violations.is_empty(),
        "E31 single-writer violations:\n{}",
        violations.join("\n")
    );
}
