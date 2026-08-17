//! No colour-management engine is reachable from anything this library ships.
//!
//! # Where the question comes from
//!
//! hayro #205, #235, #355 and #390, by way of the caller's
//! `doc/HAYRO_ISSUES_FOR_QUORRA.md` §6: four fuzzer files reaching an assertion inside
//! `qcms` and two slice overruns inside `moxcms`. Their sentence is the one that makes it
//! ours to answer rather than to sympathise with:
//!
//! > An `ICCBased` colour space carries an arbitrary attacker-supplied profile (§8.6.5.5),
//! > and it is evaluated on the rendering path. This is the one place in a renderer where
//! > untrusted *data* is fed to a parser that most projects treat as infrastructure.
//!
//! Our answer is structural. Colour is not ours (`doc/PLAN.md` integration note 6:
//! `ColourSpace::to_rgb` upstream is the only place a colour becomes RGB, and adding a
//! second one is forbidden), device RGB is what arrives, and `RENDER_LIBRARY.md` §9 lists
//! colour management first among the non-goals. A profile therefore reaches no parser
//! here because there is no parser here to reach.
//!
//! # Why a test and not only `deny.toml`
//!
//! `deny.toml` names four crates — `qcms`, `moxcms`, `lcms2`, `lcms2-sys` — and a name
//! list is a **blocklist**: the CMS crate published next year is not on it. CLAUDE.md's
//! own reason for that file is that "a non-goal that is only written in prose is a
//! non-goal that arrives as a transitive dependency", and a blocklist is prose with a
//! parser. So this file asserts the same policy from the other side:
//!
//! 1. the **direct** dependencies of the three crates we publish are an allowlist, so a
//!    new one has to be argued for here before it can be added at all;
//! 2. nothing in the transitive graph those direct dependencies reach carries a name a
//!    colour-management crate could have — a pattern, which the unpublished crate matches
//!    too;
//! 3. `deny.toml` still names every crate this file knows by name, so the CI policy and
//!    this gate cannot drift apart;
//! 4. no source file in the workspace names a colour-management API or the four-byte
//!    profile signature an ICC parser must contain.
//!
//! The graph walked is the **shipping** one. `winit`, taken for the window smoke test,
//! pulls `ab_glyph` and `tiny-skia` through `sctk-adwaita`'s Wayland decorations; both are
//! in `Cargo.lock` and neither is reachable from a published crate. That distinction is
//! the reason this file walks the lock rather than scanning it, and `dev_only` records
//! what the distinction is currently load-bearing for.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The crates this workspace publishes, in the order a caller meets them.
const SHIPPING: [&str; 3] = ["quorra", "quorra-gpu", "quorra-scene"];

/// Every direct dependency a published crate may have, with why it is there.
///
/// An allowlist rather than a count, because the failure this exists to catch is a
/// *specific* new name and the message should be able to say which. `quorra-scene`'s
/// absence from this list is the point of ADR 0001 and is asserted separately.
const DIRECT: [(&str, &str); 5] = [
    (
        "quorra-scene",
        "the scene model, which is this workspace's own",
    ),
    (
        "quorra-gpu",
        "the device half, which is this workspace's own",
    ),
    (
        "thiserror",
        "typed errors; a derive, and no runtime behaviour (CLAUDE.md's stack table)",
    ),
    ("wgpu", "the GPU abstraction (ADR 0002)"),
    (
        "pollster",
        "block_on for wgpu's two awaits; a thread is not a runtime",
    ),
];

/// The colour-management crates that exist today and that `deny.toml` must keep naming.
///
/// Named as well as pattern-matched because a name can carry a reason and a pattern
/// cannot: these four are the ones a PDF renderer actually reaches for, and two of them
/// are the ones hayro's fuzzers landed in.
const KNOWN_ENGINES: [&str; 4] = ["qcms", "moxcms", "lcms2", "lcms2-sys"];

