//! The WGSL this crate compiles, named once.
//!
//! A shader source reached the pipeline store through an `include_str!` at the line
//! that compiled it, which was fine while nothing else needed one. The uniform-layout
//! gate does: it reads the same source the adapter reads and derives what offset each
//! field of a `Params` sits at (`layout`, and the `#[cfg(test)]` modules beside every
//! writer of a uniform). Two `include_str!` lists of the same files would be two lists
//! that can drift, and a gate reading a file the device no longer compiles is a gate
//! that proves nothing — so there is one list, and it is this.
//!
//! What each shader *does* is stated in its own header. `function_ops.wgsl` has no
//! constant here because it is not a pipeline's shader: it is the operator library a
//! generated program is built from, and `crate::function::OPERATORS` publishes it for
//! the conformance corpus. It is still WGSL this crate holds, so [`ALL`] names it under
//! the constant that owns it — a gate over the shader *text* has no reason to skip it.
//!
//! [`ALL`] is that one list made enumerable, and the test below holds it to the
//! directory: a `.wgsl` file this crate does not name fails the build rather than going
//! unread by every gate. Both gates that read shader text — `layout` and `copies` —
//! are `#[cfg(test)]` modules *here* rather than integration tests, because an
//! integration test cannot reach a private module and a second `include_str!` list is
//! exactly the drift this file exists to prevent (ADR 0059).

#[cfg(test)]
pub(crate) mod copies;
#[cfg(test)]
pub(crate) mod layout;

/// The analytic rectangle lane (§0 of the brief's second fast path).
pub(crate) const RECT: &str = include_str!("shaders/rect.wgsl");
/// The coverage-quad lane, which draws a glyph tile or a rasterised shape.
pub(crate) const COVERAGE: &str = include_str!("shaders/coverage.wgsl");
/// The image quad (ISO 32000-2 §8.9.5).
pub(crate) const IMAGE: &str = include_str!("shaders/image.wgsl");
/// The shading quad — a sampled ramp or a rasterised mesh (§8.7.4.5).
pub(crate) const SHADING: &str = include_str!("shaders/shading.wgsl");
/// Group composition: §11.3.5's blend modes and §11.4's transparency model.
pub(crate) const COMPOSITE: &str = include_str!("shaders/composite.wgsl");
/// A soft mask's reduction to one channel (§11.5).
pub(crate) const REDUCE: &str = include_str!("shaders/reduce.wgsl");
/// Pixels moved and not changed (ADR 0038, ADR 0039).
pub(crate) const BLIT: &str = include_str!("shaders/blit.wgsl");
/// One finished layer put on the surface under an affine and a filter (ADR 0056).
pub(crate) const PRESENT: &str = include_str!("shaders/present.wgsl");
/// The GPU coverage lane's winding accumulation and resolve (ADR 0016).
pub(crate) const WINDING: &str = include_str!("shaders/winding.wgsl");
/// The fixed half of a generated function paint's pipeline (ADR 0053); the program's
/// own body is appended to it.
pub(crate) const FUNCTION_LANE: &str = include_str!("shaders/function_lane.wgsl");

/// Every WGSL source this crate holds, by the file it was included from.
///
/// The pairing is what lets a gate walk the sources; the file name is what lets it name
/// the offender. Nothing may be left out: `every_wgsl_file_is_named_here` compares this
/// list against the directory itself, so a shader added to the tree and not to this list
/// fails the build instead of being skipped by every text gate in silence.
#[cfg(test)]
pub(crate) const ALL: &[(&str, &str)] = &[
    ("rect.wgsl", RECT),
    ("coverage.wgsl", COVERAGE),
    ("image.wgsl", IMAGE),
    ("shading.wgsl", SHADING),
    ("composite.wgsl", COMPOSITE),
    ("reduce.wgsl", REDUCE),
    ("blit.wgsl", BLIT),
    ("present.wgsl", PRESENT),
    ("winding.wgsl", WINDING),
    ("function_lane.wgsl", FUNCTION_LANE),
    ("function_ops.wgsl", crate::function::OPERATORS),
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    /// [`super::ALL`] names every `.wgsl` file in the shader directory, and no others.
    ///
    /// The directory is the source of truth here, deliberately: a list checked only
    /// against itself is the shape the copies gate had while it was an integration test
    /// with its own `include_str!`s, and what that cost was six copies of a helper of
    /// which five were compared (ADR 0059). Reading the tree at test time is what makes
    /// "a shader nothing checks" impossible rather than unlikely.
    #[test]
    fn every_wgsl_file_is_named_here() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shaders");
        let on_disk: BTreeSet<String> = fs::read_dir(&dir)
            .expect("the shader directory is beside this file")
            .map(|entry| entry.expect("a readable directory entry").path())
            // Case-insensitively, so that a `.WGSL` is caught by the gate rather than
            // slipping past it on a case-preserving filesystem.
            .filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("wgsl"))
            })
            .filter_map(|path| path.file_name()?.to_str().map(ToOwned::to_owned))
            .collect();
        let named: BTreeSet<String> = super::ALL
            .iter()
            .map(|(file, _)| (*file).to_owned())
            .collect();
        assert_eq!(
            on_disk,
            named,
            "the shader directory and `shaders::ALL` disagree: {:?} exist unnamed, {:?} \
             are named and absent",
            on_disk.difference(&named).collect::<Vec<_>>(),
            named.difference(&on_disk).collect::<Vec<_>>(),
        );
    }

    /// And each file is named once, so a copy counted twice cannot agree with itself.
    #[test]
    fn no_source_is_named_twice() {
        let mut seen = BTreeSet::new();
        for (file, _) in super::ALL {
            assert!(
                seen.insert(*file),
                "`{file}` is named twice in `shaders::ALL`"
            );
        }
    }
}
