# ADR 0003 — What a skeleton may contain

Status: accepted, 2026-08-02. Expires when M1 lands: after that, this convention
applies only to modules M1 has not reached.

## Context

The project was set up before any rendering was to be implemented, which puts two rules
in direct tension.

CLAUDE.md principle 1: *no placeholder implementations, no `todo!()` left in merged
code, no "we'll fix it later" paths.* Principle 6: *a frame is drawn, or it is refused;
there is no third state.* A skeleton written the usual way violates both — a `Device`
whose `render` returns `Ok(Frame::default())` is a function that draws nothing and
reports success, which is precisely the failure this library exists to eliminate, shipped
as scaffolding.

The other tempting shape is an opaque type per planned struct: `pub struct Device {
inner: () }`. That compiles, warns about a field nobody reads, and states something
untrue about the design — that the shape of the type is decided — while carrying none of
the reasoning that will decide it.

## Decision

A module that M-something will fill contains **the contract, not a stand-in for the
code**: a module-level doc comment stating what the module owns, which section of
`RENDER_LIBRARY.md` and which clause of ISO 32000-2 it answers to, and the planned
signatures in a fenced `text` block so that rustdoc shows them without the compiler
being asked to believe them.

Real code appears in the skeleton in exactly one circumstance: **where the type is the
contract and could not be written differently.** Two things qualify today.

- `quorra_scene::blend::BlendMode` — sixteen variants, because ISO 32000-2 §11.3.5 names
  sixteen. `Compose`, because §4.1 names two compositing behaviours and the whole
  argument of that section is that the second one exists.
- `quorra_scene::ids` — a resource handle is a `u32` and a name.

Everything else waits. In particular there is no `Device`, no `Scene` and no
`SceneBuilder` type in the tree yet: their fields are the design work of M1 and M2, and
an empty version of each would be a claim that the work is done.

## Consequences

`cargo build`, `cargo clippy --all-targets` and `cargo test` are clean from the first
commit, with no `#[allow]`, no `todo!()` and no dead code — so the gates are meaningful
on day one rather than being switched on later once the noise is cleared.

A reader who opens `crates/quorra-gpu/src/device.rs` learns what a device must do and
what it may not do, and cannot mistake the file for an implementation in progress.

The cost is that the planned signatures in those doc blocks are not compiler-checked, so
they can drift from the plan they describe. That is acceptable for exactly as long as
they are the only thing in the file: the milestone that writes the code deletes the block
in the same commit, and a `text` block that outlives its module's implementation is a
review defect.