/// What a crate name must not contain if it is to be in the shipping graph, and which
/// §9 non-goal each pattern stands for.
///
/// Patterns rather than names, because §9's non-goals outlive any particular crate. Each
/// is checked against the lowercased name with `-` and `_` folded together, so
/// `owned_ttf_parser` and `ttf-parser` are one case.
const FORBIDDEN_SUBSTRINGS: [(&str, &str); 12] = [
    (
        "cms",
        "colour management (§9): qcms, moxcms and lcms2 all match",
    ),
    ("icc", "colour management (§9): an ICC profile parser"),
    ("colormanagement", "colour management (§9)"),
    ("colourmanagement", "colour management (§9)"),
    (
        "font",
        "font loading (§9): outlines reach us already positioned",
    ),
    ("ttf", "font loading (§9)"),
    ("glyph", "font loading (§9): a glyph is an OutlineId here"),
    ("buzz", "shaping (§9): rustybuzz, harfbuzz"),
    ("swash", "font loading and shaping (§9)"),
    (
        "skia",
        "a second 2D scene model: tiny-skia is also the caller's oracle",
    ),
    ("vello", "a second 2D scene model; this library replaces it"),
    ("svg", "a second 2D scene model: resvg, usvg"),
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/<crate> is two levels below the workspace root")
        .to_path_buf()
}

/// A crate name reduced to what a pattern is matched against: lowercase, with the two
/// separators cargo treats as interchangeable folded away.
fn folded(name: &str) -> String {
    name.to_ascii_lowercase().replace(['-', '_'], "")
}

/// The dependency names in one `[section]` of a manifest.
///
/// A manifest key is `name = …` or `name.workspace = true`; a section ends at the next
/// `[header]`. That is the whole of the syntax these four manifests use, and a manifest
/// that grew a shape this cannot read would show up as a *missing* dependency, which the
/// allowlist below fails on rather than passes.
fn manifest_section(text: &str, section: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == format!("[{section}]");
            continue;
        }
        if !inside || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let key = line
            .split(['=', '.'])
            .next()
            .expect("split always yields one field")
            .trim();
        if !key.is_empty() {
            names.push(key.to_owned());
        }
    }
    names
}

/// Every package in `Cargo.lock`, mapped to the packages it depends on.
///
/// The lock's `dependencies` list carries `"name"`, `"name version"` or
/// `"name version source"`; only the name is policy-relevant, so only the name is read.
fn lock_graph(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut package = String::new();
    let mut in_dependencies = false;
    for line in text.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            package.clear();
            in_dependencies = false;
        } else if let Some(rest) = line.strip_prefix("name = ") {
            rest.trim_matches('"').clone_into(&mut package);
            graph.entry(package.clone()).or_default();
            in_dependencies = false;
        } else if line == "dependencies = [" {
            in_dependencies = true;
        } else if in_dependencies {
            if line == "]" {
                in_dependencies = false;
            } else {
                let entry = line.trim_end_matches(',').trim_matches('"');
                let name = entry
                    .split_whitespace()
                    .next()
                    .expect("a dependency entry is not empty");
                graph
                    .entry(package.clone())
                    .or_default()
                    .push(name.to_owned());
            }
        }
    }
    graph
}

/// Every package reachable from `roots` in the lock.
fn reachable(graph: &BTreeMap<String, Vec<String>>, roots: &[String]) -> BTreeSet<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut stack: Vec<String> = roots.to_vec();
    while let Some(name) = stack.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        for next in graph.get(&name).into_iter().flatten() {
            stack.push(next.clone());
        }
    }
    seen
}

/// The direct dependencies of the three published crates, from their manifests.
///
/// Read from the manifests rather than from the lock because the lock does not separate
/// `[dependencies]` from `[dev-dependencies]` for a workspace member — which is exactly
/// the distinction this file exists to make.
fn shipping_direct(root: &Path) -> BTreeSet<String> {
    let mut direct = BTreeSet::new();
    for name in SHIPPING {
        let manifest = std::fs::read_to_string(root.join("crates").join(name).join("Cargo.toml"))
            .unwrap_or_else(|e| panic!("{name}'s manifest: {e}"));
        direct.extend(manifest_section(&manifest, "dependencies"));
    }
    direct
}

