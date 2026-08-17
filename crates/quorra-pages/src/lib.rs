//! The pages this workspace's gates and instruments draw, defined once.
//!
//! # Why this crate exists
//!
//! A page is a measured object. `doc/corpus-profile.md` says what the caller's corpus
//! is made of; `crates/quorra-gpu/tests/archetypes.rs` gates what each of those shapes
//! costs; and half a dozen instruments in `crates/quorra-gpu/examples/` measure the same
//! shapes with a clock. Until 2026-08-17 each of those held its **own copy** of the
//! generator, and re-cutting a page meant editing it in five files.
//!
//! It failed exactly as that arrangement fails. ADR 0057 changed what a clipped mark
//! costs, `examples/retained.rs` kept asserting the row from before it, and the example
//! **panicked at its own signature gate on `main` for two days** — nothing caught it,
//! because `cargo test` neither builds nor runs an example. ADR 0060 is the decision
//! that followed; this crate is half of it.
//!
//! **The rule, and it is the only one:** *a page drawn by more than one target is
//! defined here.* A page with exactly one reader stays with its reader, where its
//! reasons are — `examples/floor.rs`'s single rectangle and its figure page are not
//! fixtures, they are that instrument's subject.
//!
//! # How a caller uses it
//!
//! This crate has no adapter and no device: it depends on `quorra-scene` and nothing
//! else, so it links from a target that cannot open one, and `quorra-gpu`'s
//! dev-dependency on it does not close a cycle (ADR 0060 §3). The price is five lines
//! at each call site, which are plumbing rather than page content:
//!
//! ```ignore
//! let outlines: Vec<OutlineId> = quorra_pages::outlines(&quorra_pages::ARTWORK)
//!     .iter()
//!     .map(|path| device.upload_outline(path).expect("an archetype outline"))
//!     .collect();
//! let scene = quorra_pages::scene(&quorra_pages::ARTWORK, &outlines, None)
//!     .expect("an archetype builds");
//! ```
//!
//! A page that places images ([`IMAGE_PAGE`]) also needs [`image_spec`] uploaded and its
//! identifier passed as the third argument; every other page passes `None`.
//!
//! # What is *not* here
//!
//! The mapping from a rendered frame's `Counters` to [`Recorded`] is not, and cannot be:
//! `Counters` lives in `quorra-gpu`, which this crate must not depend on. Each consumer
//! writes that mapping itself, over **named fields** so that a wrong one cannot be
//! written silently, and compares it against the row recorded here. What rotted in the
//! defect above was the recorded *number*, and that number now exists once.

#![forbid(unsafe_code)]

mod archetype;
mod build;
mod glyph;
mod page;

pub use archetype::{
    Archetype, Recorded, clip_of, curve_clip, marks_box, outline_of, outline_side, position,
    rect_path,
};
pub use build::{image_spec, outlines, scene};
pub use glyph::{
    GLYPH_PAGE, GLYPH_PAGE_UNIQUE_PHASES, GlyphPage, glyph_outlines, glyph_scene, zoomed,
};
pub use page::{
    ARCHETYPES, ARTWORK, CALLERS_DRAWING, CLIP_MOUNTAIN, DENSE_TEXT, DENSE_TEXT_UNCLIPPED, DRAWING,
    GIANT, IMAGE_PAGE, MEDIAN_PAGE,
};
