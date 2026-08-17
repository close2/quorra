//! Every example is run by CI, and this is what says so (ADR 0060).
//!
//! # The defect this exists for
//!
//! `cargo test` neither builds nor runs an example. Several examples here carry
//! `assert!`s that are their own signature gates, and `examples/retained.rs` **panicked
//! at one of them on `main` for two days** after ADR 0057 moved the row it asserted —
//! with no signal at all, because nothing ran it. An assertion nothing executes is not
//! a gate; it is a comment that can rot into a panic.
//!
//! The answer has two halves and this file is the second. The first is that each example
//! accepts `--check`: the smallest configuration that executes every assertion it makes.
//! The second is a CI step that runs `--check` for every example — and a step that names
//! its examples is a **list**, and a list drifts from the directory beside it. ADR 0059
//! settled the same shape for the shader table: *the directory is the source of truth
//! for what exists, the list is the source of truth for what is run, and a test is what
//! stops those from being different questions.*
//!
//! So: an example added to `examples/` and not to the workflow fails the build here,
//! naming itself.
//!
//! # Why a test and not a build script
//!
//! ADR 0059's reasoning applies unchanged: this workspace has no build script, one would
//! run on every consumer's machine including the caller's, and it buys nothing over a
//! unit test CI already runs. What it costs is that a *local* `cargo test` reads a file
//! under `.github/`, which is stated in `expected_workflow` rather than discovered.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The workflow file this crate's examples are run from.
fn workflow() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.github/workflows/ci.yml")
        .canonicalize()
        .expect("the CI workflow is part of the deliverable, not an optional file")
}

/// The example targets cargo would build: `examples/<name>.rs`, and `examples/<name>/`
/// for a multi-file example, which cargo recognises by its `main.rs`.
fn examples_on_disk() -> BTreeSet<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut names = BTreeSet::new();
    for entry in std::fs::read_dir(&root).expect("this crate has an examples directory") {
        let path = entry.expect("a readable directory entry").path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if path.is_dir() {
            if path.join("main.rs").is_file() {
                names.insert(stem.to_owned());
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            names.insert(stem.to_owned());
        }
    }
    assert!(
        !names.is_empty(),
        "no examples found under {}",
        root.display()
    );
    names
}

/// The names the workflow's `--check` step loops over.
///
/// Read from the `for example in \` … `do` block rather than from a YAML parser: this
/// workspace has no YAML dependency and `deny.toml` is the reason it will not acquire
/// one for a test. The parse is deliberately literal — it finds the loop by its exact
/// opening line, so a rewrite of the step that this test cannot read **fails** rather
/// than quietly matching nothing.
fn examples_in_workflow(text: &str) -> BTreeSet<String> {
    let opening = "for example in \\";
    let Some(head) = text.split_once(opening).map(|(_, rest)| rest) else {
        panic!(
            "the workflow no longer opens its example loop with `{opening}`. If the step \
             was rewritten, rewrite this parse with it — a gate that cannot read its own \
             list is worse than no gate"
        )
    };
    let Some((list, _)) = head.split_once("\n          do") else {
        panic!("the example loop's `do` must follow its list, at the step's indentation")
    };
    list.split_whitespace()
        .filter(|token| *token != "\\")
        .map(str::to_owned)
        .collect()
}

/// Every example on disk is named in the workflow step that runs `--check`.
///
/// The failure names both directions, because they are different mistakes: an example
/// nothing runs is an assertion that has stopped being a gate, and a name in the
/// workflow with no example behind it is a step that fails for a reason nobody intended.
#[test]
fn every_example_is_run_by_ci() {
    let path = workflow();
    let text = std::fs::read_to_string(&path).expect("the workflow is readable");
    let on_disk = examples_on_disk();
    let in_workflow = examples_in_workflow(&text);

    let unrun: Vec<&String> = on_disk.difference(&in_workflow).collect();
    let phantom: Vec<&String> = in_workflow.difference(&on_disk).collect();
    assert!(
        unrun.is_empty(),
        "{unrun:?} exist under examples/ and are not run by {}. Nothing else runs an \
         example — `cargo test` does not build one — so every assertion in them is a \
         comment (ADR 0060). Add each to the `--check` step.",
        path.display(),
    );
    assert!(
        phantom.is_empty(),
        "{phantom:?} are named by {} and are not examples of this crate",
        path.display(),
    );
}

/// Every example named there actually **accepts** `--check`.
///
/// A name in the list whose example does not read the flag is worse than an omission:
/// several of these take a positional adapter substring first, so `--check` would be
/// taken for an adapter name and the run would fail — or, worse, succeed while
/// measuring the full sweep and take CI's time for nothing.
///
/// The check is textual, which is what a test outside the binary can do. It cannot say
/// the flag is *honoured*; what says that is the step running the example and the
/// example finishing, which is the other half of the arrangement.
#[test]
fn every_example_reads_the_check_flag() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut silent = Vec::new();
    for name in examples_on_disk() {
        let single = root.join(format!("{name}.rs"));
        let source = if single.is_file() {
            single
        } else {
            root.join(&name).join("main.rs")
        };
        let text = std::fs::read_to_string(&source).expect("an example's source is readable");
        if !text.contains("\"--check\"") {
            silent.push(name);
        }
    }
    assert!(
        silent.is_empty(),
        "{silent:?} do not read `--check`. Every example takes it — as the smallest run \
         that executes its assertions, or as an accepted no-op where the example already \
         is that run — because CI invokes them all the same way (ADR 0060)",
    );
}
