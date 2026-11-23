//! The server address and username of the last sign-in, remembered in
//! `<config dir>/voltrus-native/config.toml`. The password never is.
//!
//! The file may also name a `ca_file`: PEM certificates trusted for the
//! server on top of the operating system's store (see
//! `voltrus_scada_native::tls`), for a plant CA not installed on the
//! computer. Written by hand; a sign-in keeps it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Saved {
    pub url: Option<String>,
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_file: Option<PathBuf>,
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

/// Trust the `ca_file` the config names, if any, before the first
/// request. A file that cannot be read is reported; the system's store
/// still serves.
pub fn trust_ca_file() {
    let Some(path) = load().ca_file else {
        return;
    };
    match voltrus_scada_native::tls::add_ca_file(&path) {
        Ok(count) => eprintln!(
            "voltrus-native: trusting {count} certificate(s) from {}",
            path.display()
        ),
        Err(error) => eprintln!("voltrus-native: ca_file not used: {error}"),
    }
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
        // Set by hand; the sign-in keeps it.
        ca_file: load().ca_file,
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
            ca_file: None,
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
        let with_ca: Saved = toml::from_str("ca_file = \"/etc/plant/ca.pem\"\n").unwrap();
        assert_eq!(with_ca.ca_file, Some(PathBuf::from("/etc/plant/ca.pem")));
        assert!(
            toml::to_string_pretty(&with_ca)
                .unwrap()
                .contains("ca_file = ")
        );
    }
}
