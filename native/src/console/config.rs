//! The server address and username of the last sign-in, remembered in
//! `<config dir>/voltrus-native/config.toml`. The password never is.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Saved {
    pub url: Option<String>,
    pub username: Option<String>,
}

fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("voltrus-native").join("config.toml"))
}

/// What the last sign-in left, or nothing if the file is missing or unreadable.
pub fn load() -> Saved {
    path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

/// Remember a successful sign-in. Failures only cost the convenience, so
/// they are reported and otherwise ignored.
pub fn save(url: &str, username: &str) {
    let Some(path) = path() else {
        return;
    };
    let saved = Saved {
        url: Some(url.to_owned()),
        username: Some(username.to_owned()),
    };
    let result = toml::to_string_pretty(&saved)
        .map_err(std::io::Error::other)
        .and_then(|text| {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            // Write beside it and rename, so a crash never leaves half a file.
            let partial = path.with_extension("toml.partial");
            std::fs::write(&partial, text)?;
            std::fs::rename(&partial, &path)
        });
    if let Err(error) = result {
        eprintln!(
            "voltrus-native: could not remember the sign-in in {}: {error}",
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_holds_url_and_username_only() {
        let text = toml::to_string_pretty(&Saved {
            url: Some("http://plant:3000".into()),
            username: Some("operator".into()),
        })
        .unwrap();
        assert_eq!(
            text,
            "url = \"http://plant:3000\"\nusername = \"operator\"\n"
        );
        let back: Saved = toml::from_str("url = \"http://x\"\n").unwrap();
        assert_eq!(
            (back.url.as_deref(), back.username),
            (Some("http://x"), None)
        );
    }
}
