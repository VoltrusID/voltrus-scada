//! Embeds the shared symbol library (`../symbols/*.json`, the files the web
//! client draws too) into the console, so the binary needs no files beside it:
//! `symbol_files.rs` lists every symbol file by key with its text, and the
//! state styles (`_styles.json`). Files starting with `_` or `.` and
//! `*.schema.json` are not symbols, as for the web's `import.meta.glob` and
//! the server's `build.rs`.
//!
//! On Windows it also embeds the executable's icon and version information:
//! the icon (`packaging/windows/voltrus.ico`, made by
//! `scripts/windows-icon.swift`) as resource 1, which Explorer shows for the
//! file and GPUI loads for the window and the taskbar, and a VERSIONINFO with
//! this crate's version.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::{env, fs};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=packaging/windows/voltrus.ico");
    symbol_files();
    #[cfg(windows)]
    windows::embed_resources();
}

fn symbol_files() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let dir = manifest.join("..").join("symbols");
    println!("cargo:rerun-if-changed={}", dir.display());
    let entries = fs::read_dir(&dir).unwrap_or_else(|e| {
        panic!(
            "\n\nThe symbol library {} cannot be read ({e}).\n\
             The console draws its ISA-101 symbols from it; build from a tree that has symbols/.\n",
            dir.display()
        )
    });
    let mut files: Vec<(String, PathBuf)> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?.to_owned();
            let key = name.strip_suffix(".json")?;
            (!key.starts_with('_') && !key.starts_with('.') && !key.ends_with(".schema"))
                .then(|| (key.to_owned(), path.clone()))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no symbol files in {}", dir.display());
    let literal = |path: &PathBuf| format!("{:?}", path.display().to_string());
    let mut generated = String::from(
        "/// Every symbol file: its key (the file name) and its text.\npub static FILES: &[(&str, &str)] = &[\n",
    );
    for (key, path) in &files {
        println!("cargo:rerun-if-changed={}", path.display());
        writeln!(generated, "    ({key:?}, include_str!({})),", literal(path)).unwrap();
    }
    generated.push_str("];\n");
    let styles = dir.join("_styles.json");
    println!("cargo:rerun-if-changed={}", styles.display());
    writeln!(
        generated,
        "/// The state styles (`_styles.json`).\npub static STYLES: &str = include_str!({});",
        literal(&styles)
    )
    .unwrap();
    let target = out.join("symbol_files.rs");
    if fs::read_to_string(&target).ok().as_deref() != Some(generated.as_str()) {
        fs::write(&target, generated).expect("write symbol_files.rs");
    }
}

#[cfg(windows)]
mod windows {
    use std::path::PathBuf;
    use std::{env, fs};

    pub fn embed_resources() {
        let manifest =
            PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
        let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
        let icon = manifest
            .join("packaging")
            .join("windows")
            .join("voltrus.ico");
        let version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
        let number = |key: &str| -> u16 {
            env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(|| panic!("{key} is not a number"))
        };
        let (major, minor, patch) = (
            number("CARGO_PKG_VERSION_MAJOR"),
            number("CARGO_PKG_VERSION_MINOR"),
            number("CARGO_PKG_VERSION_PATCH"),
        );
        // rc.exe reads C-style string literals: backslashes are escapes.
        let icon = icon.display().to_string().replace('\\', "\\\\");
        let rc = format!(
            r#"1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3F
FILEFLAGS 0x0
FILEOS 0x40004
FILETYPE 0x1
FILESUBTYPE 0x0
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "CompanyName", "JoyoDigitama"
            VALUE "FileDescription", "Voltrus SCADA Console"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "voltrus-native"
            VALUE "LegalCopyright", "Copyright (c) 2026 JoyoDigitama. AGPL-3.0-or-later."
            VALUE "OriginalFilename", "voltrus-native.exe"
            VALUE "ProductName", "Voltrus SCADA"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
        );
        let rc_path = out.join("voltrus-native.rc");
        fs::write(&rc_path, rc).expect("write voltrus-native.rc");
        embed_resource::compile(&rc_path, embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }
}
