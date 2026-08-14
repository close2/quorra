# Round notes — the retained encode survives an atlas it overflows (ADR 0050)

The exact edits for `doc/PLAN.md` and `doc/HANDOVER.md`, written out rather than applied:
two sibling agents are working in parallel and the owner merges those two files by hand.
Everything else in this round is committed.

---

## 1. `doc/HANDOVER.md`

### 1a. Item 2 — **delete the last paragraph**

Under "### 2. A page-sized coverage tile per clipped shape — **not** multi-sheet passes",
the final paragraph reads:

> One more shape belongs to this seam since ADR 0048: a page whose glyph tiles overflow the
> atlas re-encodes on every frame, because the repack that follows bumps the atlas generation
> and invalidates its own retained encode. Magnified text is that shape.

**Remove it.** It is done, and it was not the seam it was filed under — the coverage
tiling seam has nothing to do with the glyph atlas. ADR 0050 also narrows the claim
itself: overflow alone never did this. What did was a page that fits the atlas *by bytes*
and not *by shelves*, which was one configuration in seventeen swept, and which repacked
on every frame for ever.

### 1b. "Recorded and deliberately not taken" — **add one bullet**

Append to that list:

> - the atlas has no recency, so two pages that alternate and do not fit beside each other
>   still repack once per frame — the same cost that shape had before ADR 0050, now
>   *visible* as `Counters::atlas_repacked` true on every frame rather than inferred. A
>   single page too large for its atlas is stable (0050); genuine thrash is what recency
>   answers, and ADR 0024 has been waiting for its measurement since.

### 1c. "Instruments" — **add one bullet** after the "An encode, exactly" entry

> - **Whether the atlas settles**: `examples/retained.rs`'s second section — twelve retained
>   frames of a page whose tiles overflow a stated atlas budget, printed as a string of `E`
>   (encoded) and `.` (replayed). A **property, not a clock**, so it reads the same at load
>   average 90 as on an idle machine. `E...........` is a settled atlas; `EEEEEEEEEEEE` was
>   the pathology ADR 0050 removed. The section asserts that its page is still inside the
>   band — a fixture that drifts out of it goes on passing, because a page that never
>   overflows replays trivially.

### 1d. Traps — **add one**

> **A cache's "would this help?" test must be asked in the units the cache allocates in.**
> ADR 0024 gated the atlas repack on `bytes requested <= bytes available` and the packer
> allocates *shelves*: a page at 63 % of the atlas by area did not fit it by packing, so the
> repack fired, changed nothing, and fired again on the next frame — for ever, invalidating
> its own retained encode each time (ADR 0050). Sixteen of seventeen swept configurations
> settled after one encode and looked like proof the design was fine. **Sweep the parameter,
> and read the sequence rather than the first two frames.**

---

## 2. `doc/PLAN.md`

### 2a. "### The numbers that stand" — **add one row** after the retained-encode row

The table currently has:

> | the same frame, unchanged, replayed rather than encoded | **0.174 ms** against 1.107 | `examples/retained.rs`, headless RADV, ADR 0048 |

Add beneath it:

> | — and now also when the page's glyph tiles overflow the atlas | 1 encode per page, not 1 per frame | `examples/retained.rs`'s overflow section, ADR 0050 |

### 2b. "### What is still open" — **amend the second bullet**

The bullet on the caller's adoption round ends with "`RetainedScene` is an API they must
take up rather than merely receive. `HANDOVER.md` item 1." Append:

> Two `Counters` fields land with ADR 0050 — `atlas_working_set_bytes` and
> `atlas_repacked` — and one `DeviceError` variant, `ResourceIdsExhausted`; both are
> additive, and `doc/api-change-retained-atlas.md` is what the bump owes them.

### 2c. Wherever `Counters` is enumerated in Part 1/2

`PLAN.md` describes the counter set in its §8/instrumentation prose. Add the two new
fields there in the same register as the others:

> `atlas_working_set_bytes` — what holding **all** of a frame's distinct glyph keys would
> cost, which is the number `Options::atlas_budget` is compared against, and the only one
> that tells "the atlas is too small for this page" apart from "the atlas is holding
> another page". `atlas_repacked` — whether the atlas was thrown away and re-packed after
> this frame, which is the one event that makes a retained encode stale. A page that
> settles reports it true on at most one frame; true on frame after frame is thrash, and
> the instrument exists so that state has a name (ADR 0050).

### 2d. Integration notes — **one line** wherever the resource id space is described

> Resource identifiers are never reused and the space is a `u32`: the upload that would
> exhaust it is refused with `DeviceError::ResourceIdsExhausted` rather than wrapping. The
> counter wrapped until ADR 0050 audited ADR 0048's key; a reissued id would have made a
> retained encode draw a resource it never named, with every generation counter agreeing
> that nothing had moved.

---

## 3. A debt this round did not take

`crates/quorra-gpu/tests/retained_frame.rs` is now about 1 100 lines. It was ~940 before
this round and it is one responsibility — "a replayed frame is the frame that was
encoded, and every way it could stop being one", stated in its module comment and
organised in lettered sections — but it is well past the ~500-line smell. Splitting it
wants a `tests/common/mod.rs` for `blob`, `place`, `text_page`, `artwork_page`,
`device_with` and `retained_frame`, which touches nothing else in this round and would
have made the diff harder to review. Named here rather than done.
