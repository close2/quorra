# ADR 0022 — The readback reads the pixels once, and divides never

Status: accepted, 2026-08-11. From measuring where an offscreen frame's time actually
goes, prompted by the caller's `performance.md`: their corpus gate has quorra at 2.5–3×
their multi-threaded `tiny-skia`, with "a per-frame floor that does not [grow with
pixels]".

## Context

The floor is the readback, and it is not close:

| 1191×1684, RADV, release, fastest of nine | total | encode | upload | execute | readback |
|---|---|---|---|---|---|
| one rectangle | 2.44 ms | 0.003 | 0.009 | 0.033 | **2.05** |
| 500 rectangles | 2.31 ms | 0.009 | 0.007 | 0.055 | **2.01** |
| 5 933 rectangles (a dense page) | 4.94 ms | 0.089 | 0.018 | 0.180 | **3.84** |
| the same page to a `Texture` target | **0.47 ms** | — | — | — | none |

78–84% of an offscreen frame, and a page with one rectangle costs nearly what a dense
page does — which is the definition of a floor. §6.1 of the brief said this about the
library we replace; it was true here too.

Inside it, two things dominated, and neither was the device:

- **`read_buffer` copied the mapped range into a `Vec`** — 8 MB at page size — and the
  conversion then read that copy once and threw it away.
- **`demultiply` ran three integer divisions per pixel**, six million on a page, and
  wrote its output with eight million `push` calls.

## Decision

**Convert straight out of the mapped range.** `map_and_convert` maps, demultiplies from
the view, and unmaps; the intermediate `Vec` is gone. (`read_buffer` stays for the
callers that genuinely want bytes — the timestamp resolve, and the test that reads the
coverage sheet back.)

**Replace the division with the division, evaluated ahead of time.** A 64 KiB table
indexed `[alpha << 8 | channel]`, built by a `const fn` at compile time from
ADR 0005's rule verbatim: `(c·255 + a/2) / a`, clamped to 255. It is not an
approximation and the test says so exhaustively — all 65 536 pairs against the
arithmetic it replaces. A table that agreed with the rule on the pixels a fixture
happens to contain would be curve-fitting; one that agrees on every input there is, is
the rule.

**Write through a slice, not through `push`.** The output is `vec![0; …]` and each
pixel writes its four bytes into a `chunks_exact_mut`. Transparent pixels write nothing
at all — the zeros are already right, and most of a page of text is transparent.

The three shapes of pixel are now: transparent (skip), opaque (four-byte copy), partial
(three lookups). Only the last touches the table.

## What it buys, measured

Same machine, same fixtures, fastest of nine:

| | before | after |
|---|---|---|
| page, one rectangle | 2.44 ms (readback 2.05) | **1.59 ms** (readback 1.44) |
| page, 500 rectangles | 2.31 ms (readback 2.01) | **1.53 ms** (readback 1.39) |
| page, 5 933 rectangles | 4.94 ms (readback 3.84) | **1.65 ms** (readback 1.32) |
| half page, 5 933 rectangles | 1.51 ms (readback 1.16) | **0.58 ms** (readback 0.37) |

**A dense page's offscreen frame is three times faster.** What remains is 0.10 ms of map
wait and ~0.7 ms of conversion for 16 MB of traffic — about 23 GB/s, which is memory
bandwidth on this machine, so the next win here is not another loop but not doing the
copy at all (a `Texture` target, which the caller's window already uses).

Buffer creation was measured and is **free** (0.000 ms for the 8 MB `MAP_READ` buffer),
so it is not pooled; a pool would be code with no number behind it.

`tests/perf_gate.rs` gains a readback gate at ~4× the measured value — wall-clocked
because the span genuinely is, with the before and after numbers in its message so a
failure says which shape it regressed to.

## What this does not change

The rounding rule, the boundary at which the conversion happens, or the bytes any
frame hands back. `tests/m1.rs`'s golden comparisons and the caller's corpus gate are
unmoved — this is the same picture, computed once instead of twice.

## Revisit when

The conversion stops being memory-bound — a wider vector unit would need explicit SIMD,
and that is an `unsafe`-free intrinsics question with its own benchmark. Or when a
caller wants the premultiplied bytes directly, which would let a `Readback` frame skip
the conversion entirely; nobody has asked, and §3 of the brief says straight alpha is
what the boundary hands back.
