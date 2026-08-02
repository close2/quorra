# ADR 0002 — wgpu rather than Vulkan directly

Status: accepted, 2026-08-02. Pinned at `wgpu = "30.0.0"`.

## Context

This library replaces a Vello-on-wgpu backend, and the brief's §2.4 already names
`wgpu::Texture` in the API it asks for:

> Tier 3: a texture we own, for a host that composites it itself.
> `Texture(&'a wgpu::Texture)`

So the caller expects to hand us its own wgpu texture and composite the result itself.
That is close to deciding the question, but not quite: a tier-3 host could be given a
Vulkan image handle instead, and the machine this is developed on is a Vulkan machine
(RADV on RDNA 3.5). The alternative was therefore real and worth stating: raw Vulkan
through `ash`, one backend, full control of memory, timestamps, subgroups and the
pipeline cache.

## Decision

`wgpu`, version 30.0.0, pinned in the workspace manifest with default features.

## Consequences

**What it buys.** The caller runs where its users are, not only where its developers
are; one Vulkan backend would exclude every machine that is not one. `wgpu` also gives
us, without our writing them: a validation layer that is on in debug builds, a shader
language with a compiler we do not maintain, the software adapter (lavapipe) that makes
the byte-equality question of §11.4 answerable in CI at all, and a safe API — which is
what lets principle 3's `#![forbid(unsafe_code)]` hold across the whole tree rather than
being aspirational.

**What it costs, stated so that a later measurement can argue with it.**

- **A layer between us and the timings we are required to report.** §8 wants
  `execute` from timestamp queries and per-pass `phases`. `wgpu` exposes timestamp
  queries, and M1 is where we find out whether the resolution and the pass boundaries
  are good enough to attribute a regression. If they are not, that is a finding to write
  down, not a reason to swap the abstraction.
- **Less control of allocation than principle 6 wants.** "Memory that grows" is our
  answer to Vello's hand-picked constants; `wgpu` buffer creation is a coarser tool than
  a Vulkan allocator, and the two-pass count-then-allocate design has to live within it.
- **The pipeline cache is the driver's, through `wgpu`'s door.** §7 asks for a
  persistable blob; `wgpu` offers `PipelineCache` on backends that support it, and
  whether the blob survives a driver update is the driver's answer, not ours. Reporting
  a rejected cache rather than silently recompiling is the part that is ours.

**Why the version is pinned exactly, and how it was checked.** A renderer's output can
change with its shader compiler, and §4.6 promises byte equality; a floating minor
version would let that promise be broken by a `cargo update`. 30.0.0 was verified to
build on the pinned toolchain (1.97.1, edition 2024) before the pin was written down —
its own MSRV is 1.87.0. Default features are taken for now: `vulkan`, `gles`, `dx12`,
`metal`, `webgpu` and `wgsl` are all on by default in 30.0.0 and the off-target backends
compile to nothing. Trimming them is a startup-cost question (§7) and needs the
measurement first.
