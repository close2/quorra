//! Every way this crate refuses, in two typed enums and the five vocabularies they carry.
//!
//! §5 of the brief: a failure is an `Err` that names what overflowed or did not hold,
//! so the caller can act on it — its window falls back to the CPU backend, and a
//! person reading a log can attribute the refusal without reproducing it. There is no
//! catch-all variant in any of the seven, for the same reason `ReportKind` has no
//! `Other`: a variant that names nothing makes "how often does this happen?"
//! unanswerable.
//!
//! The two enums follow the two phases of a device's life: [`DeviceError`] is
//! construction (§2.1) and residency (§2.2), [`RenderError`] is a frame (§2.4) — and a
//! present, which is a frame's last step happening somewhere else (ADR 0056). A report
//! — the frame *was* drawn, something in it is not as asked — is deliberately neither;
//! that is [`crate::report`].
//!
//! The other five are the vocabularies those two carry, each raised in exactly one
//! subsystem and each an enum for the reason [`crate::report::ReportKind`] is one.
//!
//! # The parts
//!
//! Seven items, seven modules, and this list is the only place the seam survives:
//! rustdoc inlines a re-export from a private module, so a reader of the published
//! documentation sees all seven types here and *cannot see the modules that own them*
//! (`doc/adr/0051` cost 1). The modules are private and re-exported, so
//! `quorra_gpu::error::RenderError` resolves exactly as it always has and
//! `quorra_gpu::error::render::RenderError` is not a path anybody can write — the split
//! is a decision about who reviews what, not about what a caller may name.
//!
//! | module | its one thing |
//! |---|---|
//! | `device` | what a device refuses when no frame is in flight |
//! | `render` | why one frame — or one present — was refused |
//! | `resource` | what an upload's content violated |
//! | `function` | why a program the device would have evaluated is not admitted |
//! | `pipeline` | what this adapter would not build |
//! | `layer` | what a layer handed to a presenter did not satisfy |
//! | `surface` | what the swapchain answered instead of handing over a texture |
//!
//! Which enum carries which vocabulary, since the table above does not say and the
//! rendered page will not either: [`DeviceError`] carries [`ResourceProblem`] and
//! [`FunctionProblem`]; [`RenderError`] carries [`LayerProblem`], [`SurfaceProblem`] and
//! [`PipelineProblem`]. [`PipelineProblem`] is also carried by
//! [`WarmUp::Refused`](crate::startup::WarmUp::Refused), which is not an error at all —
//! it is the answer to "is this device warm yet?" when a shader will never compile.
//!
//! # A § with no standard in front of it
//!
//! In this module a bare `§n` is `doc/RENDER_LIBRARY.md`, the brief the library exists to
//! satisfy, and every clause of the specification is written in full — `ISO 32000-2
//! §7.10.5`. The one exception is that clause's own shorthand: once a module comment has
//! written it out, "a §7.10.5 program" is what the rest of the tree calls a type 4
//! function.

mod device;
mod function;
mod layer;
mod pipeline;
mod render;
mod resource;
mod surface;

pub use device::DeviceError;
pub use function::FunctionProblem;
pub use layer::LayerProblem;
pub use pipeline::PipelineProblem;
pub use render::RenderError;
pub use resource::ResourceProblem;
pub use surface::SurfaceProblem;
