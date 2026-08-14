# 0051 — A split public module keeps its parts private

Date: 2026-08-15. Status: accepted, and built.

`quorra_scene::scene` was 1 216 lines holding four responsibilities. Splitting it is
CLAUDE.md principle 1's file-scale rule and needs no ADR. **Where the parts go does**,
because `scene` is a *public* module and the split is otherwise a breaking change to
every path a caller has written.

## The decision

The parts are **private submodules, re-exported from the parent**:

```rust
mod builder;
mod command;
mod cost;
mod frames;
mod validate;

pub use builder::SceneBuilder;
pub use command::{ClipDef, Command, GroupSpec, ImageFilter, MAX_GROUP_DEPTH, MaskDef};
pub use cost::Cost;
pub use validate::MAX_COORDINATE;
```

`quorra_scene::scene::Command` still resolves, `quorra_scene::Command` still resolves,
and **no new public path exists**: `quorra_scene::scene::command::Command` is not a
thing a caller can write, so it is not a thing we have to keep.

## Why not `pub mod command`

It would work and it would read well from inside. It is refused for one reason with a
measurement behind it in the caller's tree rather than ours: **a public path is a
promise, and this one would be a promise about our file layout.** The next time a
module outgrows itself — and `encode.rs` at 2 342 lines and `device.rs` at 2 183 are
both waiting — the split would either have to preserve the previous layout's paths or
break a caller who imported one. The viewer pins us by git revision and re-baselines a
974-page corpus on every bump; a path break costs them a round for nothing gained.

The same argument in one sentence: **splitting a file is a decision about who reviews
what, and it should not also be a decision about what a caller may write.**

## What it costs, stated rather than discovered

1. **rustdoc inlines a re-export from a private module.** A reader of the published
   documentation sees `Command` under `scene`, exactly as before, and *cannot see the
   module that owns it*. The responsibility seam this split exists to make legible is
   visible in the source tree and in `scene`'s own module comment, and nowhere else.
   The module comment therefore names all five parts and what each one's one thing is —
   that list is not decoration, it is the only place the structure survives into the
   docs.
2. **A doc link into a sibling gets a path.** `[`SceneError::NonIsolatedGroupUnsupported`]`
   resolved by an import when everything was one file. Where the moved text now sits
   without that import, the link is written
   `[`text`](crate::error::SceneError::NonIsolatedGroupUnsupported)` — same rendered
   text, and no import that exists only for rustdoc's benefit (rustc would warn it
   unused).
3. **Fields widen to `pub(super)`.** `frames` owns the open-frame stack and `validate`
   answers questions about the clip and mask counts, so `SceneBuilder`'s four fields are
   visible across the `scene` subtree instead of inside one file. That is a real loss of
   enforcement — nothing but review stops a future part of `scene` from writing
   `self.commands` directly — and it is the price of putting the state machine and the
   refusals where they can be read. It is bounded: `pub(super)` here means the `scene`
   subtree and nothing else, and the struct is still opaque to the crate, the workspace
   and the caller.

## Where this applies

Every public module of `quorra-scene` and `quorra-gpu`. It does **not** bind the private
ones: `compose` and `winding`, split in the same round, are `mod` in `lib.rs`, so their
submodules are internal either way and the visibility question there is only
`pub(crate)` versus `pub(super)` — answered by "as private as it was", not by this ADR.
