//! What we could not draw as asked.
//!
//! **Skeleton — M1 fills this** (`doc/adr/0003`).
//!
//! # This is not a logging convenience
//!
//! §2.5 and §5, and the sentence that makes it a specification item rather than a nicety:
//!
//! > anything you cannot draw as asked is a `Report`, not a silent approximation. A
//! > gradient drawn opaque, a blend mode substituted, a mask ignored — each of those is a
//! > plausible-looking wrong page, which is the worst outcome this project has a name for.
//! > We would rather have a hole and a sentence.
//!
//! A report is therefore the difference between a viewer that tells a person what is
//! missing and one that lies to them. The caller counts them per document — 74 of 974 corpus
//! documents currently report something — so a report is data, which is why the kind is
//! **enumerated and not a string**.
//!
//! # Planned signatures
//!
//! ```text
//! pub struct Report {
//!     pub kind: ReportKind,   // enumerated, so a caller can count and group
//!     pub detail: String,     // for a person, and never the only information
//! }
//!
//! pub enum ReportKind { /* … */ }
//! ```
//!
//! # The rule about the variants, which matters more than the variants
//!
//! **A `ReportKind` is added in the same commit as the code that emits it, and there is no
//! `Other`.** Enumerating kinds in advance would be guessing at our own failure modes, and
//! an `Other { detail }` variant is a string report wearing an enum's clothes — it makes
//! "how many documents hit this?" unanswerable again, which is the one question the
//! enumeration exists to answer.
//!
//! Two consequences for the design, both settled now rather than at M6:
//!
//! - **A report is not an error, and an error is not a report.** A report means *the frame
//!   was drawn and something in it is not what the scene asked for*. Anything that means the
//!   frame was not drawn is a `RenderError`. §4.7's very large coordinates and degenerate
//!   transforms are errors: refuse them loudly, never produce NaN geometry.
//! - **A substitution is never silent, and mostly never happens.** All sixteen blend modes,
//!   both fill rules and both mask rules are requirements; there is no "unsupported blend
//!   mode" report to write, because there is no unsupported blend mode. The reports that
//!   will exist are about limits — a budget reached, damage we could not honour, a pipeline
//!   cache we rejected — and each of those is a fact, with a number in it.
