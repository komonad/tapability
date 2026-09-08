//! User-facing generation settings.
//!
//! They can come from a small `key = value` file (`--config`), from command
//! line flags, or from the settings panel inside the game. The game remembers
//! whatever you change in the panel and reloads it on the next start.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Board is `size x size`.
    pub size: usize,
    /// `None` means "pick a seed from the clock".
    pub seed: Option<u64>,
    /// Fraction of cells that become black, as a range.
    pub density_lo: f64,
    pub density_hi: f64,
    /// Reject shapes whose biggest clue-free patch is larger than this fraction
    /// of the board.
    pub max_clueless_fraction: f64,
    /// Nodes for a whole generation, shared adaptively between proofs.
    pub node_budget: u64,
    /// Milliseconds for a whole generation.
    pub time_budget_ms: u64,
    /// Floor for a single uniqueness proof.
    pub check_min_nodes: u64,
    /// How much a single proof may borrow over its fair share of the pool.
    pub check_slack: u64,
    /// How many shapes to try before giving up.
    pub max_attempts: usize,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            size: 20,
            seed: None,
            density_lo: 0.40,
            density_hi: 0.50,
            max_clueless_fraction: 0.15,
            node_budget: 150_000,
            time_budget_ms: 2_000,
            check_min_nodes: 3_000,
            check_slack: 3,
            max_attempts: 200,
        }
    }
}

#[derive(Debug)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

fn number(value: &str, key: &str) -> Result<f64, ConfigError> {
    value
        .trim()
        .parse::<f64>()
        .map_err(|_| ConfigError(format!("{key}: '{value}' is not a number")))
}

/// The settings shown in the in-game panel, in display order.
pub const FIELDS: &[(&str, &str)] = &[
    ("size", "Board size (3-60)"),
    ("density", "Density LO-HI"),
    ("max_clueless_fraction", "Max clueless fraction"),
    ("node_budget", "Node budget"),
    ("time_budget_ms", "Time budget (ms)"),
    ("check_min_nodes", "Min nodes per proof"),
    ("check_slack", "Budget slack"),
    ("max_attempts", "Max attempts"),
    ("seed", "Seed (blank = random)"),
];

