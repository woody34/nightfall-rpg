//! Exact TOML/CSV class catalog loader. Source literals are converted once at startup.
//! The canonical hash includes all resolved metadata, growth and pinned provenance.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::exact_decimal::parse_decimal;
use crate::domain::class::{
    ClassDef, ClassId, ClassRegistry, GrowthRow, KnownSkillDef, RaceCollision, RaceDef,
    RaceMovement, RaceTraits, RegistryError,
};
use crate::domain::Race;

include!("class_data_embedded.rs");

/// Source text keyed by catalog-relative path. Independent from legacy Phase 1 rule files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassSource(pub BTreeMap<String, String>);

impl ClassSource {
    /// All shipped catalog files compiled into the executable.
    #[must_use]
    pub fn embedded() -> Self {
        Self(
            EMBEDDED_CLASSES
                .iter()
                .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
                .collect(),
        )
    }

    /// Reads only race, profession and growth directories in a `packages/data` tree.
    pub fn from_dir(dir: &Path) -> anyhow::Result<Self> {
        use anyhow::Context as _;
        let mut files = BTreeMap::new();
        let path = dir.join("skill_catalog.toml");
        if path.exists() {
            files.insert(
                "skill_catalog.toml".to_owned(),
                std::fs::read_to_string(&path)
                    .with_context(|| format!("read {}", path.display()))?,
            );
        }
        for (folder, extension) in [
            ("races", "toml"),
            ("professions", "toml"),
            ("growth", "csv"),
        ] {
            let path = dir.join(folder);
            for entry in
                std::fs::read_dir(&path).with_context(|| format!("read {}", path.display()))?
            {
                let file = entry?.path();
                if file.extension().is_some_and(|e| e == extension) {
                    let name = file
                        .file_name()
                        .ok_or_else(|| anyhow::anyhow!("missing file name"))?
                        .to_string_lossy();
                    let text = std::fs::read_to_string(&file)
                        .with_context(|| format!("read {}", file.display()))?;
                    files.insert(format!("{folder}/{name}"), text);
                }
            }
        }
        Ok(Self(files))
    }
}

/// Immutable validated catalog plus canonical data identity for snapshots and client caches.
#[derive(Debug, Clone)]
pub struct ResolvedClasses {
    /// Validated catalog; serialize into snapshots for replay without a filesystem dependency.
    pub registry: Arc<ClassRegistry>,
    /// SHA-256 of parsed semantic values and source provenance.
    pub config_hash: String,
}

