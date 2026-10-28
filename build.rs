//! Embeds the built web client (`static/`) into the executable.
//!
//! Release builds (`--release`) embed it and fail when `static/index.html` is
//! missing: build the client first (`cd client && npm ci && npm run build`),
//! then `cargo build --release`. Debug builds embed nothing and the server
//! reads `static/` from disk, so `cargo build`, `cargo test` and `cargo clippy`
//! work without a client build. `VOLTRUS_SKIP_UI_EMBED=1` makes a release build
//! without the UI (a server for the API only, simulators, CI jobs that never
//! open the UI); `VOLTRUS_EMBED_UI=1` embeds it in a debug build, to try the
//! single-file behaviour without a release build.
//!
//! Each file is gzip-compressed here, so the executable carries about a third of
//! the size and the server can send the bytes as they are to browsers that accept
//! gzip. The generated `ui_assets.rs` (included by `src/api/ui.rs`) lists the
//! files sorted by path, `index.html` and `assets/...` as the client build lays
//! them out.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

use flate2::Compression;
use flate2::write::GzEncoder;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=VOLTRUS_SKIP_UI_EMBED");
    println!("cargo:rerun-if-env-changed=VOLTRUS_EMBED_UI");

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let ui_dir = manifest.join("static");

    let release = env::var("PROFILE").is_ok_and(|p| p == "release");
    let skip = env::var("VOLTRUS_SKIP_UI_EMBED").is_ok_and(|v| !v.is_empty() && v != "0");
    let force = env::var("VOLTRUS_EMBED_UI").is_ok_and(|v| !v.is_empty() && v != "0");
    let embed = (release || force) && !skip;

    let mut generated = String::from("pub static ASSETS: &[Asset] = &[\n");
    if embed {
        println!("cargo:rerun-if-changed=static");
        if !ui_dir.join("index.html").is_file() {
            panic!(
                "\n\nThe web client is not built: {} does not exist.\n\
                 A release build embeds the web UI into the executable, so build the client first:\n\
                 \n    cd client && npm ci && npm run build && cd ..\n    cargo build --release\n\n\
                 To build without the web UI (API only), set VOLTRUS_SKIP_UI_EMBED=1.\n",
                ui_dir.join("index.html").display()
            );
        }
        let blobs = out.join("ui");
        let _ = fs::remove_dir_all(&blobs);
        fs::create_dir_all(&blobs).expect("create OUT_DIR/ui");

        let mut files = Vec::new();
        collect(&ui_dir, &ui_dir, &mut files);
        files.sort();
        for (i, rel) in files.iter().enumerate() {
            let raw = fs::read(ui_dir.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"));
            let mut enc = GzEncoder::new(Vec::new(), Compression::best());
            enc.write_all(&raw).expect("gzip");
            let gz = enc.finish().expect("gzip");
            // Keep the original when gzip saves less than 5 % (images, fonts).
            let (data, gzipped) = if gz.len() * 20 < raw.len() * 19 {
                (gz, true)
            } else {
                (raw.clone(), false)
            };
            let blob = blobs.join(format!("{i}.bin"));
            fs::write(&blob, &data).expect("write blob");
            writeln!(
                generated,
                "    Asset {{ path: {rel:?}, data: include_bytes!({:?}), gzipped: {gzipped}, len: {}, hash: {:#018x} }},",
                blob.display().to_string(),
                raw.len(),
                fnv1a(&raw),
            )
            .unwrap();
        }
    }
    generated.push_str("];\n");

    // Rewrite only on change, so a debug build does not recompile when nothing did.
    let target = out.join("ui_assets.rs");
    if fs::read_to_string(&target).ok().as_deref() != Some(generated.as_str()) {
        fs::write(&target, generated).expect("write ui_assets.rs");
    }
}

/// Every file under `dir` as a `/`-separated path relative to `root`, without
/// source maps (never served) and hidden files.
fn collect(root: &Path, dir: &Path, files: &mut Vec<String>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') || name.ends_with(".map") {
            continue;
        }
        if path.is_dir() {
            collect(root, &path, files);
        } else {
            let rel = path.strip_prefix(root).expect("under root");
            let parts: Vec<_> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect();
            files.push(parts.join("/"));
        }
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
