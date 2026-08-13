# ADR 0040 — What a first frame pays has no size

Status: accepted, 2026-08-13. Re-measures ADR 0035, withdraws its headline number, and
moves what a first frame actually pays to the thread nobody blocks on.

## Context

ADR 0035 gave a host `Device::warm_for(width, height)` and measured a first frame going
from 24.7 ms to 10.3 with it. Four ADRs since have eroded what it warms: a layer became as
big as its plan (0036), a mask as big as its own (0037), a plan kept one accumulator
instead of a pair (0038), and the root became as big as what the page marks (0039). The
warmed texture is the target's size, so it is now claimed only by a root that marks the
whole target — **26 % of the 7.9 % of the caller's corpus frames that allocate a root
texture at all**, which is two frames in a hundred (ADR 0039's measurement, scale 4).

The obvious repair is to let `LayerPool::acquire` serve a smaller plan out of a larger
texture, which every pass would then have to bound with a viewport, because the lane
shaders map device space to clip space by dividing by the attachment's extent. ADR 0036's
hazard note is about exactly that class of change: a pane taught three places to subtract
an origin, shipped with one missing, and drew nothing at all for every band after the
first.

**So price the thing first.** What follows is the measurement, and it says the repair is
not worth making — and that the mechanism it would repair was never worth what ADR 0035
credited it with.

## How these numbers were taken

RADV, release, **one device per process** — a second device in a process finds every heap
warm — driving a probe that rendered four frames of a fixture into `Target::Readback` and
printed `Timings`. Configurations were run **round-robin, one process each per round**, so
that drift in machine load falls on all of them equally, 27 to 40 rounds each. The machine
was somebody's desktop with other work on it throughout: load averages between 20 and 235.
**Every frame figure below is therefore a minimum**, and the two that carry the argument
are not frame differences at all but direct spans: the duration of `create_texture`, and
the pipeline-compilation entry a frame reports in `Timings::phases`.

The fixture is ADR 0035's in shape: eight isolated groups over a page, at 1 191 × 1 684 and
at 2 448 × 4 752, plus a flat variant with no group at all (nine corpus frames in ten are
flat) and a variant whose groups mark a tenth of the page (the root such a page allocates is
a tenth of the target). `host` below is the wall clock around `render` less
`Timings::readback`, which takes the demultiply and the map wait — both CPU-bound and both
the loudest thing on a loaded machine — out of the number.

## The measurement

**1. The allocation the hint moves costs 0.06 ms.** `warm_for` timed from the caller's
side, one call per process: **min 0.035 ms and mean 0.076 over 80 processes at
1 191 × 1 684, min 0.044 and mean 0.081 over 40 at 2 448 × 4 752**. Broken out by
`create_texture` calls on a fresh device:

| | first call | second | third |
|---|---:|---:|---:|
| 1 191 × 1 684 (8 MB) | 0.040 ms | 0.006 | 0.003 |
| 2 448 × 4 752 (46 MB) | 0.053–0.237 | 0.011–0.041 | 0.006–0.007 |

A 46 MB texture is created in sixty microseconds because RADV commits the memory when the
GPU first touches it, not when the allocation is made. **The whole budget of this
mechanism is 0.06 ms**, which is what any repair of it can buy.

**2. Calling it and not calling it are indistinguishable.** First-frame `host` minima over
40 rounds, three arms — no hint, the hint as it ships, and the hint made and dropped
before the frame (ADR 0035's own note about the driver keeping a freed allocation warm):

| fixture | none | `warm_for` | made and dropped |
|---|---:|---:|---:|
| 1 191 × 1 684, eight groups over the page | 22.48 | 23.05 | 22.14 |
| 1 191 × 1 684, groups over a tenth of it | 5.17 | 5.19 | 5.41 |
| 1 191 × 1 684, flat | 3.80 | 3.71 | 3.58 |
| 2 448 × 4 752, eight groups over the page | 81.28 | 79.99 | 79.93 |
| 2 448 × 4 752, groups over a tenth of it | 6.70 | 6.76 | 6.44 |

The first row is the case the hint was built for — the root marks the whole target, so the
pool *does* take the warmed texture — and it is the row where the hint reads slowest. The
differences are under a millisecond and they change sign. **ADR 0035's 24.7 → 10.3 does not
reproduce**, in a fixture of that shape, at either size.

**3. The excess does not scale with the target.** First frame less the steady frames after
it, both minima, no hint, a 30-round sweep of its own:

| fixture | first − steady |
|---|---:|
| 512 × 512, flat | 1.5 ms |
| 1 191 × 1 684, flat | 1.9 |
| 2 448 × 4 752, flat | 3.4 |
| 1 191 × 1 684, eight groups | 5.7 |
| 2 448 × 4 752, eight groups | 5.2 |

Forty-four times the pixels buys about twice the excess across the flat rows, and the two
grouped rows differ by **six times the area and by nothing at all**. Having a group costs
more than being large.

The caller's §9 said the excess was **flat across target sizes** and was right; ADR 0031
paraphrased it as "it scales with the target — page-sized textures, their bind groups, and
the driver's first touch of a memory heap that size", and ADR 0035 built an API on the
paraphrase.

**4. Where the excess is: two pipelines, compiled inside the frame.** A layered first frame
reports **exactly two `pipeline compile (first use)` entries in `Timings::phases`**, in
40 frames of 40 — `Kind::Composite` and `Kind::Blit`, which no page without a group ever
asks for. Their total: **0.75 ms at the minimum, 1.53 at the median of 40 loaded runs, and
2.6 ms on the quietest single run observed** (1.7 + 0.9). A flat first frame compiles
nothing and still costs 1.5 to 2 ms more than its successors, which is the driver's first
submission and is not ours.

§9's own reason for ruling compilation out — *settling a second between bring-up and the
first render changes nothing* — is sound about the warm-up thread and says nothing about a
**first-use** compile, which happens inside the frame that needs it however long anyone
waited. That is the inference this ADR overturns.

**5. Only a whole frame warms anything, and a 64 × 64 one warms most of it.** Five kinds of
warm-up before the measured frame, at 1 191 × 1 684, 30 rounds, as first − steady minima:

| what happened before the measured frame | flat | eight groups |
|---|---:|---:|
| nothing | 1.14 | 2.23 |
| an empty command buffer, submitted and waited for | 0.67 | 2.37 |
| a **clear of an attachment the target's size**, submitted | 0.68 | 2.61 |
| a throwaway frame at 64 × 64 | 0.31 | 1.12 |
| a throwaway frame at the target's size | 0.09 | 0.50 |

The one row that is about size buys nothing an empty submission does not. What warms a first
frame is a frame — and three quarters of what a target-sized one buys is already bought by a
64 × 64 one, which is the shape of a cost that is mostly not about the size.

And the target-sized warm frame *costs* what a frame costs: **4.7 ms at 1 191 × 1 684 and
22.3 ms at 2 448 × 4 752** on the calling thread, minima over 40 rounds, against 3.4 ms for
the 64 × 64 one. A hint that spends 22 ms to save 2 is not a hint.

## Decision

**The warm set gains `Composite` and `Blit`.** The two compilations a layered first frame
pays for are size-independent, so no size hint could ever have reached them, and the
background thread ADR 0018 already gave the device can. `warm_up_now` compiles four
pipelines where it compiled two.

**`warm_for` keeps its signature, its contract and its one texture**, and its doc comment
now says what it is worth: 0.06 ms, not fourteen milliseconds. It is public API, it costs a
caller nothing, and the promise it makes is one a driver could still honour — an
implementation that commits memory at allocation rather than at first use would make it
matter again. What is not kept is the claim.

**`LayerPool::acquire` goes on matching on exact extent.** Serving a smaller plan from a
larger texture would buy 0.06 ms and would put a viewport in every pass that renders into a
layer, which is ADR 0036's hazard with a new name. Refused, with the number written beside
it in `layers.rs`.

**Neither `warm_for` drawing a whole frame nor a device remembering the last frame's root
region is taken.** The first costs more than it saves at page sizes and its benefit is
size-independent anyway; the second is a better prediction of a quantity that is worth
0.06 ms.

## What it bought

The two compilations leave the frame: **40 layered first frames of 40 report no pipeline
compilation at all**, where 40 of 40 reported two before. `tests/device_lifecycle.rs`
asserts that as a property — the absence of the phase, not a duration — because a wall
clock on this machine is context and not evidence.

Frame minima moved in the same direction and by more than the compilations weigh (a layered
first frame's `host` excess reads 10.9 ms before and 6.1 after, in a 40-round alternation
under a load average near 60). **That surplus is not claimed**: it is the difference of two
noisy minima, and the compilations' own duration is the part that was measured directly.

## What it cost

**`is_warm` arrives about 1.1 ms later.** The warm set's own duration, measured by
`StartupTimings::pipeline_compilation` in the same alternation: minima **6.07 ms before,
7.15 after**; medians 14.3 and 16.3 under load. Nothing blocks on that thread — the caller
renders page one on the CPU while it runs — but a host that calls `wait_until_warm` before
its first frame pays the 1.1 ms there instead of in the frame, and for that host this
change is a wash rather than a win.

**A host with no thread to give pays the 1.1 ms in its constructor.** `spawn_warm_up`
compiles the set inline when a thread cannot be spawned, which §2.1 requires and which this
makes correspondingly more expensive — the documented cost of that host, one pipeline pair
larger.

**A device that never composites now compiles two pipelines it never uses.** Nine corpus
frames in ten are flat. This is the trade §7 names — "Only the pipelines a page of text
needs may be on the critical path" — resolved on the ground that the *background* thread is
not the critical path, and that a page with a group is ordinary rather than exotic.
`Reduce` (soft masks), the knockout variants, the image and shading lanes and the winding
lane stay lazy, which is the same rule applied to rarer things.

**ADR 0035's headline number is withdrawn.** It is a measurement nobody can reproduce on
this machine today, and the honest reading is that a first frame on a quiet machine in that
session cost what it cost for a reason the fixture could not attribute. The API it produced
survives it, documented at its measured worth.

## What is left, and why it is not done here

**Every pipeline is keyed by (kind, target format), and a surface's format is not
`WARM_FORMAT`.** `SurfaceState` negotiates `Bgra8Unorm` where it can; the warm set compiles
`Rgba8Unorm`. So a presenting host's first frame compiles the lane it draws with — and, if
the page has a group, the blit as well — inside that frame, exactly as a layered frame did
before this ADR. The device knows the surface's format at construction, one line above
`spawn_warm_up`, so the fix is small.

It is not taken here because **this account cannot measure it**: a surface frame needs a
window, the agent user has no X authority for the owner's display, and `Xvfb` with
`lavapipe` would measure a software compiler rather than the one a viewer's first frame
waits for. It is the largest identified first-frame cost still on the caller's presenting
path, and `HANDOVER.md` carries it as work with a measurement attached to it.