/// Every parse, provenance and semantic failure found during startup.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid class data: {0:?}")]
pub struct ClassDataError(pub Vec<String>);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceRef {
    repository: String,
    revision: String,
    file: String,
    symbol: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfessionFile {
    source: SourceRef,
    #[serde(flatten)]
    definition: ClassDef,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CollisionFile {
    radius_male: String,
    radius_female: String,
    height_male: String,
    height_female: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RaceFile {
    source: SourceRef,
    id: Race,
    display_name: String,
    l2_ref: String,
    starting_zone: String,
    reference_starting_zone: String,
    start_points: Vec<[i32; 2]>,
    mystic_path: bool,
    base_class_ids: Vec<ClassId>,
    passive_skill_keys: Vec<String>,
    movement: RaceMovement,
    collision: CollisionFile,
    traits: RaceTraits,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillCatalogFile {
    source: SourceRef,
    skills: Vec<KnownSkillDef>,
}

/// Loads all files, rejects malformed decimals, then validates the complete catalog.
pub fn load_classes(source: &ClassSource) -> Result<ResolvedClasses, ClassDataError> {
    let mut errors = Vec::new();
    let mut classes = Vec::new();
    let mut races = Vec::new();
    let mut growth = BTreeMap::new();
    let mut provenance = BTreeMap::new();
    let mut known_skills = Vec::new();
    for (path, text) in &source.0 {
        if path == "skill_catalog.toml" {
            match toml::from_str::<SkillCatalogFile>(text) {
                Ok(file) => {
                    check_source(path, &file.source, &mut errors);
                    provenance.insert(path.clone(), file.source);
                    known_skills = file.skills;
                },
                Err(e) => errors.push(format!("{path}: {e}")),
            }
        } else if path.starts_with("professions/")
            && Path::new(path).extension().is_some_and(|e| e == "toml")
        {
            match toml::from_str::<ProfessionFile>(text) {
                Ok(file) => {
                    check_source(path, &file.source, &mut errors);
                    if path != &format!("professions/{}.toml", file.definition.key) {
                        errors.push(format!("{path}: key must match file name"));
                    }
                    provenance.insert(path.clone(), file.source);
                    classes.push(file.definition);
                },
                Err(e) => errors.push(format!("{path}: {e}")),
            }
        } else if path.starts_with("races/")
            && Path::new(path).extension().is_some_and(|e| e == "toml")
        {
            match toml::from_str::<RaceFile>(text) {
                Ok(file) => {
                    check_source(path, &file.source, &mut errors);
                    if path != &format!("races/{}.toml", file.id.as_str()) {
                        errors.push(format!("{path}: race must match file name"));
                    }
                    provenance.insert(path.clone(), file.source.clone());
                    match race(file) {
                        Ok(race) => races.push(race),
                        Err(e) => errors.push(format!("{path}: {e}")),
                    }
                },
                Err(e) => errors.push(format!("{path}: {e}")),
            }
        } else if path.starts_with("growth/")
            && Path::new(path).extension().is_some_and(|e| e == "csv")
        {
            match parse_growth(text) {
                Ok(rows) => {
                    growth.insert(
                        path.trim_start_matches("growth/")
                            .trim_end_matches(".csv")
                            .to_owned(),
                        rows,
                    );
                },
                Err(e) => errors.push(format!("{path}: {e}")),
            }
        } else {
            errors.push(format!("{path}: unknown catalog file"));
        }
    }
    if !errors.is_empty() {
        return Err(ClassDataError(errors));
    }
    let registry = ClassRegistry::new_with_skills(classes, races, growth, known_skills).map_err(
        |e| match e {
            RegistryError::Invalid(errors) => ClassDataError(errors),
            other @ (RegistryError::UnknownClass(_) | RegistryError::LevelOutOfRange(_)) => {
                ClassDataError(vec![other.to_string()])
            },
        },
    )?;
    let bytes = serde_json::to_vec(&(&registry, &provenance))
        .map_err(|e| ClassDataError(vec![format!("canonical catalog encoding: {e}")]))?;
    Ok(ResolvedClasses {
        registry: Arc::new(registry),
        config_hash: format!("sha256:{:x}", Sha256::digest(bytes)),
    })
}

/// Loads an external catalog; any I/O or validation failure aborts startup.
pub fn load_classes_dir(dir: &Path) -> anyhow::Result<ResolvedClasses> {
    Ok(load_classes(&ClassSource::from_dir(dir)?)?)
}

fn check_source(path: &str, source: &SourceRef, errors: &mut Vec<String>) {
    if source.repository != "https://bitbucket.org/l2jserver/l2j-server-datapack"
        || source.revision != "3ca488dd2bd0bfaca43e378886a3c2e37968153a"
        || !(if path == "skill_catalog.toml" {
            source.file == "src/main/resources/data/skillTrees/classSkillTree.xml"
        } else {
            source
                .file
                .starts_with("src/main/resources/data/stats/chars/")
        })
        || source.symbol.is_empty()
    {
        errors.push(format!("{path}: invalid pinned High Five provenance"));
    }
}

fn race(file: RaceFile) -> Result<RaceDef, String> {
    let c = file.collision;
    Ok(RaceDef {
        id: file.id,
        display_name: file.display_name,
        l2_ref: file.l2_ref,
        starting_zone: file.starting_zone,
        reference_starting_zone: file.reference_starting_zone,
        start_points: file.start_points,
        mystic_path: file.mystic_path,
        base_class_ids: file.base_class_ids,
        passive_skill_keys: file.passive_skill_keys,
        movement: file.movement,
        collision: RaceCollision {
            radius_male: parse_decimal(&c.radius_male)?,
            radius_female: parse_decimal(&c.radius_female)?,
            height_male: parse_decimal(&c.height_male)?,
            height_female: parse_decimal(&c.height_female)?,
        },
        traits: file.traits,
    })
}

fn parse_growth(text: &str) -> Result<Vec<GrowthRow>, String> {
    let mut lines = text.lines();
    if lines.next() != Some("level,hp,mp,cp") {
        return Err("expected level,hp,mp,cp CSV header".into());
    }
    let mut rows = Vec::new();
    for (index, line) in lines.enumerate() {
        let cells: Vec<_> = line.split(',').collect();
        let [level, hp, mp, cp] = cells.as_slice() else {
            return Err(format!("row {index}: expected four cells"));
        };
        let expected = index
            .checked_add(1)
            .ok_or_else(|| "too many rows".to_owned())?;
        if level.parse::<usize>() != Ok(expected) {
            return Err(format!("row {index}: levels must be contiguous from 1"));
        }
        rows.push(GrowthRow {
            hp: parse_decimal(hp)?,
            mp: parse_decimal(mp)?,
            cp: parse_decimal(cp)?,
        });
    }
    Ok(rows)
}

#[cfg(test)]
#[path = "class_data_tests.rs"]
mod tests;
