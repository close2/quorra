//! The glyph atlas.
//!
//! **Skeleton — M4 fills this** (`doc/adr/0003`).
//!
//! # The number that decides the design
//!
//! One dense page of ISO 32000-2 is **5 933 fills of 107 distinct outlines**, and today
//! every one of the 5 933 is flattened again on every frame. The caller priced a coverage
//! cache for its *CPU* backend and refused it, and the measurement transfers (its ADR 0131):
//!
//! | keying | reuse on that page | reuse on `tracemonkey.pdf` | oracle |
//! |---|---|---|---|
//! | exact position | 116 hits of 5 933 | **not once** | clean |
//! | quantised to 1/8 pixel | — | — | **contradicts pages** |
//! | quantised to 1/16 pixel | **5.0×** | 1.3× | clean |
//!
//! A glyph's sub-pixel phase is an arbitrary float, so an exactly-correct cache never hits.
//! At 1/16 of a pixel it hits five times over on a dense page and the oracle's verdicts do
//! not move. The caller refused it because `tiny-skia` provides no blitter for a cached
//! coverage bitmap; **on a GPU the blitter is a textured quad, which is the natural
//! primitive** — so the same number that stopped them is the reason we build it.
//!
//! # The contract
//!
//! - Key on `(outline, scale bucket, sub-pixel phase)` with the **quantum settable and
//!   documented** — §4.5's fifth decision, the one the caller wants to make and needs us to
//!   expose. Default 1/16 if we like, but settable, and switchable off. **Quantising glyph
//!   positions silently would change where the text sits**, and the oracle would contradict
//!   pages with nobody able to say why.
//! - An **R8 coverage atlas with eviction, sized from a budget the caller sets**, not from a
//!   constant of ours.
//! - **Report the count of distinct keys, not the hit rate** (`crate::frame::Counters`).
//!
//! # Two questions M4 answers with measurements
//!
//! - **§11.3: what does the atlas cost on a page it cannot help?** `tracemonkey.pdf` reuses
//!   1.3×. A cache that is 5× on one page and a net loss on another is a decision, not a
//!   feature — and the decision needs both numbers.
//! - **Where the coverage is rasterised.** If the atlas is filled on the CPU by `tiny-skia`,
//!   the glyphs come from *the same code that is the caller's correctness oracle*, which is
//!   a correctness argument no other arrangement gets for free. Against it: a dependency, a
//!   transfer per new glyph, and a CPU cost on the frame that first needs one. §6.3 offers
//!   this as persuasive rather than prescriptive, so it is an ADR with a measurement, not a
//!   preference.