/// Where the lock walk starts: the direct dependencies that are **not** members of this
/// workspace.
///
/// A member is excluded because its entry in `Cargo.lock` lists its dev-dependencies too
/// — cargo does not separate them there — so following `quorra-gpu` through the lock would
/// drag `winit` and `png` in and make the shipping graph indistinguishable from the flat
/// one. Every member's own `[dependencies]` is read from its manifest instead, which is
/// what [`shipping_direct`] already did.
fn lock_roots(root: &Path) -> Vec<String> {
    shipping_direct(root)
        .into_iter()
        .filter(|name| !SHIPPING.contains(&name.as_str()))
        .collect()
}

/// **The allowlist**: a published crate depends on these four names and no others.
///
/// The gate that closes `deny.toml`'s structural hole. A blocklist is silent about the
/// crate nobody has heard of; this fails on any addition at all, which puts the decision
/// where CLAUDE.md principle 4 wants it — in a place that has to state a reason.
#[test]
fn a_published_crate_depends_on_four_names_and_each_has_a_reason() {
    let direct = shipping_direct(&workspace_root());
    let allowed: BTreeSet<String> = DIRECT.iter().map(|(name, _)| (*name).to_owned()).collect();
    let added: Vec<_> = direct.difference(&allowed).cloned().collect();
    let gone: Vec<_> = allowed.difference(&direct).cloned().collect();
    assert!(
        added.is_empty(),
        "a crate this workspace publishes has acquired a direct dependency that DIRECT \
         does not name: {added:?}. Every dependency of ours is a dependency of a PDF \
         viewer's process (deny.toml's opening paragraph), and §9's non-goals are the \
         list to check it against before it is added."
    );
    assert!(
        gone.is_empty(),
        "DIRECT names a dependency no published crate has any more; a stale entry is one \
         nobody re-read: {gone:?}"
    );
}

/// ADR 0001, as a fact about the graph: **building a scene requires no device.**
///
/// `quorra-scene` has an empty `[dependencies]`, and `wgpu` in particular is not in it.
/// The brief's §2.3 rests on this, and the manifest's own comment says the ADR has to be
/// rewritten before it changes — so the ADR gets a gate rather than a comment.
#[test]
fn the_scene_crate_has_no_dependencies_at_all() {
    let manifest = std::fs::read_to_string(
        workspace_root()
            .join("crates")
            .join("quorra-scene")
            .join("Cargo.toml"),
    )
    .expect("quorra-scene's manifest");
    assert_eq!(
        manifest_section(&manifest, "dependencies"),
        Vec::<String>::new(),
        "quorra-scene has acquired a dependency; §2.3's \"building a scene requires no \
         device\" is held by this being empty (ADR 0001)"
    );
}

/// **No colour-management engine, no font crate and no second 2D renderer is reachable
/// from anything we publish** — the shipping graph, walked, against §9's non-goals.
#[test]
fn the_shipping_graph_reaches_no_non_goal() {
    let root = workspace_root();
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).expect("the workspace lock");
    let graph = lock_graph(&lock);
    assert!(
        graph.len() > 100,
        "the lock parse found only {} packages, so it is not reading the lock",
        graph.len()
    );

    let shipping = reachable(&graph, &lock_roots(&root));
    assert!(
        shipping.contains("wgpu") && shipping.contains("naga"),
        "the walk must reach wgpu and its shader compiler, or it is walking nothing: \
         {} packages",
        shipping.len()
    );

    for name in &shipping {
        let folded = folded(name);
        for (pattern, non_goal) in FORBIDDEN_SUBSTRINGS {
            assert!(
                !folded.contains(pattern),
                "`{name}` is reachable from a published crate and its name matches \
                 `{pattern}` — {non_goal}. If this is a false positive, the pattern is \
                 what to argue with; if it is not, §9 says this job belongs to the \
                 caller and doing it twice is how two implementations of one decision \
                 get into a process."
            );
        }
        for engine in KNOWN_ENGINES {
            assert_ne!(
                name.as_str(),
                engine,
                "`{engine}` is reachable from a published crate. An ICCBased profile is \
                 attacker-supplied data (§8.6.5.5) and this is the parser hayro's \
                 fuzzers landed in four times; colour is settled upstream (integration \
                 note 6)."
            );
        }
    }
}

