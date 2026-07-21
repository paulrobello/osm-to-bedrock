//! User-supplied overrides for the OSM tag → Block mappings.
//!
//! Loaded from a YAML file referenced by the `--block-mapping` CLI flag. See
//! `docs/superpowers/specs/2026-07-21-custom-block-mappings-design.md` for the
//! format and semantics.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::blocks::{Block, BlockOverrides};

/// Load [`BlockOverrides`] from a YAML file.
///
/// Returns `Ok(None)` when the file does not exist, and `Err` on any parse,
/// I/O, or validation error (unknown block name, unknown top-level key).
pub fn load_block_overrides(path: &Path) -> Result<Option<BlockOverrides>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading block mapping file {}", path.display()))?;
    let raw: RawBlockOverrides = serde_yaml_ng::from_str(&text)
        .with_context(|| format!("parsing block mapping file {}", path.display()))?;
    Ok(Some(resolve(raw, path)?))
}

/// Load overrides for the `--block-mapping` CLI flag.
///
/// `Ok(None)` when the flag was not set (`path` is `None`). If the flag was set
/// but the file is missing or invalid, this is a hard error.
pub fn load_block_overrides_arg(path: &Option<PathBuf>) -> Result<Option<BlockOverrides>> {
    match path {
        Some(p) => match load_block_overrides(p)? {
            Some(o) => Ok(Some(o)),
            None => bail!("block mapping file not found: {}", p.display()),
        },
        None => Ok(None),
    }
}

/// Intermediate deserialiser: tag-value → block-name strings, before the names
/// are resolved to `Block` variants. `deny_unknown_fields` catches top-level
/// typos like `buliding:`.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBlockOverrides {
    #[serde(default)]
    building: HashMap<String, String>,
    #[serde(default)]
    highway: HashMap<String, String>,
    #[serde(default)]
    landuse: HashMap<String, String>,
    #[serde(default)]
    natural: HashMap<String, String>,
}

/// Resolve every block name in `raw` to a `Block`, producing a `BlockOverrides`.
fn resolve(raw: RawBlockOverrides, path: &Path) -> Result<BlockOverrides> {
    Ok(BlockOverrides {
        building: resolve_map(raw.building, "building", path)?,
        highway: resolve_map(raw.highway, "highway", path)?,
        landuse: resolve_map(raw.landuse, "landuse", path)?,
        natural: resolve_map(raw.natural, "natural", path)?,
    })
}

fn resolve_map(
    raw: HashMap<String, String>,
    category: &str,
    path: &Path,
) -> Result<HashMap<String, Block>> {
    let mut out = HashMap::with_capacity(raw.len());
    for (tag_value, block_name) in raw {
        let block = Block::from_name(&block_name).ok_or_else(|| {
            anyhow::anyhow!(
                "{file}: unknown block name \"{name}\" under \"{cat}\" (key \"{key}\"). \
                 Use a Block variant name, e.g. OakLog, PolishedBlackstoneSlab, Water.",
                file = path.display(),
                name = block_name,
                cat = category,
                key = tag_value,
            )
        })?;
        out.insert(tag_value, block);
    }
    Ok(out)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::Block;
    use std::io::Write as _;

    #[test]
    fn missing_file_returns_none() {
        let result =
            load_block_overrides(Path::new("/tmp/__nonexistent_block_mapping__.yaml")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn empty_file_yields_default() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp).unwrap();
        let ov = load_block_overrides(tmp.path()).unwrap().unwrap();
        assert_eq!(ov, BlockOverrides::default());
    }

    #[test]
    fn parses_all_four_sections() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            tmp,
            "building:\n  brick: OakPlanks\nhighway:\n  motorway: SmoothStoneSlab\n\
             landuse:\n  farmland: Dirt\nnatural:\n  wood: OakLog\n"
        )
        .unwrap();
        let ov = load_block_overrides(tmp.path()).unwrap().unwrap();
        assert_eq!(ov.building.get("brick"), Some(&Block::OakPlanks));
        assert_eq!(ov.highway.get("motorway"), Some(&Block::SmoothStoneSlab));
        assert_eq!(ov.landuse.get("farmland"), Some(&Block::Dirt));
        assert_eq!(ov.natural.get("wood"), Some(&Block::OakLog));
    }

    #[test]
    fn partial_file_parses() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "building:\n  glass: GlassPane\n").unwrap();
        let ov = load_block_overrides(tmp.path()).unwrap().unwrap();
        assert_eq!(ov.building.get("glass"), Some(&Block::GlassPane));
        assert!(ov.highway.is_empty());
        assert!(ov.landuse.is_empty());
        assert!(ov.natural.is_empty());
    }

    #[test]
    fn unknown_block_name_errors() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "building:\n  brick: NotABlock\n").unwrap();
        let err = load_block_overrides(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("unknown block name"), "{msg}");
        assert!(msg.contains("NotABlock"), "{msg}");
        assert!(msg.contains("brick"), "{msg}");
    }

    #[test]
    fn unknown_top_level_key_errors() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "buliding:\n  brick: Brick\n").unwrap(); // typo
        assert!(load_block_overrides(tmp.path()).is_err());
    }

    #[test]
    fn load_arg_none_when_flag_unset() {
        assert_eq!(load_block_overrides_arg(&None).unwrap(), None);
    }

    #[test]
    fn load_arg_errors_when_flag_set_but_file_missing() {
        let path = PathBuf::from("/tmp/__definitely_missing_block_mapping__.yaml");
        let err = load_block_overrides_arg(&Some(path)).unwrap_err();
        assert!(format!("{err:#}").contains("block mapping file not found"));
    }

    #[test]
    fn load_arg_loads_when_file_present() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "building:\n  wood: SprucePlanks\n").unwrap();
        let ov = load_block_overrides_arg(&Some(tmp.path().to_path_buf()))
            .unwrap()
            .unwrap();
        assert_eq!(ov.building.get("wood"), Some(&Block::SprucePlanks));
    }
}
