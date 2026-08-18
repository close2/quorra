# The capture that read one present behind — round notes, 2026-08-18

`doc/notes-present-rate.md` §4 recorded a defect and named its fix without taking it:

> `examples/present_thread`'s existing step 6 … **failed once in five real-display runs**,
> at load average 25.22 … The cause is `present_until_settled`: it presents for a fixed
> 300 ms and then captures, and **a wall clock is not a synchronisation**.
>
> The right shape is *capture until two consecutive captures agree, then assert* … and
> **not** "retry until it passes", which is a gate that cannot fail.

This round takes it. The instrument is
`crates/quorra-gpu/examples/present_thread/settle.rs`; ADR 0068 is the decision, and
everything below is what was measured on the way to it.

## 1. What synchronisation exists at this seam. **None, and that is a finding**

`Presenter::present` submits a pass and calls `Queue::present`. Between that call
returning and the moment `xwd` can read the window back stand four stages, and not one of
them offers a wait this example can take:

| stage | what would answer | why it is not reachable |
|---|---|---|
| wgpu | a signal that the image was presented | `SurfaceTexture::present` returns `()`. `Queue::on_submitted_work_done` answers a different question — the *device* finished, not that anything reached a window. `src/surface.rs` asks for `desired_maximum_frame_latency: 2`, so two presents may be in flight before the third blocks |
| Vulkan | `VK_KHR_present_wait` / `vkWaitForPresentKHR` | **wgpu 30 exposes none of it** |
| X11 | the Present extension's `PresentCompleteNotify` | it is delivered on the X connection that *issued* the present, which is the one `wgpu` opened inside itself. The example's connection is winit's; it never sees the event, and an `XSync` on it orders nothing against another client's requests |
| XWayland + compositor | — | the X server here does not scan out. It turns the presented pixmap into a Wayland surface commit, and the compositor composites when it composites |

So the honest statement is **not** "a convergence criterion is a workaround for a wait we
did not find". It is that **the wait does not exist**, and a criterion over the captures
is therefore the correct instrument rather than a second-best one. Under `Xvfb` the last
two stages are absent, which is exactly why 300 ms was always enough there and why the
defect could only appear on the owner's display.

## 2. The criterion, and the half that §4 did not name

> **Present until two consecutive captures agree, and what they agree on is not what the
> window was last *proven* to show.**

§4's sentence stops at the comma, and the criterion that stops there **converges on the
stale window**: while the new picture is in flight, every capture reads the old one and
every pair of them agrees. That is the observed defect turned into a permanent green
gate — the very outcome §4 was guarding against when it rejected "retry until it passes".

Adding the second half makes the settles a **chain**: each one hands back a capture proven
to be the window as it is, and that capture is the baseline the next settle must differ
from. A chain needs a first link, and it is an **erase** — a present carrying no layers
leaves the presenter's own clear (ADR 0056; `src/present/pass.rs` loads the swapchain image
with `Clear(TRANSPARENT)`), and "all of the window is the clear" is a state named by the
*library* rather than by a previous capture. A stale capture during an erase reads the
picture, which is not the clear, so the first link cannot converge early either.

**It is still not "retry until it passes".** Nothing in `settle.rs` knows what the picture
should look like. Any picture that is stably not the previous one satisfies it, so a window
that settles on the *wrong* picture settles immediately and fails the assertions in
`main.rs`. The division is deliberate: `settle` answers "is the window showing the present I
issued", `main` answers "is that present right", and neither can quietly do the other's job.

## 3. The bound, and where it comes from

**64 capture rounds — a count, not a duration** (ADR 0052: *a claim about "how many" is a
count and is exact; a claim about "how fast" is a duration and this machine cannot measure
one*). It is derived from what can stand between a present and a readable window:

- `desired_maximum_frame_latency: 2` (`src/surface.rs`), so at most two presents in flight;
- one pending XWayland presentation per window;
- one compositor frame.

That is about **five refreshes**. Sixty-four is an order of magnitude above it. Each round
carries one present, so the bound is also at least 64 refreshes — 533 ms at 119.96 Hz —
*plus* 64 `xwd` round trips; but it is a ceiling that is never approached rather than a wait
that is always paid, which is what makes its exact value cheap. A settle that reaches it
fails with `NotSettled`, whose two arms name the count and which of the two ways it failed.

## 4. What 22 real-display runs say

RADV (`AMD Radeon 890M Graphics (RADV STRIX1)`) on the owner's display through XWayland
(`env -u WAYLAND_DISPLAY`), `--check`, 2026-08-18. `--check` shortens only the rate phase;
steps 1 to 9, including the two capture sites that flaked, run at full size either way.

| batch | runs | load average | completed | pixel assertions |
|---|---:|---:|---:|---|
| quiet | 10 | 7.75 – 8.27 | 10 | **all green** |
| loaded — 48 busy loops on 24 cores | 12 | 36.87 – 55.77 | 10 | **all green** |

**Twenty runs reached the end and every pixel assertion in every one of them passed.** Two
of the loaded runs refused earlier, in a different gate, and §5 is about them.

**Twenty green runs is not by itself a strong statement**, and it should not be quoted as
one: against the recorded rate of 1 in 5, twenty clean runs would happen by chance 1.2 % of
the time, and the 95 % upper bound on the true rate from 0 of 20 is still 14 %. The evidence
that actually carries this round is the next table, because it is about the **mechanism**
rather than about the outcome.

### The criterion was observed to fire, 27 times

Every settle prints how many captures it needed. Two is the minimum — one capture, then one
that agrees with it — so **any settle above two took a capture that was not the window's
settled contents**, which is precisely the capture the old instrument took and asserted on.

