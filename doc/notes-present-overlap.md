# The count that was a duration — round notes, 2026-08-22

`doc/notes-present-settle.md` §7 recommended this and did not take it:

> **`presents >= 2` in the render phase is a duration wearing a count's clothes** (§5).
> Either the fixture's render is made unambiguously longer than a present by construction,
> or the assertion is restated as something a schedule cannot decide. One round, no corpus.

This round takes the second option. ADR 0071 is the decision; everything below is what was
measured on the way to it.

## 1. Why the first option was not taken

"Make the render unambiguously longer than a present by construction" reads like the safer
of the two, and it is the one that cannot be built honestly here. Making the render longer
*by construction* means synchronising it — the render thread waits until the presenting one
has got through — and a thread that is waiting is not a thread holding the device. The flag
would then say "still in the phase" where it is supposed to say "still rendering", and the
proof would be about the fixture's plumbing.

Bounding the loop and letting the **presenting** thread stop it gives the same certainty
with the device busy throughout: the render thread renders back-to-back, and the first
present to return finds it still doing so.

## 2. The ordering, and the two flags it needs

`RenderHold::holding` is raised before the first render and lowered after the last. The
presenting thread reads it **after `present` returns**; `RenderHold::proven` tells the loop
it can stop.

Two details are load-bearing and both are in the code as comments:

- **The scene is built before the flag goes up.** `fixture::page()` is 18 000 rectangles,
  and a present that overlapped a scene build would prove nothing about a device held by a
  render.
- **The loop checks `proven` after completing a render, not before starting one.** So the
  ceiling never truncates a render in flight, which is what run 12 below exercises.

## 3. What the ceiling is, and what it is not

`RENDER_HOLD_CEILING` is 300 ms — the same shape as `rate::CADENCE_SPAN`, a stopping rule
rather than a measurement. It exists so that the regression the gate is for fails the phase
instead of hanging it, and a healthy run never reaches it.

It is *not* a claim that a present completes within 300 ms. The slowest present this machine
has been observed to complete under deliberate load is 25.4 ms at load 36.9
(`doc/notes-present-settle.md` §5); the ceiling is an order of magnitude above it, and if a
machine ever exceeded it the failure would name 300 ms and the render count rather than
misreport the split.

## 4. Twelve loaded runs

`xvfb-run` on llvmpipe, `--check`, 48 busy loops on 24 cores started deliberately —
`doc/notes-present-settle.md` §5's population is 3 failures in 18 runs at load 36.9 to 55.8,
so the load here is the range that broke the old assertion and past it.

| run | load | renders | the thread held the device for | proven |
|---:|---:|---:|---:|:--|
| 1 | 35.6 | 1 | 157.4 ms | yes |
| 2 | 39.8 | 1 | 114.2 ms | yes |
| 3 | 41.2 | 1 | 104.2 ms | yes |
| 4 | 45.5 | 1 | 104.8 ms | yes |
| 5 | 47.3 | 1 | 130.8 ms | yes |
| 6 | 51.6 | 1 | 77.5 ms | yes |
| 7 | 56.8 | 1 | 78.3 ms | yes |
| 8 | 59.5 | 1 | 278.5 ms | yes |
| 9 | 59.1 | 1 | 178.9 ms | yes |
| 10 | 62.3 | 1 | 154.7 ms | yes |
| 11 | 65.4 | 1 | 71.8 ms | yes |
| 12 | 67.8 | 1 | 400.1 ms | yes |

**12 of 12**, and the whole example passed each time. Two rows are worth reading rather than
counting:

- **Every run took exactly one render**, which is the cost the phase paid before this change
  too. The ceiling is a bound that is never approached.
- **Run 12's render was 400 ms, past the 300 ms ceiling**, and it still proved the ordering —
  because the loop tests the ceiling *after* the render it is inside. A render slower than
  the ceiling cannot defeat the gate; it just makes the loop one render long, which is what
  the loop is for.

**What this population is not.** It is llvmpipe under `Xvfb`, where presents are not
refresh-locked, so it is not a re-run of the failing population — the `AI` user has no X
authority for the owner's display (`CLAUDE.md`). What it does exercise is the *mechanism* of
two of the three recorded failures: a presenting thread that cannot be scheduled, and a
render short enough that the old count could not be reached. The third case — a render
shorter than one refresh — is now structurally unreachable, because the render loop does not
end until the present has returned or the ceiling is hit.

## 5. Verified able to fail

One forced defect, and it is the regression the phase exists for rather than a mutation of
the assertion: **a present that cannot proceed while the device is held elsewhere**, forced
by spinning on `holding` before presenting.

```
a present returned while the device was held elsewhere: false (30 renders in 304.115627ms, …)

thread 'main' panicked at crates/quorra-gpu/examples/present_thread/main.rs:354:5:
the point of the split is presenting during a render: the present returned only after the
render thread had stopped, having completed 30 renders in 304.115627ms
```

Red, by name, with the count and the span in the message — and **terminating**, which is the
half the ceiling exists for. Without a bounded loop the same forced defect is a deadlock: the
loop would wait for a proof from a thread waiting for the loop.

## 6. Recommended next

- Nothing in this file. The round changed no `src/`, drew no page differently, and owes no
  corpus run.
- The one thing it cannot do from here is a run on the owner's display through XWayland,
  where the old assertion's population was gathered. It is cheap — `env -u WAYLAND_DISPLAY
  cargo run --release -p quorra-gpu --example present_thread` — and it is the only place the
  refresh-locked path is real.