impl Settings {
    /// Set one setting by name. Used by the config file, the CLI and the panel.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), ConfigError> {
        let value = value.trim();
        match key {
            "size" => {
                let n = number(value, key)? as usize;
                if !(3..=60).contains(&n) {
                    return Err(ConfigError(format!("size: {n} is outside 3..60")));
                }
                self.size = n;
            }
            "seed" => {
                if value.is_empty() {
                    self.seed = None;
                } else {
                    self.seed = Some(
                        value
                            .parse::<u64>()
                            .map_err(|_| ConfigError(format!("seed: '{value}' is not a number")))?,
                    );
                }
            }
            "density" => {
                let (lo, hi) = value
                    .split_once('-')
                    .ok_or_else(|| ConfigError(format!("density: expected LO-HI, got '{value}'")))?;
                let lo = number(lo, "density")?;
                let hi = number(hi, "density")?;
                if !(0.05..=0.75).contains(&lo) || !(lo..=0.75).contains(&hi) {
                    return Err(ConfigError(format!(
                        "density: {lo}-{hi} must satisfy 0.05 <= LO <= HI <= 0.75"
                    )));
                }
                self.density_lo = lo;
                self.density_hi = hi;
            }
            "max_clueless_fraction" => {
                let v = number(value, key)?;
                if !(0.0..=1.0).contains(&v) {
                    return Err(ConfigError(format!("{key}: {v} must be 0..1")));
                }
                self.max_clueless_fraction = v;
            }
            "node_budget" => self.node_budget = number(value, key)?.max(1.0) as u64,
            "time_budget_ms" => self.time_budget_ms = number(value, key)? as u64,
            "check_min_nodes" => self.check_min_nodes = number(value, key)?.max(1.0) as u64,
            "check_slack" => self.check_slack = (number(value, key)? as u64).max(1),
            "max_attempts" => {
                let n = number(value, key)? as usize;
                if n == 0 {
                    return Err(ConfigError("max_attempts: must be at least 1".into()));
                }
                self.max_attempts = n;
            }
            other => return Err(ConfigError(format!("unknown setting: '{other}'"))),
        }
        Ok(())
    }

    /// Current value of a field, as it appears in the panel.
    pub fn value_of(&self, key: &str) -> String {
        match key {
            "size" => self.size.to_string(),
            "density" => format!("{:.2}-{:.2}", self.density_lo, self.density_hi),
            "max_clueless_fraction" => format!("{:.2}", self.max_clueless_fraction),
            "node_budget" => self.node_budget.to_string(),
            "time_budget_ms" => self.time_budget_ms.to_string(),
            "check_min_nodes" => self.check_min_nodes.to_string(),
            "check_slack" => self.check_slack.to_string(),
            "max_attempts" => self.max_attempts.to_string(),
            "seed" => self.seed.map(|s| s.to_string()).unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// Serialise in the same `key = value` form the parser accepts.
    pub fn to_text(&self) -> String {
        let mut out = String::from("# written by the tapa settings panel\n");
        out.push_str(&format!("size = {}\n", self.size));
        if let Some(seed) = self.seed {
            out.push_str(&format!("seed = {seed}\n"));
        }
        out.push_str(&format!(
            "density = {:.2}-{:.2}\n",
            self.density_lo, self.density_hi
        ));
        out.push_str(&format!(
            "max_clueless_fraction = {:.2}\n",
            self.max_clueless_fraction
        ));
        out.push_str(&format!("node_budget = {}\n", self.node_budget));
        out.push_str(&format!("time_budget_ms = {}\n", self.time_budget_ms));
        out.push_str(&format!("check_min_nodes = {}\n", self.check_min_nodes));
        out.push_str(&format!("check_slack = {}\n", self.check_slack));
        out.push_str(&format!("max_attempts = {}\n", self.max_attempts));
        out
    }

    /// Load settings from a `key = value` file. `#` starts a comment.
    pub fn load_file(&mut self, path: &Path) -> Result<(), ConfigError> {
        let text = fs::read_to_string(path)
            .map_err(|e| ConfigError(format!("cannot read {}: {e}", path.display())))?;
        // editors on Windows like to add a UTF-8 BOM; it is not part of the key
        let text = text.trim_start_matches('\u{feff}');
        for (line_number, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (key, value) = line.split_once('=').ok_or_else(|| {
                ConfigError(format!(
                    "{}:{}: expected 'key = value', got '{line}'",
                    path.display(),
                    line_number + 1
                ))
            })?;
            self.set(key.trim(), value.trim()).map_err(|e| {
                ConfigError(format!("{}:{}: {}", path.display(), line_number + 1, e.0))
            })?;
        }
        Ok(())
    }

    /// Write the settings, falling back to a file in the working directory when
    /// the preferred location is not writable. Returns the path actually used.
    pub fn save_file(&self, path: &Path) -> Result<PathBuf, ConfigError> {
        match self.try_save(path) {
            Ok(()) => Ok(path.to_path_buf()),
            Err(first) => {
                let fallback = PathBuf::from("tapa-settings.conf");
                if fallback == path {
                    return Err(first);
                }
                self.try_save(&fallback).map(|()| fallback).map_err(|_| first)
            }
        }
    }

    fn try_save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| {
                    ConfigError(format!("cannot create {}: {e}", parent.display()))
                })?;
            }
        }
        fs::write(path, self.to_text())
            .map_err(|e| ConfigError(format!("cannot write {}: {e}", path.display())))
    }

    /// Where the game remembers settings: `%APPDATA%\tapa\settings.conf`, or a
    /// file in the working directory when that is not writable.
    pub fn default_path() -> PathBuf {
        if let Ok(appdata) = std::env::var("APPDATA") {
            if !appdata.is_empty() {
                return PathBuf::from(appdata).join("tapa").join("settings.conf");
            }
        }
        PathBuf::from("tapa-settings.conf")
    }

    /// The writable settings path, probed once at startup.
    pub fn resolve_path() -> PathBuf {
        let preferred = Settings::default_path();
        if let Some(parent) = preferred.parent() {
            if fs::create_dir_all(parent).is_ok() {
                let probe = parent.join(".tapa-write-probe");
                if fs::write(&probe, b"x").is_ok() {
                    let _ = fs::remove_file(&probe);
                    return preferred;
                }
            }
        }
        PathBuf::from("tapa-settings.conf")
    }

    /// Load the remembered settings, if a settings file exists. The writable
    /// path wins, then `%APPDATA%`, then a file in the working directory.
    pub fn load_default() -> Settings {
        let mut settings = Settings::default();
        let candidates = [
            Settings::resolve_path(),
            Settings::default_path(),
            PathBuf::from("tapa-settings.conf"),
        ];
        for path in candidates {
            if path.exists() && settings.load_file(&path).is_ok() {
                break;
            }
        }
        settings
    }

    /// A short human readable summary for the window title.
    pub fn summary(&self) -> String {
        format!(
            "{}x{} density {:.2}-{:.2}",
            self.size, self.size, self.density_lo, self.density_hi
        )
    }
}