| what was being settled | 2 captures | 3 | 4 |
|---|---:|---:|---:|
| the presenter's own clear (erase, twice a run) | 15 | 13 | 12 |
| the page under the chrome (step 5) | 19 | — | 1 |
| the page under a linear filter, alone (**step 6, the one that flaked**) | 19 | — | 1 |
| a frame drawn through `Target::Surface` (step 8) | 20 | — | — |
| **all 100 settles** | **73** | **13** | **14** |

**27 of 100 captures taken first were not the settled window.** Under the old instrument
those are exactly the captures a run would have asserted against, and a 27 % per-capture
stale rate over the two capture sites that existed is entirely consistent with the 1-in-5
run failure rate §4 recorded. The criterion did not merely not fail — **it was seen doing
the work it exists for**, on the same machine, at loads spanning 7.75 to 55.77.

**And the bound has 16× headroom.** The largest number of captures any settle needed is
**4**, against a bound of 64. Nothing in this population came within an order of magnitude
of the refusal.

**The erase is where the lag concentrates**, and that is worth reading rather than
smoothing away: 25 of 40 erases needed a third or fourth capture, against 2 of 60 for the
three picture settles. The erase is the settle that immediately follows a *burst* of
presents carrying something else — the render phase's chrome, or the rate phase's whole
sweep — so it is the one with the deepest queue of stale images in front of it. It is also
the link that makes the rest of the chain sound.

## 5. A second flake, pre-existing, found by the load this round applied

Two loaded runs died before any capture, in an assertion this round did not touch
(`main.rs`, the render phase):

```
presents while one render of 25.378556ms was in flight: 1
thread 'main' panicked at crates/quorra-gpu/examples/present_thread/main.rs:
the point of the split is presenting during a render; only 1 got through
```

The gate is `presents >= 2`, and it means "at least one present *completed* while the
render was still running", which is a real property. But the number of presents that fit a
span is `span / refresh`, and **the span is a wall clock on a shared machine**: at load
36.87 the presenting thread managed one present in 25.4 ms — three refreshes — because it
could not be scheduled. The other failure is the mirror case: the render itself took 6.4 ms,
*less than one refresh*, so one present is the arithmetically correct answer and the
assertion is wrong about its own subject.

**This is not the settle criterion and it is not new** — the code is unchanged since
ADR 0056 — but it is the same family as `HANDOVER.md`'s *"read a gate's threshold against
the number it names"*: a count whose value is decided by a duration is a duration wearing a
count's clothes. It did not appear in `doc/notes-present-rate.md`'s five runs because those
ran at load 8.9 to 25.2; it appears at 36.9 to 47.2. `doc/notes-present-rate.md` §2 already
bounds the split's claims to *"a machine that is not oversubscribed"*, and 48 busy loops on
24 cores is twice past that boundary — so this is the recorded boundary arriving in a second
assertion rather than a new finding about the design.

**It is recorded and not fixed here**, for §4's own reason: deciding what that assertion
should be instead is a separate decision, and folding it into this round would mix two
subjects. §7 recommends it as the next item.

## 6. How each part was verified able to fail

`the_criterion_refuses_a_stale_window` is the **control**, and it is a control rather than a
test because the criterion's whole job is to refuse something — `HANDOVER.md`: *a gate whose
assertion is an absence needs a control*. It runs before anything opens a window, for
`arrangement::the_shapes_are_the_ones_adr_0058_counted`'s reason: it needs no display, so
nothing about a display can excuse it not running.

Four forced defects, all four red, all under `Xvfb` because every one of them is
display-independent — the criterion's arms are counts and comparisons.

**A. `judge` loses the half that refuses a stale window** (`ChangedFrom(_) => true`) — which
is `doc/notes-present-rate.md` §4's fix exactly as that section wrote it. The control fires
before a window is ever opened:

```
a stale window that is perfectly stable must never settle: that is the 300 ms wall
clock's failure wearing a convergence criterion's name
  left: Landed
 right: NotYet
```

**B. Nothing is presented at step 6**, so the window keeps showing what step 5 left:

```
the window never showed the page under a linear filter, alone: 64 presents, 64
captures, and every one of them read what the window showed before (all 307200 pixels
identical)
```

That is the arm that matters most. **The old instrument, in this exact situation, would
have captured the step-5 picture and asserted against it** — which is the shape of the
recorded failure. The new one refuses, names the bound, names the picture it was waiting
for, and says the last capture was identical to the baseline in all 307 200 pixels.

**C. Step 6 alternates the linear page and the clear**, so the window never holds still:

```
the window never held still on the page under a linear filter, alone: 64 captures, and
no two consecutive ones agreed on anything new; the last two differ by 258048 of 307200
pixels differ, first at (64, 32): [0, 0, 0] against [51, 102, 204]
```

(64, 32) is the page's own first covered pixel, which is the right place for the two
pictures to first disagree.

**D. The page's placement moved one pixel left** (`OFFSET` 64 → 63) — the case the task set
as "a wrong-but-stable window". The criterion **converges**, correctly, because the window
genuinely is showing what was presented, and the *assertion* reports:

```
settled on the presenter's own clear after 2 captures
settled on the page under the chrome after 2 captures
just left of the mark: the window shows [204, 51, 51], the scene says [51, 102, 204]
```

At an offset of 63 the mark's left edge moves from device 264 to 263, so the pixel that
must be field reads mark. **The criterion did not convert a wrong picture into a pass**,
and that is the division of labour working: `settle` said the window is showing what was
presented, and `main` said what was presented is wrong.

## 7. Recommended next

- **`presents >= 2` in the render phase is a duration wearing a count's clothes** (§5).
  Either the fixture's render is made unambiguously longer than a present by construction,
  or the assertion is restated as something a schedule cannot decide. One round, no corpus.
- Nothing else. This round changed no `src/`, drew no page differently, and owes no corpus
  run.
