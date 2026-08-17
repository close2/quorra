//! `f32::mul_add` is a libcall on this build's target, so it stays off the hot paths.
//!
//! The question comes from the caller's reading of hayro #630. A JPEG 2000 inverse colour
//! transform there calls `f32::mul_add` with no runtime feature detection, so on a target
//! built without FMA it falls through to libc's software emulation, which is very slow and
//! does not dispatch. `mul_add` is the natural way to write a colour transform or a
//! gradient step, it is one instruction where the target has it, and it is a function call
//! where the target does not.
//!
//! **This build's target does not have it.** `rust-toolchain.toml` pins the compiler and
//! nothing in this repository sets `target-cpu` or `target-feature` — there is no tracked
//! `.cargo/config.toml`, and `Cargo.toml`'s release profile sets only `lto` and
//! `codegen-units` — so the baseline `x86-64` applies and FMA is not in it. That is not a
//! guess: the 2026-08-14 profiling round measured the same shape one function over, and
//! `doc/history/2026-08-02--2026-08-14-m1-through-adr-0048.md` records it — `floorf`,
//! `ceilf` and `roundf` are software on this target, **1.29 M instructions a frame**, and
//! raising `target-cpu` was deliberately not taken because it decides which processors this
//! library runs on.
//!
//! # What this test is, and what it is not
//!
//! It is not a ban. Two `src/` sites use `mul_add` today and both are correct uses: each
//! runs at most once per frame, where one libcall is nothing and the fused rounding is a
//! small accuracy gain. The allow-list below names them with the reason each is cold, and
//! the test fails when the set changes in either direction — a new site, or one of these
//! two moved or deleted without the list being updated.
//!
//! The failure message is the whole point of the gate: whoever adds the next `mul_add` is
//! writing a gradient step or a colour transform, which is exactly hayro #630's function,
//! and they should read this before deciding rather than after profiling.
//!
//! **WGSL's `fma` is a different question and is not gated here.** `reduce.wgsl` uses it,
//! and on the device it is an instruction the adapter has; nothing about a host target
//! bears on it.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Every `src/` site that may call `mul_add`, as `crate/path/from/src`, with why it is
/// cold. A site not on this list fails the test; a site on it that has gone fails too,
/// because a stale exemption is an exemption nobody re-read.
const COLD_SITES: [(&str, &str); 2] = [
    (
        "quorra-gpu/src/winding.rs",
        "sample_offsets: at most 256 offsets, built once when the winding lane's uniform \
         buffers are made, not per fragment and not per segment",
    ),
    (
        "quorra-gpu/src/mask.rs",
        "transparent_value: §11.5.3's luminosity of one backdrop colour, once per mask \
         plan per frame",
    ),
];

/// The workspace root, from this test crate's manifest directory.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/<crate> is two levels below the workspace root")
        .to_path_buf()
}

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `mul_add` stays off every path a frame walks more than once.
///
/// Only `src/` is scanned. Tests, examples and benches build fixtures and reference
/// arithmetic, neither of which ships or runs per pixel, and several of them use `mul_add`
/// deliberately — `tests/common/scene.rs` lays out 5 933 rectangles with it.
#[test]
fn mul_add_appears_only_where_it_is_called_once_a_frame() {
    let root = workspace_root();
    let crates = root.join("crates");
    let mut found: BTreeSet<String> = BTreeSet::new();
    let mut sources = Vec::new();
    let entries = std::fs::read_dir(&crates).expect("crates/ is readable");
    for entry in entries {
        let src = entry.expect("a directory entry").path().join("src");
        if src.is_dir() {
            rust_files(&src, &mut sources);
        }
    }
    assert!(
        sources.len() > 40,
        "the walk found only {} source files, so it is not walking the workspace",
        sources.len()
    );
    for path in &sources {
        let text = std::fs::read_to_string(path).expect("a source file");
        if text.contains("mul_add") {
            let relative = path
                .strip_prefix(&crates)
                .expect("under crates/")
                .to_string_lossy()
                .replace('\\', "/");
            found.insert(relative);
        }
    }

    let allowed: BTreeSet<String> = COLD_SITES.iter().map(|(p, _)| (*p).to_owned()).collect();
    let unexpected: Vec<_> = found.difference(&allowed).cloned().collect();
    let vanished: Vec<_> = allowed.difference(&found).cloned().collect();

    assert!(
        unexpected.is_empty(),
        "`mul_add` is a libcall on this build's target — no `target-cpu` is set anywhere in \
         this repository, so the baseline x86-64 applies and it has no FMA. On a path a \
         frame walks once that costs nothing; on a per-pixel or per-segment path it is \
         hayro #630, where a colour transform written this way fell through to libc's \
         software emulation. New site(s): {unexpected:?}. If the new one is cold too, add \
         it to COLD_SITES with the reason; if it is hot, it needs a measurement before it \
         needs a review."
    );
    assert!(
        vanished.is_empty(),
        "COLD_SITES names a file that no longer calls `mul_add`; a stale exemption is one \
         nobody re-read: {vanished:?}"
    );
}
