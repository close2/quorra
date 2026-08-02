# ADR 0001 — Three crates, and a scene that cannot see a device

Status: accepted, 2026-08-02.

## Context

`RENDER_LIBRARY.md` §2.3 states the property it calls the single most important one in
the document:

> The single most important property in this document: a `Scene` must contain no
> reference to a viewport, a resolution, a device transform, or a target size.

and adds a corollary it asks to have written into our documentation:

> **`Scene: Send + Sync`, and building one requires no device**. Our interpreter runs on
> a worker thread and would build scenes there.

Both are easy to state and easy to lose. A scene builder that holds a `&Device` for one
convenience — an atlas lookup, a limits check, a cached ramp upload — has silently made
scene building device-bound, and nothing about the code looks wrong afterwards. The same
mistake in the caller's tree is trap 2 of its handover document: *a decision either
backend can make alone is a decision neither has made*.

Three crates were considered: one crate with modules, two crates (scene and gpu), and
three (scene, gpu, and a facade).

## Decision

Three crates.

- **`quorra-scene`** — what is to be drawn. Ids, geometry, paint, blend and compose
  modes, masks, images, meshes, the `Scene` and its builder. **It does not depend on
  `wgpu`, and it must not.**
- **`quorra-gpu`** — the device, the pipelines, the atlas, the targets, the frame and
  its instrumentation. Depends on `quorra-scene` and on `wgpu`.
- **`quorra`** — the facade a caller names in its manifest, re-exporting both.

The dependency edge from `quorra-scene` to a device does not exist, so §2.3's corollary
is not a promise a reviewer has to check: a scene *cannot* touch a device, because there
is nothing in scope to touch. `Send + Sync` follows for the same structural reason — no
GPU resource can be reachable from a `Scene` if no GPU type is in scope.

## Consequences

The property is enforced by `cargo build` rather than by review, which is the whole
benefit and the reason a two-crate split was not enough on its own — the facade is what
makes the split invisible to the caller, so the structure costs them nothing.

A second benefit, unlooked for but real: `quorra-scene` compiles in a second and has no
GPU in its test environment, so the parts of the contract that are pure data — a clip
chain's intersection semantics, an empty clip admitting nothing, `Scene::cost()` — are
testable without an adapter at all. Those tests will run on any machine, including one
with no GPU and no `Xvfb`.

The cost is three manifests, three sets of lint attributes, and the ordinary friction of
a type that turns out to be needed on both sides of the line. `Viewport` is the first
such type: it is pure data and looks like it belongs beside geometry, but it exists only
to describe a target, so it lives in `quorra-gpu`. When another one appears, the rule
that decides it is: **if a scene could be built without it, it is not a scene type.**

A cost we should name rather than discover: a facade invites `pub use` of everything,
and a flat re-export of two crates' worth of names is a worse API than either crate. The
facade re-exports the names §2 of the brief actually puts in a caller's hands, and
re-exports the two crates themselves for the rest.
