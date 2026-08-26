# 0084 — The walk that remains, and the stall that guards the exact allocation

Date: 2026-08-27. Status: **accepted — a design round: it measures, it enumerates, and
it builds nothing.** ADR 0083 removed the walk's two largest terms and said the next
step "is not a term but the walk itself"; this round prices that step so it can be
taken deliberately, in stages, rather than begun as one heroic rewrite.

## Where the frame is now

The caller's worst page (58 009 distinct fills), `Coverage::Compute`, alternating
transforms so every frame is a zoom step, the 890M, steady frames ≈ **107–123 ms**:

| term | ms | what it is |
|---|---:|---|
| scene | 9–15 | the caller's translate walk (their `render-quorra`), rebuilt per zoom because their scene bakes the placement |
| encode | 20–26 | this library's walk: dispatch, clip resolution, charges, seats, instances, records |
| upload | 23–27 | of which **the count stall is 16–18** (measured around the poll); the rest is record and buffer writes and the arena's probes |
| readback | ~10 | headless only; a window never pays it |
| unattributed | ~25 | the device's own passes and copies, still untimestamped |

Three separable subjects, in the order they should fall.

## 1. The stall is the price of exact allocation, and every shortcut was checked

The count pass's total crosses to the host so the edge buffer is allocated exactly and
refused by name (principle 6). Alternatives enumerated and rejected:

- **Guarded emit against a guessed capacity, checked later** — an overflow detected
  after the present is a frame that already showed missing fills: a plausible wrong
  picture, principle 6's worst case, unacceptable for even one refresh.
- **A GPU prefix scan with the flag mapped at frame end** — the map completes after all
  queued work, so the mid-frame stall becomes an end-of-frame full sync: no better
  windowed, worse pipelined.
- **Fused count-and-emit with atomic bump allocation** — the edge order becomes
  scheduler-dependent, the deposit's summation order with it, and same-adapter
  determinism (kept by ADR 0082 on purpose) dies.
- **CPU-computable counts** — the count is the subdivision recursion, which is exactly
  the work ADR 0081 moved off the host.

What remains honest: shrink what the stall *waits for* (submit the seed copy and size-
independent allocations before the count; let the count overlap the frame's remaining
host work), and shrink the count pass itself (it re-walks every segment; a persistent
per-(outline, linear-bits) count cache has the atlas's key and the atlas's weakness —
cold exactly when needed). Neither is the order-of-magnitude step.

## 2. The scene rebuild is the caller's, and dissolving it has a named blocker

`render-quorra` bakes each page's placement into the scene (`Encoder::placed`), so a
zoom is a new scene and a 9–15 ms translate — although quorra's `Viewport` has taken a
full affine since ADR 0001, and a scene built in *arrangement space* (pages at their
layout offsets, the zoom and scroll one uniform) would survive every view change and
make `RetainedScene` mean something across a zoom for the first time. ADR 0368 priced
this as candidate (b) at 2.4 % and shelved it; it is worth more now because it is the
**enabler** for §3, not a saving in itself.

**The blocker is the scene contract's device-space entries** — above all
`Stroke::width` in device pixels (`paint.rs`, deliberate: §10.7.5's stroke adjustment
is resolution-dependent *by specification*), plus pre-rasterised meshes and
per-placement image filters. An arrangement-space scene must either restate those per
frame (a small per-frame side scene quorra cannot currently merge) or quorra must
learn scene-space strokes with the adjustment applied at encode from the viewport —
the honest design, since the encode already knows the viewport. That is its own ADR,
on the other side of the §4.5 contract, and the caller's voice in it matters.

## 3. The walk falls to retained records, in two stages

**Stage A — host-side replay.** When a retained scene is re-rendered at a new affine
and the previous encode was *replaceable* — root plan only; every op a rect instance
or a compute-tile quad; the few CPU tiles (strokes) listed for re-rasterisation; no
atlas quads, winding, masks, functions, images, children or residues — the encode is
not re-walked. A flat record list retained from the first encode (outline id and arena
range denormalised, control box, command transform, paint, style, in encounter order)
is replayed: compose, four corners, cull, seat, two instance writes per record, no
hashing anywhere. Estimated 5–8 ms against today's 20–26, and it subsumes §2's saving
once the scene survives. The admission test is the design's safety: anything not
replaceable re-walks, byte-for-byte as today.

**Stage B — the walk on the device.** The same records resident in a buffer, a place
pass computing boxes and culls under the viewport uniform, a scan for the sheet
layout, seats by a serial-on-device packer or a layout freed from shelf semantics, and
indirect dispatch — the encode becomes a constant-cost dispatch chain and the stall
of §1 dissolves into it. This is the vello-class endgame ADR 0698 priced from outside;
nothing about it should be built before Stage A has shown its admission test and its
record shape are right.

## Decision

Stage A is the next build, **after** the stroke-contract ADR of §2 — in that order,
because replay without scene survival saves only half a term, and scene survival
without the stroke answer is blocked. Neither is this session's: both want a fresh
round against this document, and §2 wants the caller consulted. Until then the shipped
state stands on its numbers: the worst page's zoom step at ~95–110 ms windowed, from
272 where the week began.
