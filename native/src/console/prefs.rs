//! What this workstation remembers beside the server's per-user
//! preferences, in `<config dir>/voltrus-native/preferences.toml`: the
//! theme chosen for each brightness (the server keeps only light, dark or
//! system), the history aggregation default, the kiosk lock and OS
//! notifications. The web client keeps the same things in the browser's
//! local storage.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::Console;
use super::shell::Shell;

/// How the server condenses history to the chart's resolution
/// (`/api/v1/history?agg=`): an id, a label and what it does.
pub const AGGREGATIONS: [(&str, &str, &str); 3] = [
    (
        "sampling",
        "Sampling",
        "Evenly spaced points. Fastest, but skips the values between them.",
    ),
    (
        "avg",
        "Average",
        "The mean of each interval. A smooth line that hides peaks.",
    ),
    (
        "minmax",
        "Min/Max/Avg",
        "The mean with its minimum and maximum. Nothing is lost.",
    ),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// The bundled theme shown when the appearance is dark.
    pub dark_theme: String,
    /// The bundled theme shown when the appearance is light.
    pub light_theme: String,
    /// One of [`AGGREGATIONS`].
    pub history_agg: String,
    /// Quit, sign-out and window close are blocked.
    pub kiosk: bool,
    /// A system notification for each new critical alarm.
    pub os_notifications: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            dark_theme: vt_ui::theme::DEFAULT_DARK.to_owned(),
            light_theme: vt_ui::theme::DEFAULT_LIGHT.to_owned(),
            history_agg: "minmax".to_owned(),
            kiosk: false,
            os_notifications: false,
        }
    }
}

impl Prefs {
    /// The theme name for this brightness.
    pub fn theme_for(&self, dark: bool) -> &str {
        if dark {
            &self.dark_theme
        } else {
            &self.light_theme
        }
    }

    /// The aggregation id, one of [`AGGREGATIONS`] even if the file says
    /// something else.
    pub fn agg(&self) -> &'static str {
        AGGREGATIONS
            .iter()
            .map(|(id, _, _)| *id)
            .find(|id| *id == self.history_agg)
            .unwrap_or("minmax")
    }
}

fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("voltrus-native").join("preferences.toml"))
}

/// The saved preferences, or the defaults if the file is missing or
/// unreadable.
pub fn load() -> Prefs {
    path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

/// Save the preferences; a failure only costs remembering them.
pub fn save(prefs: &Prefs) {
    let Some(path) = path() else {
        return;
    };
    let result = toml::to_string_pretty(prefs)
        .map_err(std::io::Error::other)
        .and_then(|text| {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let partial = path.with_extension("toml.partial");
            std::fs::write(&partial, text)?;
            std::fs::rename(&partial, &path)
        });
    if let Err(error) = result {
        eprintln!(
            "voltrus-native: could not save the preferences in {}: {error}",
            path.display()
        );
    }
}

impl Console {
    /// The history aggregation default (`sampling`, `avg` or `minmax`) for
    /// `/api/v1/history?agg=`.
    pub fn history_agg(&mut self) -> &'static str {
        self.state::<Shell>().prefs.agg()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_partial_file_falls_back_to_defaults() {
        let partial: Prefs = toml::from_str("kiosk = true\nhistory_agg = \"avg\"\n").unwrap();
        assert!(partial.kiosk);
        assert_eq!(partial.agg(), "avg");
        assert_eq!(partial.theme_for(true), vt_ui::theme::DEFAULT_DARK);
        assert_eq!(partial.theme_for(false), vt_ui::theme::DEFAULT_LIGHT);
        let odd: Prefs = toml::from_str("history_agg = \"median\"\n").unwrap();
        assert_eq!(
            odd.agg(),
            "minmax",
            "an unknown aggregation reads as the default"
        );
        let round: Prefs = toml::from_str(&toml::to_string_pretty(&partial).unwrap()).unwrap();
        assert_eq!(round, partial);
    }
}
