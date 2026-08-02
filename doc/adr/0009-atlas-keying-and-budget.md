# ADR 0009 — Atlas keying: exact linear part, quantised phase, reset-not-evict

Status: accepted, 2026-08-02. Landed with M4.

## Context

§6.3 of the brief: key on `(outline, scale bucket, sub-pixel phase)` with the quantum
settable and documented — §4.5's fifth decision, the caller's to make and ours to
expose. Their measurement (ADR 0131 in their tree): 1/16-pixel quantisation reused
5.0× on a dense page with the oracle unmoved; 1/8 contradicted pages; exact keying
never hit.

## Decision

- **Key = (outline id, linear part bit-exact, phase).** The "scale bucket" is the
  composed device linear part `[a b c d]` by f32 bit patterns, not a rounded bucket:
  glyphs at one font size carry the *identical* matrix, so exact bits hit exactly
  where reuse exists, and an animated zoom simply misses — rasterising being the
  correct price of genuinely new geometry. A rounded bucket would draw a glyph at a
  slightly wrong size, which is the plausible-lie class of §5.
- **Phase quantised at `Options::glyph_quantum`** (default 1/16), `None` = off =
  exact-bit phase keying. Quantising moves rendered text by at most half a quantum —
  why the knob is exposed rather than chosen silently. `tests/m45.rs` pins both
  behaviours through the public counter.
- **`Counters::atlas_distinct_keys` counts distinct keys per frame**, never a hit
  rate (§6.3's lesson, verbatim).
- **Tiles above 128 px per side are not cached**: they take the scratch path,
  uncached but pixel-identical (one rasteriser feeds both). The bound keeps one
  zoomed letterform from evicting a page of text.
- **A full atlas resets after the frame instead of evicting piecemeal.** Mid-frame,
  an unfittable tile falls through to scratch (`atlas_pressure`), the frame stays
  correct, and the *next* frame repacks from empty — so packing is a pure function
  of the scene sequence from a reset, and pixels never depend on cache history
  (`tests/m45.rs` proves atlas-starved and atlas-backed frames byte-identical).
- **Budget**: `Options::atlas_budget` (default 8 MiB of R8), sized near-square,
  clamped to the device's texture limit. The texture itself is created on first
  need, never at startup (§7).

## Consequences

- The M4 gate numbers: 5 933 fills → 107 keys → 107 entries on the dense page;
  steady-state 1.0 ms/frame at window scale on RADV (ADR 0008 carries the table).
- The reset strategy trades occasional whole-atlas re-rasterisation (bounded by the
  cold-frame cost, ~2 ms for a dense page) for determinism and simplicity. If
  thrash appears on real corpora, the recorded next step is generation-tagged
  entries with per-frame LRU eviction — a change confined to `atlas.rs`.
