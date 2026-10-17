// Copyright (C) 2026 Sarat Chandra
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Locates libmpv for linking. Set `MPV_LIB_DIR` to override detection, or
//! `MPV_STATIC_DIR` to link the static build from `scripts/build-mpv.sh`.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    println!("cargo:rerun-if-env-changed=MPV_LIB_DIR");
    println!("cargo:rerun-if-env-changed=MPV_STATIC_DIR");
    if let Ok(dir) = std::env::var("MPV_STATIC_DIR").map(|d| d.trim().to_string())
        && !dir.is_empty()
    {
        link_static(&Path::new(env!("CARGO_MANIFEST_DIR")).join(dir));
        return;
    }
    if let Ok(dir) = std::env::var("MPV_LIB_DIR").map(|d| d.trim().to_string())
        && !dir.is_empty()
    {
        println!("cargo:rustc-link-search=native={dir}");
        return;
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(out) = Command::new("pkg-config")
        .args(["--variable=libdir", "mpv"])
        .output()
    {
        let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !dir.is_empty() {
            candidates.push(PathBuf::from(dir));
        }
    }
    // Sibling `lib` of the mpv binary (Nix, Homebrew, MSYS layouts).
    if let Ok(out) = Command::new("which").arg("mpv").output() {
        let bin = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if let Ok(real) = std::fs::canonicalize(&bin)
            && let Some(prefix) = real.parent().and_then(|p| p.parent())
        {
            candidates.push(prefix.join("lib"));
        }
    }
    candidates.extend(
        [
            "/opt/homebrew/lib",
            "/usr/local/lib",
            "/usr/lib",
            "/usr/lib/x86_64-linux-gnu",
        ]
        .iter()
        .map(PathBuf::from),
    );
    for dir in candidates {
        let has_lib = [
            "libmpv.dylib",
            "libmpv.so",
            "libmpv.2.dylib",
            "libmpv.so.2",
            "mpv.lib",
            "libmpv.dll.a",
        ]
        .iter()
        .any(|name| dir.join(name).exists());
        if has_lib {
            println!("cargo:rustc-link-search=native={}", dir.display());
            return;
        }
    }
    println!("cargo:warning=libmpv not found; set MPV_LIB_DIR to the directory containing libmpv");
}

/// Links libmpv and its dependencies statically, as listed by its pkg-config
/// file. Our `-lmpv` comes before the rest, so static link order holds even
/// though libmpv2-sys adds its own `-lmpv` later.
fn link_static(prefix: &Path) {
    let pc = prefix.join("lib/pkgconfig");
    println!("cargo:rerun-if-changed={}", pc.join("mpv.pc").display());
    let out = Command::new("pkg-config")
        .env("PKG_CONFIG_PATH", &pc)
        .args(["--static", "--libs", "mpv"])
        .output()
        .expect("pkg-config is required for MPV_STATIC_DIR");
    assert!(
        out.status.success(),
        "no static libmpv in {}; run scripts/build-mpv.sh first: {}",
        prefix.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    let flags = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut flags = flags.split_whitespace();
    while let Some(flag) = flags.next() {
        if let Some(dir) = flag.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={dir}");
        } else if let Some(lib) = flag.strip_prefix("-l") {
            println!("cargo:rustc-link-lib={lib}");
        } else if flag == "-framework"
            && let Some(name) = flags.next()
        {
            println!("cargo:rustc-link-lib=framework={name}");
        } else {
            println!("cargo:rustc-link-arg={flag}");
        }
    }
    // libplacebo has C++ parts its pkg-config file doesn't list.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-lib=c++");
    } else {
        println!("cargo:rustc-link-lib=static=stdc++");
    }
}
