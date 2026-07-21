//! Stamps the git revision of the source tree into the compiled crate.
//!
//! WHY this exists at all: Groloo-Web loads the core from a *pinned vendored
//! folder* (`public/assets/heart/<version>/`). Nothing today proves the bytes in
//! that folder are the bytes that folder's name claims — a stale or mispointed
//! copy is completely invisible, and it presents as "the app behaves like an old
//! build" rather than as an error. Baking the revision into the artifact lets the
//! shell ask the core what it actually is, immediately after instantiate, and
//! shout when the answer disagrees with the path it loaded from.
//!
//! Failure is not an error here. A source tarball, a vendored build, or a fresh
//! `git init` with no commit yet must all still compile; they just report
//! `unknown` and the shell's comparison degrades to "cannot verify" instead of
//! "verified wrong".

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Let CI (or a reproducible-build harness) supply the revision directly when
    // git is absent from the image or the checkout is a detached export.
    println!("cargo:rerun-if-env-changed=GROLOO_CORE_GIT_SHA");

    // Re-stamp when HEAD moves. Both paths are needed: `.git/HEAD` changes on a
    // branch switch, the ref file changes on a commit. Neither existing is fine.
    for p in ["../../.git/HEAD", "../../.git/refs/heads"] {
        if std::path::Path::new(p).exists() {
            println!("cargo:rerun-if-changed={p}");
        }
    }

    let sha = std::env::var("GROLOO_CORE_GIT_SHA")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(git_short_sha)
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=GROLOO_CORE_GIT_SHA={sha}");
}

/// `git rev-parse --short=7 HEAD`, or `None` for any reason whatsoever — no git
/// on PATH, not a repository, or a repository with no commits yet.
fn git_short_sha() -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}
