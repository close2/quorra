# 0079 — The compute lane: Warnock read against the numbers, and the bytes that crossed three drivers

Date: 2026-08-25. Status: **accepted as a direction, with its gate measured**. Built in this
round: one test, `tests/compute_coverage_determinism.rs`. Everything else here is the design
record the caller's ADR 0698 asked this library for, plus the owner's follow-up question,
asked verbatim:

> should we have a look at warnock (GPU)? Either way take advantage of the GPU. we want to
> be as fast as possible

## The question, and what was measured before answering it

The caller's ADR 0698 priced a compute-class rasterizer on their retired vello backend
(their Radeon 890M): ~3.3 ms device-side per magnification for a dense text page against
~18 ms host-side today; ~50–75 ms extrapolated for their 58 003-fill worst case against
250–295 ms — with vello refusing the full page outright on its fixed 48 MiB buffers, which
is principle 6's argument restated by a panic. What that left open was *which* device-side
algorithm, and whether any of them can keep this library's determinism.

Three inputs were gathered this round.

**1. The field.** Warnock's 1969 recursive subdivision has exactly one massively-parallel
descendant with published numbers: MPVG (Ganacim et al., SIGGRAPH Asia 2014) — a
"shortcut tree … inspired by the seminal work of Warnock", on Nehab & Hoppe 2008.
Measured properties: **conflation-free by construction** (all layers sampled in one pass,
per sample; 8→512 samples nearly free), occlusion culling worth +33% on Paris-30k, and a
heavy scene-dependent preprocessing stage (192.9 ms and 94.7 MiB for Paris-30k) that must
rerun when content moves. Its measured rival, Li/Hou/Zhou 2016 (scanline / boundary
fragments, integer crossing tables), beats it ~10× on animated content and 2.5× over NVPR
at 32 samples; and Li's design is the named ancestor of vello's current "sparse strips"
rework (linebender issue #670), whose stated goals are conflation-artifact-free
compositing and memory not proportional to bounding boxes. Slug's per-pixel analytic
glyph evaluation entered the public domain in March 2026 (patent dedicated, MIT
shaders) — newly available for the text question.

**2. Occlusion, on the page that would want it** (the caller's probe, described in their
ADR 0698's session, run against their CPU oracle, since removed as their probes are):
Entwurf at a page fit — 58 009 commands, all opaque fills — has **3 854 commands (6.6%)
fully occluded and 99.8% of painted pixels visible**; a 16-px-tile census reads median 87,
p90 242, max 348 commands per occupied tile, identical before and after occlusion
culling. The mosaic abuts; it does not stack. **Warnock's distinctive win — culling
covered work — is worth nothing on exactly the workload that motivates the lane**, and
MPVG's own +33% was measured on art with deep stacking. What survives of the subdivision
lineage's appeal is per-sample resolution, which is not exclusive to it: a tile-list fine
stage resolves per sample just as well (MPVG resolves inside lattice cells; Li inside
boundary fragments; sparse strips inside strips).

**3. Determinism, measured rather than argued** — `tests/compute_coverage_determinism.rs`:
`raster/fill.rs`'s exact trapezoid arithmetic ported statement for statement to a WGSL
compute shader, one invocation per row so the per-cell deposit order is the CPU's own,
with the port's four named hazards handled (`floor(x + 0.5)` for Rust's ties-away
`round`; `x − 2⌊x/2⌋` for `rem_euclid`, exact by Sterbenz; a magnitude guard for the
non-finite slope; no reliance on WGSL `%`). Result, on every adapter this machine has —
**RADV (Vulkan), llvmpipe (Vulkan), radeonsi (GL): zero of 4 096 pixels differ, both fill
rules, 588 edges through every branch including the border cuts** — and byte-stable
across runs. The honest bound: all three share Mesa's compiler, and the specs promise
none of it — WGSL permits fusing and reassociation, leaves the rounding *direction* of
even a single addition unspecified, and naga emits no `NoContraction`. Cross-vendor
byte-identity of float pipelines is unachievable *by contract*; integer/fixed-point
arithmetic is exact on every conformant implementation and reproducible by construction.

## Decision

**A compute coverage lane in the tile-list shape, not a Warnock tree** — Li/sparse-strip
lineage: resident outlines, GPU flattening, per-tile work lists, fine rasterization
resolving **per sample within the tile** so that abutting fills composite without
conflation (the property the caller's owner is asking for as "colors", and which their
ADR 0699's 2× settled pass only halves). Occlusion culling is left out on the strength of
the measurement above; it can be revisited by argument if a stacking-heavy corpus page
ever makes it worth a probe.

**The determinism rule for the lane**: accumulation and coverage arithmetic in
fixed-point integers wherever byte-identity across adapters is claimed — that claim is
this library's to keep, not the driver's — with the float port's measured Mesa-identity
standing as evidence that a float prototype can be developed against the CPU oracle
byte-for-byte on this machine's three adapters before the fixed-point derivation is
finalized. The test stays as the gate: a vendor compiler that fuses will fail it with
named pixels, which is the failure doing its job.

**What binds from this library's own principles**, so the lane is quorra rather than
vello-with-our-name: count-then-allocate or fenced-growth memory (a refusal that names
the limit — the caller's page needs ~70 MB where vello's constant held 48 MiB); pipelines
compiled lazily off the launch path; the CPU lane stays exactly what it is — the oracle
and the fallback — and the scene contract's device-space entries (stroke widths,
pre-rasterized meshes, per-placement filters) move scene-side only as the lane reaches
them, each its own ADR.

## What was not decided

No schedule, no phase plan, no claim about glyphs (Slug's liberation is noted, not
adopted). The next concrete step is the lane's memory-counting design — the one part
neither the probe nor the survey answers — and it should arrive with a measurement of the
caller's per-tile census against a chosen tile size, which their probe already knows how
to take.