/// The two crates that make the walk above worth doing, pinned as **dev-only**.
///
/// `ab_glyph` and `tiny-skia` are in `Cargo.lock` today, reached through
/// `winit → sctk-adwaita`'s Wayland decorations. A gate that scanned the flat lock would
/// have to either fail or exempt them by name; this one records the shape of the fact
/// instead — they are in the lock, and they are not in the shipping graph — so that a day
/// on which either becomes reachable is a day this test says so.
///
/// It is also the control for the walk: it fails if `reachable` ever starts returning
/// everything.
#[test]
fn the_font_and_raster_crates_in_the_lock_are_dev_only() {
    let root = workspace_root();
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).expect("the workspace lock");
    let graph = lock_graph(&lock);
    let shipping = reachable(&graph, &lock_roots(&root));
    for name in ["ab_glyph", "tiny-skia", "owned_ttf_parser"] {
        assert!(
            graph.contains_key(name),
            "`{name}` has left the lock, so this test's premise is stale: either the \
             window smoke test's dependency changed or the entry should go"
        );
        assert!(
            !shipping.contains(name),
            "`{name}` is now reachable from a published crate. It arrives through \
             `winit → sctk-adwaita` for the window smoke test; a route from the library \
             itself is a §9 non-goal linked into the caller's process."
        );
    }
}

/// `deny.toml` still names every engine [`KNOWN_ENGINES`] does.
///
/// Two expressions of one policy drift apart unless something compares them. `cargo deny`
/// runs in CI and this test runs everywhere; each catches what the other cannot — a
/// licence or an advisory there, a name deleted from the ban list here.
#[test]
fn deny_toml_still_bans_every_engine_this_test_knows_by_name() {
    let policy =
        std::fs::read_to_string(workspace_root().join("deny.toml")).expect("deny.toml exists");
    for engine in KNOWN_ENGINES {
        assert!(
            policy.contains(&format!("crate = \"{engine}\"")),
            "deny.toml no longer bans `{engine}`. The ban and this test are the two \
             halves of one policy: CI's cargo-deny sees a dependency arriving, this test \
             sees the ban being removed."
        );
    }
}

/// Every `.rs` and `.wgsl` file under `dir`, recursively.
fn source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            source_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs" || e == "wgsl") {
            out.push(path);
        }
    }
}

/// Nothing we ship *implements* a colour-management engine either.
///
/// A dependency gate answers "did we link one"; this answers "did we write one". The
/// tokens are the ones a profile parser cannot avoid: the four-byte `acsp` signature
/// ICC.1 puts at offset 36 of every profile, and the entry points of the three engines
/// `deny.toml` names.
///
/// Only `src/` is scanned, and the search is case-sensitive on purpose — the prose in
/// `paint.rs` and `function.rs` says *why* there is no CMS here, and a gate that fired on
/// its own documentation would be deleted rather than fixed.
#[test]
fn no_source_file_parses_a_colour_profile() {
    let root = workspace_root();
    let crates = root.join("crates");
    let mut sources = Vec::new();
    for entry in std::fs::read_dir(&crates).expect("crates/ is readable") {
        let src = entry.expect("a directory entry").path().join("src");
        if src.is_dir() {
            source_files(&src, &mut sources);
        }
    }
    assert!(
        sources.len() > 40,
        "the walk found only {} source files, so it is not walking the workspace",
        sources.len()
    );
    for path in &sources {
        let text = std::fs::read_to_string(path).expect("a source file");
        for token in ["acsp", "IccProfile", "cmsOpenProfile", "qcms_", "moxcms"] {
            assert!(
                !text.contains(token),
                "{} contains `{token}`, which only a colour-profile parser has a use \
                 for. Colour becomes RGB once, upstream, in `ColourSpace::to_rgb` \
                 (integration note 6); a second place is forbidden whether it is linked \
                 or written.",
                path.display()
            );
        }
    }
}
