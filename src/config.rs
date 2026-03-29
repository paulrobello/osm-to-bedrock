//! YAML configuration file support for osm-to-bedrock.
//!
//! Provides loading, merging, and dumping of configuration from
//! `.osm-to-bedrock.yaml` files in the current directory or the
//! user's config directory.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// All CLI-overridable parameters that can also be set in a YAML config file.
///
/// Every field is `Option<T>` so that absent YAML keys remain `None` and can
/// be distinguished from explicitly supplied values during config merging.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub scale: Option<f64>,
    pub sea_level: Option<i32>,
    pub building_height: Option<i32>,
    pub wall_straighten_threshold: Option<i32>,
    pub signs: Option<bool>,
    pub address_signs: Option<bool>,
    pub poi_markers: Option<bool>,
    pub poi_decorations: Option<bool>,
    pub nature_decorations: Option<bool>,
    pub vertical_scale: Option<f64>,
    pub elevation: Option<PathBuf>,
    pub overpass_url: Option<String>,
    pub no_roads: Option<bool>,
    pub no_buildings: Option<bool>,
    pub no_water: Option<bool>,
    pub no_landuse: Option<bool>,
    pub no_railways: Option<bool>,
    pub overture: Option<bool>,
    pub overture_themes: Option<String>,
    pub overture_timeout: Option<u64>,
    pub snow_line: Option<i32>,
    pub elevation_smoothing: Option<i32>,
    pub surface_thickness: Option<i32>,
}

/// Emit a merge expression for a single field.
macro_rules! merge_field {
    ($self:ident, $other:ident, $field:ident) => {
        if $self.$field.is_none() {
            $self.$field = $other.$field.clone();
        }
    };
}

impl Config {
    /// Fill every `None` field in `self` with the value from `other`.
    ///
    /// `self` is treated as the higher-priority source; `other` supplies
    /// defaults for any fields that `self` leaves unset.
    pub fn merge(&mut self, other: &Config) {
        merge_field!(self, other, scale);
        merge_field!(self, other, sea_level);
        merge_field!(self, other, building_height);
        merge_field!(self, other, wall_straighten_threshold);
        merge_field!(self, other, signs);
        merge_field!(self, other, address_signs);
        merge_field!(self, other, poi_markers);
        merge_field!(self, other, poi_decorations);
        merge_field!(self, other, nature_decorations);
        merge_field!(self, other, vertical_scale);
        merge_field!(self, other, elevation);
        merge_field!(self, other, overpass_url);
        merge_field!(self, other, no_roads);
        merge_field!(self, other, no_buildings);
        merge_field!(self, other, no_water);
        merge_field!(self, other, no_landuse);
        merge_field!(self, other, no_railways);
        merge_field!(self, other, overture);
        merge_field!(self, other, overture_themes);
        merge_field!(self, other, overture_timeout);
        merge_field!(self, other, snow_line);
        merge_field!(self, other, elevation_smoothing);
        merge_field!(self, other, surface_thickness);
    }

    /// Load a `Config` from a YAML file at `path`.
    ///
    /// Returns `Ok(None)` when the file does not exist, and `Err` on any
    /// parse or I/O error.
    pub fn from_file(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file {}", path.display()))?;
        let cfg: Config = serde_yaml_ng::from_str(&text)
            .with_context(|| format!("parsing config file {}", path.display()))?;
        Ok(Some(cfg))
    }

    /// Resolve and load the configuration using the following priority chain:
    ///
    /// 1. `explicit_path` — if supplied, load this file; error if it is absent.
    /// 2. `.osm-to-bedrock.yaml` in the current working directory.
    /// 3. `~/.config/osm-to-bedrock/config.yaml`.
    /// 4. `Config::default()` (all `None`) if none of the above exist.
    pub fn load(explicit_path: Option<&Path>) -> Result<Self> {
        if let Some(path) = explicit_path {
            return Self::from_file(path)?
                .ok_or_else(|| anyhow::anyhow!("config file not found: {}", path.display()));
        }

        // Try CWD local file.
        let cwd_path = PathBuf::from(".osm-to-bedrock.yaml");
        if let Some(cfg) = Self::from_file(&cwd_path)? {
            log::debug!("Loaded config from {}", cwd_path.display());
            return Ok(cfg);
        }

        // Try user config directory.
        if let Some(cfg_dir) = dirs_path() {
            let user_path = cfg_dir.join("config.yaml");
            if let Some(cfg) = Self::from_file(&user_path)? {
                log::debug!("Loaded config from {}", user_path.display());
                return Ok(cfg);
            }
        }

        Ok(Config::default())
    }

