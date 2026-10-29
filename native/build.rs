//! Embeds the Windows executable's icon and version information: the icon
//! (`packaging/windows/voltrus.ico`, made by `scripts/windows-icon.swift`) as
//! resource 1, which Explorer shows for the file and GPUI loads for the window
//! and the taskbar, and a VERSIONINFO with this crate's version. Nothing to do
//! when building on other hosts.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=packaging/windows/voltrus.ico");
    #[cfg(windows)]
    windows::embed_resources();
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
