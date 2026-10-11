//! Choices from the settings window that outlive a launch, in `<data_dir>/settings.json`.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The settings file's name inside the data directory.
const FILE: &str = "settings.json";

/// The saved settings.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prefs {
    /// The key of the chosen summarizer; `None` means the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summarizer: Option<String>,
}

impl Prefs {
    /// The settings saved in `data_dir`; the defaults when there are none or they can't be read.
    pub fn load(data_dir: &Path) -> Self {
        std::fs::read_to_string(path(data_dir))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Save these settings in `data_dir`.
    ///
    /// # Errors
    ///
    /// Returns the [`io::Error`] when the file can't be written.
    pub fn save(&self, data_dir: &Path) -> io::Result<()> {
        std::fs::write(path(data_dir), serde_json::to_string_pretty(self)?)
    }
}

fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_settings_load_back_and_a_bad_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Prefs::load(dir.path()), Prefs::default());

        let prefs = Prefs {
            summarizer: Some("off".into()),
        };
        prefs.save(dir.path()).unwrap();
        assert_eq!(Prefs::load(dir.path()), prefs);

        std::fs::write(dir.path().join(FILE), "not json").unwrap();
        assert_eq!(Prefs::load(dir.path()), Prefs::default());
    }
}
