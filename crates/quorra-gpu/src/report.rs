//! What we could not draw as asked.
//!
//! # This is not a logging convenience
//!
//! §2.5 and §5 of the brief, and the sentence that makes it a specification item
//! rather than a nicety:
//!
//! > anything you cannot draw as asked is a `Report`, not a silent approximation. A
//! > gradient drawn opaque, a blend mode substituted, a mask ignored — each of those is
//! > a plausible-looking wrong page, which is the worst outcome this project has a name
//! > for. We would rather have a hole and a sentence.
//!
//! A report is the difference between a viewer that tells a person what is missing and
//! one that lies to them. The caller counts them per document — 74 of 974 corpus
//! documents currently report something — so a report is data, which is why the kind is
//! **enumerated and not a string**.
//!
//! # The rule about the variants
//!
//! A [`ReportKind`] is added in the same commit as the code that emits it, and there is
//! no `Other`: an `Other { detail }` variant would be a string report wearing an enum's
//! clothes, making "how many documents hit this?" unanswerable again. A report means
//! *the frame was drawn and something about it is not what was asked for*; anything
//! that means the frame was not drawn is a [`RenderError`].
//!
//! [`RenderError`]: crate::error::RenderError

/// One thing the device could not do as asked, attached to the frame that was drawn
/// anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// What happened, enumerated so a caller can count and group.
    pub kind: ReportKind,
    /// For a person, and never the only information.
    pub detail: String,
}

/// The enumerated kinds of report. Each variant is added in the same commit as the
/// code that emits it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReportKind {
    /// A [`Viewport::damage`] list was provided, but per-tile damage tracking does not
    /// exist yet (M8), so the device redrew the entire target instead. The frame is
    /// correct and complete; what was not honoured is the *economy* the caller asked
    /// for.
    ///
    /// [`Viewport::damage`]: crate::viewport::Viewport::damage
    DamageNotHonoured,
    /// The frame painted with a §7.10.5 program that reads a value the operand stack has
    /// not got, and this device supplies 0 there.
    ///
    /// **ISO 32000-2 defines nothing here.** PostScript raises `stackunderflow`;
    /// §7.10.5 says nothing at all; the caller's own evaluator serves the value from
    /// `unwrap_or(0.0)` and one of their two witnesses depends on it three times. So the
    /// value is a *decision*, ADR 0053 requires that adopting it is never silent, and this
    /// is that report.
    ///
    /// # What the report says, and what it deliberately does not
    ///
    /// It says **the program contains N such reads**, counted statically over every branch
    /// — not that the frame's pixels relied on one. A pop inside an `ifelse` arm is
    /// counted whether or not that arm ran, and making the count dynamic would need a
    /// per-fragment counter, which is a per-fragment cost for a diagnostic.
    ///
    /// The wording follows the count rather than the other way round, because CLAUDE.md's
    /// second instrumentation rule is exactly this mistake in its other direction: a
    /// number that describes the lookups you made, reported as though it described the
    /// ones you should have made. A caller reading this learns that the program's picture
    /// *may* depend on an undefined value and where to look; it must not read it as proof
    /// that a pixel did.
    FunctionEmptyStackRead,
}
