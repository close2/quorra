# One outline lookup instead of two, and the lifetime that turned out not to fight

Round notes for `doc/notes-encode-split.md` §5 item 4 and `doc/notes-recording-shares.md`
§5's first bullet, written 2026-08-16 against this worktree. The debt was recorded by the
`encode.rs` split, priced by the recording-shares round, and taken here.

**The short version.** `fill_solid` asked `ResourceStore::outlines` for an outline
`encode_fill` had already found, once per solid fill on the hottest walk in the tree. The
fix is one field: `SolidFill` carries `&'a StoredOutline` instead of a second lookup of
the `OutlineId` it already holds. **The lifetime the handover said "fights `&mut self`"
does not fight it** — §2 says why, and that half is the more useful finding, because the
same reading had stood unchallenged through two rounds. Measured, one cold-atlas encode
under callgrind:

| page | before, Ir | after, Ir | saved | share of that page's `recording` |
|---|---:|---:|---:|---:|
| **drawing at 58 009 marks** (the caller's page) | 5 719 068 893 | 5 709 728 076 | **9 340 817** | **2.26 %** |
| dense text | 114 863 284 | 114 134 166 | **729 118** | **5.93 %** |
| artwork | 601 326 611 | 601 246 151 | **80 460** | 0.24 % |

The prices `notes-recording-shares.md` §5 published from a *seamed* build — 9 514 587,
711 286 and 81 202 — are reproduced to within 1.9 %, 2.5 % and 0.9 % by a build with no
seams in it at all, which is the strongest thing that can be said about both measurements.

---

## 1. The instrument, and what says it encoded the right pages

`doc/HANDOVER.md`'s "An encode, exactly", unchanged: a `#[cfg(test)]` module inside
`quorra-gpu` that builds a `ResourceStore` and an `AtlasStore` directly — `encode` needs no
adapter — copies the three page shapes out of `examples/encode_threads.rs`, encodes once
outside the collected region to fault in the allocator's arenas, then once inside an
`#[inline(never)] steady_run` against a **fresh** `AtlasStore`, so the collected encode is
the cold-atlas frame every zoom step is. Built with `CARGO_PROFILE_RELEASE_DEBUG=1` into its
own `CARGO_TARGET_DIR`, run under `valgrind --tool=callgrind --collect-atstart=no
--toggle-collect='*steady_run*' --read-inline-info=yes --cache-sim=no`. **The harness was
deleted with the round**, which is `Cargo.toml`'s standing decision about benchmark
harnesses.

**No `#[inline(never)]` seams this time, and that is a deliberate difference from the
recording-shares round.** That round needed to *name* the parts, so it paid a 0.07–0.20 %
distortion for thirty-four call boundaries. This round needs one number — what the whole
encode costs with the second lookup and without it — and a release build with `lto = "fat"`
inlines both lookups into their callers either way, so the delta between two unseamed
builds is the delta the caller gets. §3 uses a seamed build for the call counts only, where
a distortion cannot affect an integer.

**The counter rows, before believing any number above.** Printed by the harness after each
collected encode, and **identical before and after the change on all three pages** — which
is the whole of the claim that the encode is still the same pure function of the command
list. Against `crates/quorra-gpu/tests/archetypes.rs`'s `BASELINE` and
`doc/notes-encode-threads.md` §3:

| page | commands, culled, outlines, atlas keys, clip regions, tiles, residue regions, residue tiles | matches |
|---|---|---|
| dense text | `[4320, 0, 818, 2164, 1, 40, 2, 0]` | `archetypes.rs` `BASELINE`'s dense-text row exactly |
| artwork | `[684, 0, 300, 300, 1, 600, 185, 0]` | the same row exactly |
| drawing at 58 009 | `[58009, 0, 58009, 58003, 0, 6, 0, 0]` | `notes-encode-threads.md` §3 exactly |

(`layer_textures` is the seventh field of `archetypes.rs`'s nine and is dropped here: the
device counts it and an encode does not.)

Wall clocks were not used and are not quotable on this machine — `doc/HANDOVER.md`'s "4.49
ms for an encode the owner clocked at 1.96–2.35". Every number in this note is an
instruction count and load cannot touch one.

## 2. The lifetime, and why it does not fight `&mut self`

The debt was recorded twice — `notes-encode-split.md` §5.4 and `HANDOVER.md`'s small-debts
list — with the same reason for not taking it: *"carrying the borrow needs a lifetime on
`SolidFill` that fights `&mut self`, which is a design question and not a move."*

**It is not a design question.** The encoder does not *own* the resource store; it holds
`resources: &'a ResourceStore`, a shared reference, for the whole frame. Reading that field
out of `&mut self` copies the reference — `&T` is `Copy` — and a borrow taken through it is
a reborrow of `*self.resources`, whose lifetime may be the full `'a` and which keeps no loan
on `self` at all. So

```rust
struct SolidFill<'a> {
    outline: OutlineId,
    stored: &'a crate::resources::StoredOutline,
    // …
}

fn fill_solid(&mut self, fill: &SolidFill<'a>, resolved: &ResolvedClip) -> …
```

compiles with no other change than naming the lifetime the `impl` block already had
(`impl Encoder<'_>` → `impl<'a> Encoder<'a>`). No `Rc`, no index-and-look-up-later, no
splitting `Encoder` into halves, no clone of the segments.

**The evidence that this was always true was in the two functions themselves.** Both
already held a `&StoredOutline` across `&mut self` calls: `encode_fill` holds `stored`
across `resolve_clip`, `hulls.bounds` and `push_rect_instance`, and `fill_solid` holds its
own copy across `prospect_for` and `enqueue` — and `Job::glyph(&stored.segments, …)` puts
that borrow into `Encoder`'s `queue: Vec<Job<'a>>`, which is `'a` and nothing shorter. A
lifetime that reaches a field of the encoder cannot have been fighting the encoder's own
`&mut`.

**The lesson is about the note rather than about the borrow checker.** "Needs a lifetime
that fights `&mut self`" was written once, from a reading, and then quoted forward through
the split round, the recording-shares round and two revisions of `HANDOVER.md`, gaining a
price but never a compile. It cost one edit and one `cargo build` to disprove. The general
form is CLAUDE.md's rule about the specification applied to our own documents: **a claim
that something cannot be done is a claim, and it decays.**

## 3. What the change is, and the call counts that confirm it

`encode_fill` looks the outline up and takes the `UnknownOutline` refusal for the id;
`fill_solid` then did

```rust
let stored = self.resources.outline(fill.outline).ok_or(RenderError::UnknownOutline { … })?;
```

for the same id — a second `hashbrown` probe whose refusal arm was unreachable, because the
first lookup had already established that the id resolves and nothing between them can
release a resource. `SolidFill` now carries the borrow and `fill_solid` reads the field.

**The call counts, from a build with `#[inline(never)]` on `ResourceStore::outline` and
nothing else** — the one place an integer says what a share cannot. Dense text, callers of
`ResourceStore::outline` by `callgrind_annotate --tree=caller`:

| | `encode_fill` edge | `fill_solid` edge |
|---|---:|---:|
| before | 710 795 Ir | 710 795 Ir |
| after | 710 045 Ir | **absent** |

Two edges of *equal* weight before, one after: the two call sites were the same lookup of
the same id, and one of them is gone. (The raw counts callgrind prints for those edges are
doubled by `--read-inline-info=yes`, which attributes an entry to both the outer function
and its inlined frame; the ratio is what the table reads, and both edges carry the same
factor.) The seamed totals move 114 969 674 → 114 163 457, a larger delta than the unseamed
806 217 → 729 118 because an un-inlined call costs more than an inlined one — which is
itself a check that the seam is where it is claimed to be.

## 4. What this does not buy

`notes-recording-shares.md` §4's floor is unmoved and this note does not claim otherwise.
Recording is 7.23 % of the caller's page's encode by instruction count; this is 2.26 % of
that, which is **0.16 % of the encode** and about **1.7 ms of a 128 ms** one at the ratio
§2.1 measured between recording's instructions and its time. Their frame is 185.6 ms and
its floor with the whole of `encode` deleted is 107.0. Nothing here changes what stands
between that page and 120 Hz.

**It is worth taking anyway, and the reason is the dense-text column rather than the
caller's.** 5.93 % of a page of text's recording is the largest *relative* share of any
single item this project has priced and removed, the page shape is the one §6.2's baseline
is stated on, and the cost of taking it is one struct field and one named lifetime. An
optimisation that makes the code shorter is not a trade against clarity, which is the
tension CLAUDE.md's "on the tension between 2 and 4" is about — there was nothing to trade.

## 5. Also closed, and one thing seen

Four comments, all recorded by `doc/notes-encode-split.md` §5 and none of them behaviour:

1. **`push_op`'s two openings** (§5.1) — the first paragraph described `append_op`, which
   sat below it with no doc comment at all, and the two ran together with no blank line.
   Each paragraph is now on the function it describes.
2. **`CULL_MARGIN` cited `Encoder::push_glyph`** (§5.2), which ADR 0054 replaced with
   `parallel`'s `Job::glyph`. `cargo doc -p quorra-gpu --document-private-items` goes **32
   warnings → 31**, and the one that leaves is that link. (The split recorded 37; the tree
   has moved since, and the count is quoted here as measured today rather than carried.)
3. **`culled`'s link definition in the middle of its prose** (§5.3) — rustdoc rendered it
   correctly and a reader of the file did not. Moved to the end of the comment.
4. **`command`'s `#[allow(clippy::only_used_in_recursion)]`** (§5.5) no longer fires, and
   its comment described a state of the walk that has moved on. Deleted;
   `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets` is clean without it.

And **§5.6's duplicated arithmetic was taken, in its own commit and flagged for dropping**:
`coverage_tile` and `visible_tile` held the same ten lines of `shape ∩ clip ∩ target`
rounded out to whole pixels, and `coverage_tile` now asks `visible_tile` for it. The
module comment already stated the invariant this creates — *"the two branches have to
agree, to the pixel, about the tile they produce"* — as something a reader had to check by
holding both in front of them; it is now true by construction. **A sibling round is
changing what a coverage tile is bounded by and `coverage_tile` is at the centre of it**,
so this is a separate commit that can be dropped or re-applied at merge without touching
anything else in this round.

## 6. Verification

- `cargo test --workspace`: **442 passed, 0 failed, 2 ignored** over 52 suites, plus three
  doctests. Checked against `grep -rn '#\[test\]' crates --include=*.rs | wc -l` = **444**,
  which is 442 + 2 ignored exactly — the arithmetic `doc/HANDOVER.md`'s trap asks for, and
  the reason a green run is not evidence on its own.
- `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets`: clean, with
  `Checking quorra-gpu` printed rather than only `Finished`.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`: clean.
- `cargo fmt --all --check`: clean.
- **`tests/archetypes.rs` is the gate that says no pixel moved**, and it is the same gate
  the harness's counter rows report: seven rows of nine counters, each an exact function of
  the scene and the viewport, identical before and after. No gate was added or changed this
  round, so none needed verifying able to fail.

Two shared-target-dir collisions were seen and are not defects here: one `extern location
for quorra_gpu does not exist` and one `found possibly newer version of crate quorra_gpu`,
both in the doctest job, both while a sibling agent was building into
`/home/AI/cargo-target/quorra`. Re-run in isolation, `cargo test --workspace --doc` passes.
`function_knockout`'s `a_point_the_domain_leaves_unpainted_knocks_nothing_out` failed once
in the same run and passed six times in a row afterwards, at base and at change; it is
recorded here rather than dismissed, because a flake nobody wrote down is a defect nobody
looked for.