/// The settings file written next to the executable when none exists yet.
pub const EXAMPLE: &str = "\
# Tapa generation settings. Every line is optional.
#   size                  board is size x size (3..60)
#   seed                  fixed seed; omit for a clock-based seed
#   density               fraction of black cells, LO-HI
#   max_clueless_fraction reject shapes whose biggest clue-free patch is a
#                         larger fraction of the board than this
#   node_budget           nodes for one whole generation (shared adaptively)
#   time_budget_ms        milliseconds for one whole generation
#   check_min_nodes       floor for a single uniqueness proof
#   check_slack           how much one proof may borrow over its fair share
#   max_attempts          how many shapes to try before giving up
#
# Use it with:       tapa --config tapa.conf
# Override one line: tapa --config tapa.conf --set size=24
# In the game: press F2 to open the settings panel; changes are remembered.

size = 20
density = 0.40-0.50
max_clueless_fraction = 0.15
node_budget = 150000
time_budget_ms = 2000
check_min_nodes = 3000
check_slack = 3
max_attempts = 200
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_values() {
        let mut s = Settings::default();
        s.set("size", "25").unwrap();
        s.set("density", "0.3-0.4").unwrap();
        s.set("seed", "42").unwrap();
        s.set("check_slack", "5").unwrap();
        assert_eq!(s.size, 25);
        assert_eq!(s.density_lo, 0.3);
        assert_eq!(s.density_hi, 0.4);
        assert_eq!(s.seed, Some(42));
        assert_eq!(s.check_slack, 5);
        s.set("seed", "").unwrap();
        assert_eq!(s.seed, None);
    }

    #[test]
    fn rejects_bad_values() {
        let mut s = Settings::default();
        assert!(s.set("size", "2").is_err());
        assert!(s.set("size", "99").is_err());
        assert!(s.set("density", "0.5").is_err());
        assert!(s.set("density", "0.6-0.4").is_err());
        assert!(s.set("nonsense", "1").is_err());
        assert!(s.set("size", "abc").is_err());
    }

    #[test]
    fn round_trips_through_text() {
        let mut s = Settings::default();
        s.set("size", "17").unwrap();
        s.set("density", "0.33-0.44").unwrap();
        s.set("node_budget", "123456").unwrap();
        s.set("seed", "7").unwrap();

        let text = s.to_text();
        let mut back = Settings::default();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (k, v) = line.split_once('=').unwrap();
            back.set(k.trim(), v.trim()).unwrap();
        }
        assert_eq!(s, back);
    }

    #[test]
    fn every_field_has_a_value() {
        let s = Settings::default();
        for (key, _) in FIELDS {
            if *key == "seed" {
                continue; // blank means "pick a random seed"
            }
            assert!(!s.value_of(key).is_empty(), "{key} has no value");
        }
    }

    #[test]
    fn parses_the_example_file() {
        let mut s = Settings::default();
        for line in EXAMPLE.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (key, value) = line.split_once('=').unwrap();
            s.set(key.trim(), value.trim()).unwrap();
        }
        assert_eq!(s.size, 20);
        assert_eq!(s.density_lo, 0.40);
    }
}
