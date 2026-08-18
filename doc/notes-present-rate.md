# The presenter at 120 Hz — round notes, 2026-08-17

Two numbers were parked with the same reason attached, and the reason has expired.

> **ADR 0056**: *"We cannot answer whether it holds the rate. `Xvfb` reports a refresh rate
> of 0.00 and 120 Hz cannot be observed on this machine at all."*

> **ADR 0058**: *"What the pass costs inside a real frame is still not ours to say … what
> share of a refresh a present takes is the owner's measurement on the display that states
> one."*

Both were taken on the owner's display — `eDP-1`, **2880 × 1800 at 119.96 Hz**, one refresh
**8.34 ms** — through the loop that runs a script from this account on the real GPU. The
instrument is `crates/quorra-gpu/examples/present_thread/`, **extended rather than
replaced**: `arrangement.rs` (exact arithmetic, no adapter in it) and `rate.rs` (the
display's own clock) beside the pixel proof that was already there. CI runs all of it under
`--check`.

**The short answers.** The split holds the rate exactly — **149 of 149 presents landed on the
next refresh** while a render held the device, in four runs at load averages 8.9 to 12.2, at
1.02 presents per refresh of the span. And the present pass is **0.367 ms, 4.4 % of a
refresh**, at the caller's four-layer arrangement at the window this could be measured at;
scaled to their own window by fragment count, **11.4 %**. So ADR 0058's own guess about
itself was right: *"if it is 0.3 ms, this bought them a fifth of a percent and its real value
is that sizing a layer now pays."*

**And the boundary, because a claim with no boundary is not a measurement.** A fifth run at
load average 23.74 misses 2 refreshes of 37 and reads the pass 1.9× slower. Everything below
is a statement about a machine that is not oversubscribed, and the instrument says so rather
than averaging it away.

## 0. Which numbers here are exact and which are indicative

ADR 0052's seam decides it, and this round is unusually clean along it, because **the
display's refresh is a clock we did not build and cannot bias.**

| kind | what it is | trust |
|---|---|---|
| fragment counts | arithmetic over the placements | **exact** |
| refreshes each present consumed | a count, quantised against the measured refresh | **exact as a count** |
| presents per refresh of a render span | a count over a stated span | **exact as a count** |
| the largest replication that still lands every refresh | a count, bracketing the pass against the display's clock | **exact as a bracket** |
| median present intervals, and the per-copy slope read off them | two host wall clocks, minima of three round-robin rounds | **indicative** |
| anything scaled to the caller's 2048 × 2560 window | the measured per-fragment rate times a fragment count | **a model, and labelled as one everywhere below** |

## 1. The window, and why it is not the caller's

**The caller's window is 2048 × 2560** — their `QUORRA_NONBLOCKING_RENDER.md` §2's
1280 × 1600 at a device scale of 1.6 — and **2560 rows do not fit an 1800-row display.** So
everything was measured at **1280 × 1600**: their window at a device scale of 1, every layer
extent divided by 1.6 and rounded, every placement divided by 1.6 and not rounded. The
example computes its counts from the size the window system actually gave rather than the
size it asked for, so a window manager that refused would have produced honest numbers for a
different window rather than pretty ones for this one. It did not refuse.

| layer | extent | placed at | rectangle fragments |
|---|---|---|---:|
| page | 980 × 1386 | (300.00, 106.88) | 1 362 609 |
| selection — *content-sized* | 750 × 163 | (400.00, 562.50) | 124 832 |
| sidebar | 300 × 1600 | (0, 0) | 481 600 |
| modal card | 1280 × 1600 | (0, 0) | 2 048 000 |

| arrangement | full-screen triangle | rectangle | share |
|---|---:|---:|---:|
| window-sized overlays — **the caller's shape today** | 8 192 000 | 7 506 609 | 91.6 % |
| content-sized overlays | 8 192 000 | 4 017 041 | 49.0 % |

Those shares are ADR 0058's 91.6 % and 49.0 % to the decimal, at a window 2.56× smaller,
which is the check that these are the same four layers.

**One of the four extents was recovered rather than read**, and it is worth writing down
because the recovery is now a gate. `doc/notes-present-quad.md` §1 records the page, the
sidebar and the modal from the caller's tree and leaves the selection at *"see below"* — a
selection has no natural size. What it does record is that instrument's totals, and two of
those rows (with the modal and without it) are two equations in the one unknown: both give a
dilated area of **314 924 fragments**, which is a 1200 × 260 rectangle. `arrangement.rs`
carries the caller's four layers at *their* window purely so that
`the_shapes_are_the_ones_adr_0058_counted` can reproduce **19 210 251 / 10 270 775 /
5 027 895** exactly — and it does, which is what makes the shapes measured here provably the
shapes ADR 0058 decided on.

## 2. ADR 0056's number. Does the split hold 119.96 Hz? **Yes, exactly**

The library configures the surface `PresentMode::Fifo` and offers no other mode, so presents
are refresh-locked by construction. That makes the honest question a property rather than a
duration: **does the presenting thread make its 8.34 ms window** while the device is busy on
another thread?

**The refresh, measured rather than assumed.** A run of presents with nothing to draw
measures the rate the display released images at. Over 120 such presents, in each of four
runs: **8.300, 8.315, 8.342, 8.322 ms** — **120.48, 120.27, 119.88, 120.16 Hz** — against the
8.34 ms and 119.96 Hz the display states. That is agreement to half a percent through a whole
compositor, and one of the four reads the stated figure exactly.

**The cadence.** A thread takes the device and renders §6.2's dense-text page repeatedly for
about 300 ms while the main thread presents the caller's four layers. Four runs, two loop
rounds, load averages 12.21 / 10.94 / 10.71 / 8.89:

| run | span | renders in it | presents | presents per refresh of the span | refreshes each present consumed |
|---|---:|---:|---:|---:|---|
| 1 | 306.1 ms | 179 | 38 | **1.03** | **1 refresh × 38 (100 %)** |
| 2 | 303.3 ms | 69 | 37 | **1.01** | **1 refresh × 37 (100 %)** |
| 3 | 302.2 ms | 133 | 37 | **1.02** | **1 refresh × 37 (100 %)** |
| 4 | 302.3 ms | 76 | 37 | **1.02** | **1 refresh × 37 (100 %)** |
| 5 — *load 23.74* | 300.2 ms | 41 | 37 | 1.04 | 1 × 35 (94.6 %), **2 × 1**, **3 × 1** |

**Every present landed on the next refresh. Not one was missed, in any of the first four
runs — 149 of 149.** The `presents per refresh` slightly above 1 is the measured refresh
being a fraction under the stated one, not a present arriving twice.

**Run 5 is the fifth run and it is deliberately in the table**, because it is the one that
says what the first four are a statement about. It was taken while another agent's release
build had the load average at **23.74**, twice the 8.89 – 12.21 of the others, and there the
presenting thread misses **2 refreshes of 37**. That is the right answer, not a defect: a
thread that has to be scheduled to present will miss a refresh on a machine with nothing left
to schedule it with. **So the claim is "the split holds 119.96 Hz on a machine that is not
oversubscribed", and the instrument is sensitive enough to say when it does not** — which is
what makes the 149-of-149 rows worth anything.

**What the failure would have looked like**, so that "it held" means something: the histogram
would carry a `2 refresh ×n` bucket — a present that took two refreshes to land — and
`presents per refresh` would fall to 0.5. That is exactly the shape the caller measured
before the split and recorded in ADR 0056's context: *a median interval between presents of
167.4 ms against a refresh of 8.333, with 1 present in 23 landing on the next refresh.* Here
it is 38 in 38.

**One honest correction to how this number should be quoted.** ADR 0056 and
`doc/answer-nonblocking-render.md` say *"three to five presents through a single render"*,
measured under `Xvfb`. That ratio is a statement about **the fixture's render**, not about
the split: on RADV a dense-text render is 1.7 ms in run 1 and 4.4 ms in run 2, so 179 and 69
of them fit the span and the presents-per-render figure is 0.21 and 0.54 — *below one*, and
it would be alarming if anyone read it as a regression. **The count that carries the claim is
presents per refresh, and it is 1.00.** The caller's page is the case where the two coincide:
their frame is 4 454.9 ms, so one of their renders spans 534 refreshes and, on this evidence,
would get 534 presents where their measured arrangement got 3.

## 3. ADR 0058's number. What share of a refresh does the present pass take?

The pass cannot be timed from the presenting thread without stalling it, which is why
`PresentCost` deliberately carries no timestamp query (ADR 0056). So it is loaded instead:
**`n` copies of the caller's four layers in one present**, which costs `n` passes. Under
`Fifo` the interval between presents is `max(refresh, what one present costs)`, so the row at
which the interval leaves the refresh is where the pass crossed 8.3 ms — and that is a
**count**, read against a clock the display owns.

Minima of three round-robin rounds of 32 presents, at 1280 × 1600, window-sized overlays
(7 506 609 fragments per copy):

| n | run 1 | run 2 | run 3 | run 4 | refreshes/present |
|---:|---:|---:|---:|---:|---:|
| 1 | 8.254 ms | 8.329 ms | 8.288 ms | 8.311 ms | 0.99 – 1.00 |
| 2 | 8.332 ms | 8.284 ms | 8.309 ms | 8.295 ms | 1.00 |
| 4 | 8.303 ms | 8.327 ms | 8.297 ms | 8.292 ms | 0.99 – 1.00 |
| 8 | 8.301 ms | 8.310 ms | 8.320 ms | 8.301 ms | 1.00 |
| 16 | 8.377 ms | 8.295 ms | 8.509 ms | 8.281 ms | **1.00 – 1.02** |
| 32 | 11.432 ms | 11.036 ms | 12.023 ms | 11.734 ms | 1.33 – 1.44 |
| 64 | 23.225 ms | 23.139 ms | 23.768 ms | 26.739 ms | 2.78 – 3.21 |

**The bracket, which is the exact part.** Sixteen copies of the caller's whole four-layer
arrangement — **120 105 744 fragments in one present** — still land every refresh in all four
unloaded runs; thirty-two never do. So one present of that arrangement is **at most 1/16 of a
refresh, 0.52 ms**. (Under run 5's load the bracket honestly moves to 8, which is the same
statement as its slower slope and not a second finding.)

**The slope, which is the indicative part.** Rows 1 through 16 are floored by the display and
say nothing about the pass — dividing a floored row by `n` measures the refresh divided by
`n`, which is why the example prints `(floored)` there rather than a number. The difference
between the two loaded rows divides out everything that does not scale with the layer count
(the acquire, the clear, the submission, the event pump):

| run | (n=64 − n=32) / 32 | of a refresh | load average |
|---|---:|---:|---:|
| 1 | **0.369 ms** | 4.4 % | 10.94 |
| 2 | **0.378 ms** | 4.5 % | 8.89 |
| 3 | **0.367 ms** | 4.4 % | 12.21 |
| 4 | 0.469 ms | 5.6 % | 10.71, and 17.40 by the end of it |
| 5 | 0.680 ms | 8.1 % | **23.74**, and the bracket falls to 8 copies rather than 16 |

**Take the minimum: 0.367 ms, 4.4 % of a refresh.** Three of five runs agree to 3 %; run 4's
load average had risen to 17.40 by the time it finished and run 5's stood at 23.74
throughout. That is `HANDOVER.md`'s trap arriving on schedule — *"wall clocks lie under load,
and this machine is somebody's desktop"* — and it is why every row is printed with its load
beside it rather than averaged in. **A reader who took the mean of these five would publish
0.46 ms and be 25 % wrong.**

**The two instruments agree**, which is the reason to trust either: 8.31 / 0.367 = 22.6
copies is where the crossing should be, and it is observed between 16 (holds, four times out
of four) and 32 (does not, four times out of four).

### Scaled to the caller's own window — a model, and here is the model

The measured rate is 0.37 ms / 7 506 609 fragments = **49 ps a fragment on the swapchain
image**. Multiplying that by ADR 0058's own counts at 2048 × 2560:

| arrangement at the caller's 2048 × 2560 | fragments | modelled | of a refresh |
|---|---:|---:|---:|
| before ADR 0058 — four full-screen triangles | 20 971 520 | 1.03 ms | 12.4 % |
| window-sized overlays, **their shape today** | 19 210 251 | 0.95 ms | 11.4 % |
| content-sized overlays, if they size their layer textures | 10 270 775 | 0.51 ms | 6.1 % |

So **ADR 0058 bought them 0.09 ms — 1.0 % of a refresh — at the arrangement they have**, and
**0.53 ms, 6.3 % of a refresh, at the one they could have.** ADR 0058 predicted this about
itself in its own decision section and it is confirmed: *the milliseconds are not the value;
the value is that a host sizing its layers now gets something for it, where before the pass
cost `layers × window` whatever a layer's size.*

### And a caveat of `doc/notes-present-quad.md` resolves, in the favourable direction

That round timed the pass with device timestamp queries into an **offscreen** `Rgba8Unorm`
target and said so: *"a surface's image may differ in tiling or compression, so these are the
pass's shape rather than the window's cost."* It read **1.44 – 1.49 ms** for window-sized
overlays at 2048 × 2560 — 76 ps a fragment, against the **49 ps** measured here on the
swapchain. The window is about **1.5× cheaper per fragment than the texture**, and the gap is
if anything understated: the marginal copy in this round carries four bind groups and four
uniform buffers of host work that the timestamped pass did not include. **Nobody should quote
the offscreen figure as the window's cost** — it overstates it by half again.

## 4. One defect found in the pixel proof, not fixed here, and why

`examples/present_thread`'s existing step 6 — the `ImageFilter::Linear` present, asserted by
reading the window back — **failed once in five real-display runs**, at load average 25.22:

```
thread 'main' panicked at examples/present_thread/main.rs:459:
where the chrome was: the window shows [0, 204, 51], the scene says [51, 102, 204]
```

`[0, 204, 51]` is the *chrome*, which the previous present carried and this one did not. So
the window `xwd` read was **one present behind** — the assertion is right and the picture
arrived late. The cause is `present_until_settled`: it presents for a fixed 300 ms and then
captures, and **a wall clock is not a synchronisation**. Under `Xvfb` there is no compositor
between the present and the dump and 300 ms was always enough; through a real compositor at
load 25 it is not.

**It is recorded and not fixed in this round**, deliberately. The right shape is *capture
until two consecutive captures agree, then assert* — "wait until the window is stable", which
can still fail on a stable-but-wrong window — and **not** "retry until it passes", which is a
gate that cannot fail. That is a change to how a green proof synchronises, on the evidence of
one observation, and it deserves its own round rather than being folded into a measurement.
Nothing this round touched caused it: the pixel proof runs at 640 × 480 *before* the rate
phase resizes anything, and it is a pre-existing wall clock newly exposed by being run
somewhere it had never run.

> **Taken on 2026-08-18 — ADR 0068, `doc/notes-present-settle.md`.** The wall clock is
> gone. **One correction to the fix named above**: *"capture until two consecutive captures
> agree"* is not sufficient on its own — while the new picture is in flight every capture
> reads the old one and every pair of them agrees, so that criterion settles instantly on
> exactly the stale window this section records, and is green for ever. The criterion built
> is *"two consecutive captures agree **on something other than what the window was last
> proven to show**"*, which makes the settles a chain and needs a first link (an erase to
> the presenter's own clear). It was measured doing the work rather than merely passing:
> over 20 completed real-display runs at loads 7.75 to 55.77, **27 of 100 settles took a
> first capture that was not the settled window** — the capture the old instrument asserted
> on. And the search for a synchronisation to use instead came back empty and that is a
> finding: wgpu 30 exposes no present-completion signal and `VK_KHR_present_wait` at all,
> and the X Present extension's `PresentCompleteNotify` goes to the connection `wgpu` opened
> inside itself.

## 5. Two traps this round paid for

**`xwd` cannot see a Wayland window, and the failure does not say "Wayland".** `present_thread`
had only ever run under `Xvfb`. On the owner's machine it opened its window, warmed its
device, rendered on a thread, presented — and then died on
`xwd: error: No window with name quorra presenter thread exists!`. It is not renamed and not
reparented: `xdotool search`, `xwininfo -root -tree` and a poll every 50 ms for five seconds
all find **nothing**, because `XDG_SESSION_TYPE=wayland` and winit prefers the Wayland backend
whenever `WAYLAND_DISPLAY` is set, so the window is not in the X tree at all. Running with
`env -u WAYLAND_DISPLAY` takes the X11 backend, the compositor's XWayland server puts the
window on the same display at the same refresh, and every one of the example's pixel
assertions passes on the real GPU — the first time they have. **`CLAUDE.md`'s "Session: X11"
is stale** and cost this round two loop rounds to discover.

**Under `Fifo`, the minimum interval is not the refresh.** The first version of this
instrument took the refresh as the *minimum* interval over a run of empty presents, on the
reasoning that no interval can be shorter than a refresh. The measured minima are **0.201 ms
and 0.064 ms** — a swapchain has more than one image and a burst gets through whatever
settling you do, and one such interval would have divided every other number in this
document by forty. **The median is what a run of presents with nothing to draw measures**, and
it reads 8.300 and 8.315 against a stated 8.34. Both are printed, which is how the trap
became visible rather than becoming a result.

## 6. What was built, and how each part was verified able to fail

`crates/quorra-gpu/examples/present_thread/` gains two modules and keeps its nine existing
steps unchanged:

- **`arrangement.rs`** — the caller's four layers, `Shape`, the rectangle-fragment model of
  `Layer::device_bounds`, and the gate that reproduces ADR 0058's three published totals.
  Exact, and it needs no window, no device and no adapter.
- **`rate.rs`** — the refresh probe, the cadence phase and the replication sweep, with
  `SETTLE_PRESENTS` discarded at the start of every run and the whole sweep round-robin so
  drift falls on every row.
- `main.rs` gains `Display::size` and `Display::resize` (which reports the size it *got*),
  and `fixture::through_the_surface` takes a window size rather than reading a constant —
  because the rate phase resizes the window and a scene built for the old size would leave
  the rest of it transparent by ADR 0039, which looks like a defect and is not one.

| gate | forced defect | what happened |
|---|---|---|
| `arrangement::the_shapes_are_the_ones_adr_0058_counted` | the page's extent 2217 → **2218** | ``window-sized overlays: this file's shapes draw 19211820 fragments at 2048 x 2560; ADR 0058 and doc/notes-present-quad.md §2 count 19210251`` — a difference of exactly 1569, the page's clamped width |
| the sweep's own replication | `for _ in 0..copies` → `1..copies`, one copy short | `n = 1 must draw 4 layers; left: 0, right: 4` |

**And one thing the first forced defect taught, which is worth more than the gate.** The
first attempt moved the page's **width** 1568 → 1569 and **the gate passed.** The page sits at
x = 480 with a width of 1568, so its right edge is at 2048 — the window's own — and the
outward pixel is clamped away on that axis at either width. A layer at the edge of its window
absorbs an error in the direction of the edge, which means *a fixture near a boundary is a
fixture that can be wrong on that axis without anything noticing.* Same family as
`HANDOVER.md`'s "a clip that overlaps nothing looks exactly like a clip that works": **when a
gate's subject is a number, force the defect on the axis where the number can move.**

**`--check` needs a window, and says why.** The arithmetic gate does not, but the phases
around it do, and `present_thread` has needed one since ADR 0056 — CI already runs the whole
example under `Xvfb`, where the rate phase does one render, six presents and one sweep row
and prints no statistics. Under `Xvfb` the refresh probe measures llvmpipe rather than a
display (0.467 ms, "2143 Hz"), which is precisely ADR 0056's original complaint and is why
nothing in the rate phase is asserted against a duration.