    /// Serialise the resolved configuration to YAML and print it to stdout.
    pub fn dump(&self) -> Result<()> {
        let yaml = serde_yaml_ng::to_string(self).context("serialising config to YAML")?;
        print!("{yaml}");
        Ok(())
    }
}

/// Returns `~/.config/osm-to-bedrock/` derived from the `HOME` environment
/// variable, or `None` if `HOME` is not set.
fn dirs_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config").join("osm-to-bedrock"))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// Parse a YAML snippet that sets several fields; verify values and that
    /// unset fields remain `None`.
    #[test]
    fn parse_full_yaml() {
        let yaml = r#"
scale: 2.5
sea_level: 70
building_height: 10
signs: true
no_roads: false
overture_timeout: 30
"#;
        let cfg: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(cfg.scale, Some(2.5));
        assert_eq!(cfg.sea_level, Some(70));
        assert_eq!(cfg.building_height, Some(10));
        assert_eq!(cfg.signs, Some(true));
        assert_eq!(cfg.no_roads, Some(false));
        assert_eq!(cfg.overture_timeout, Some(30));
        // Fields not in the snippet must be None.
        assert!(cfg.address_signs.is_none());
        assert!(cfg.poi_markers.is_none());
        assert!(cfg.elevation.is_none());
        assert!(cfg.overpass_url.is_none());
    }

    /// An empty string deserialises to an all-`None` `Config`.
    #[test]
    fn parse_empty_yaml() {
        let cfg: Config = serde_yaml_ng::from_str("").unwrap();
        assert!(cfg.scale.is_none());
        assert!(cfg.sea_level.is_none());
        assert!(cfg.signs.is_none());
        assert!(cfg.overture.is_none());
    }

    /// Unknown keys in YAML must be silently ignored (forward compatibility).
    #[test]
    fn unknown_keys_ignored() {
        let yaml = r#"
scale: 1.0
future_feature: something
another_unknown: 42
"#;
        let result: Result<Config, _> = serde_yaml_ng::from_str(yaml);
        assert!(result.is_ok(), "unknown keys should not cause an error");
        let cfg = result.unwrap();
        assert_eq!(cfg.scale, Some(1.0));
    }

    /// `merge` fills `None` fields in `self` from `other`, and does not
    /// overwrite fields that are already set in `self`.
    #[test]
    fn merge_fills_none_fields() {
        let mut high = Config {
            scale: Some(3.0),
            sea_level: Some(65),
            ..Default::default()
        };
        let low = Config {
            scale: Some(1.0),         // should NOT overwrite high.scale
            building_height: Some(8), // should fill high.building_height
            signs: Some(true),        // should fill high.signs
            ..Default::default()
        };
        high.merge(&low);

        assert_eq!(high.scale, Some(3.0)); // unchanged
        assert_eq!(high.sea_level, Some(65)); // unchanged
        assert_eq!(high.building_height, Some(8)); // filled from low
        assert_eq!(high.signs, Some(true)); // filled from low
        assert!(high.address_signs.is_none()); // still None (not in either)
    }

    /// `from_file` returns `Ok(None)` for a path that does not exist.
    #[test]
    fn from_file_returns_none_for_missing() {
        let result = Config::from_file(Path::new("/tmp/__nonexistent_osm_config_xyz__.yaml"));
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    /// `from_file` parses a valid YAML file written to a temp file.
    #[test]
    fn from_file_reads_valid_yaml() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "scale: 4.0\nbuilding_height: 12").unwrap();
        let cfg = Config::from_file(tmp.path()).unwrap().unwrap();
        assert_eq!(cfg.scale, Some(4.0));
        assert_eq!(cfg.building_height, Some(12));
    }

    /// `load` with an explicit path that does not exist must return `Err`.
    #[test]
    fn load_explicit_path_not_found_errors() {
        let result = Config::load(Some(Path::new("/tmp/__missing_osm_config__.yaml")));
        assert!(result.is_err());
    }

    /// `dump` serialises to YAML without panicking and the output contains
    /// keys for fields that are `Some`.
    #[test]
    fn dump_produces_yaml() {
        let cfg = Config {
            scale: Some(2.0),
            sea_level: Some(70),
            signs: Some(false),
            ..Default::default()
        };
        // Capture stdout by redirecting to a buffer via serialisation directly.
        let yaml = serde_yaml_ng::to_string(&cfg).unwrap();
        assert!(yaml.contains("scale"), "YAML should contain 'scale'");
        assert!(
            yaml.contains("sea_level"),
            "YAML should contain 'sea_level'"
        );
        // dump() itself must not panic.
        assert!(cfg.dump().is_ok());
    }
}
