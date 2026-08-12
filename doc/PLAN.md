# Plan

The brief is `RENDER_LIBRARY.md`; this file is the design in its current state of
belief, the order of work, and the state of both. Bare section numbers (§) are the
brief's; "clause" numbers are ISO 32000-2's.

The file has two parts. **Part 1 says how the library will work** — the architecture as
currently believed, with each piece naming the measurement that could overturn it,
because §11 is explicit that the design should turn on measurements rather than on
anyone's taste, and most of those measurements have not been taken yet. **Part 2 says
the steps that get there** — nine milestones, each with its work, its exit gates, and
the question it settles. When a Part 1 hypothesis is confirmed or overturned, the
decision becomes an ADR and this file is corrected in the same commit; a plan that
disagrees with the tree is worse than no plan.

## Where we are

**Shelves stay near one width, so the sheet stays near square** (2026-08-12, ADR 0034):
the item ADR 0021 left — "the sheet's *height* … is a packer question with its own
measurement" — measured at last. `issue16287.pdf` at 4× committed a **6 026 × 2 406 sheet
for 6.93 M texels of tiles, 48 % used**: the first shelf grew to the full width and the
next three held one tile each, and a sheet is a rectangle, so those three paid 3 900 empty
columns apiece. A shelf may now grow to `√(2 × placed area)` rather than to the packing
width — the side of the square a shelf packing of that area fills, computed from what has
been placed and floored at the widest tile. The same page commits **2 224 × 3 293, 95 %
used**; a 29-tile page goes 50 % → 72 %. It buys bytes, which is the currency refusals are
counted in, and postpones the 16 384 ceiling — and **it buys no measured time**: the page
whose sheet falls by 18 M texels of per-frame upload reads 344 ms against 331, three runs
each. No page of the corpus changes verdict or reason at either scale. The full answer is
to place tiles by size rather than in encounter order, which needs positions assigned after
the walk instead of during it, and that is a two-pass encode.

**A group can be one stage of §11.4.6** (2026-08-12, ADR 0033): the other half of the
caller's §14.2, and the reason three of their four refused pages needed more than
ADR 0032. §11.3.7.2 makes a group's shape *the union of the shapes of the objects it
contains*, so a knockout element that is itself a group has a shape no fill can state —
and `SceneBuilder::group` carried no compositing operator at all. `GroupSpec::compose`
sits beside `blend` now: `DestOut` composites the group as the erase `P' = (1 − f) × P`
weighted by the group's own alpha, which is its shape when the caller draws the shape half
opaque, and `Plus` as the deposit. `Src` is refused on a group — that is what `knockout`
states — as are a staged group carrying a blend mode (§11.3.5 composites it there) and a
non-isolated one (§11.4.4 seeds its buffer with its own backdrop, so the alpha the erase
reads as a shape would carry the backdrop's too). Measured on two overlapping half-opaque
wedges: **0.77 of 255 against the clause's line for the pair, 114.95 for the ordinary
composite**. The cost is a new field on a public struct — thirty-two literals here and the
caller's adapter besides — and `Plus`'s saturation obligation extending to groups.

**The staged pair belongs where the clause puts it** (2026-08-12, ADR 0032): ADR 0025 gave
the caller §11.4.6's two operators and then refused them inside a knockout group, on the
reading that a group already staging the clause per element would apply it twice. It does
not — §11.4.6 weights each element by *its own* source shape, so a staged element replaces
the group's erase for itself — and the refusal made the operators unusable for the case
they were added for: `Command::Shaped` appears **only** as a direct element of a knockout
group, because that is the only clause that uses shape and opacity apart. The position is
accepted now, `StagedComposeReason::InsideKnockoutGroup` is deleted rather than left
unreachable (a breaking change their own test anticipates), and the blend-mode refusal
stands. Measured on a soft-masked wedge inside a knockout group, worst premultiplied
deviation from the clause's line: **0.77 of 255 for the pair against 108.29 for the same
element written as one mark**. What is left is §14.2's second ask — a compositing operator
on a *group* mark, which three of their four refused pages need because a nested group's
shape is the union of its elements'.

**The timestamp instrument is made with the device, not with the frame** (2026-08-12,
ADR 0031): the caller's §9 reports a first frame costing 12-18 ms more than its
successors, flat across target sizes and not pipeline compilation. Reproduced here at
11.2-11.9 ms against 0.9, and timing the inside of `Device::render` found **2.43 ms of it
making the frame's `QuerySet` and two sixteen-byte buffers** — an instrument, charged to
the frame a person waits for. The driver pools them after the first: 2.35-3.34 ms for the
first on five fresh devices, 0.018-0.036 for the second. So one query lives with the
device now and each frame borrows it, giving it back only if its read succeeded — a
failed read can leave the map buffer mapped, and the frame after would get a validation
error instead of a number. **What is left of the first frame is about 6 ms and is not ours
to warm**: it is inside `run_frame` and scales with the target, so it is page-sized
textures and the driver's first touch of a heap that size, and a warm-up thread cannot
allocate them before the viewport exists. That makes the remainder an API question — a
size hint, or `Device::warm_for` — and it is the caller's contract, so it is written down
rather than taken.

**A clip chain is one region, so its links intersect** (2026-08-12, ADR 0030): the caller
asked, in their `QUORRA_FEEDBACK.md` §18, what rule composes a chain here and whether
§10.7.4 changes it. It was a product — so one clip restated *n* times raised its
antialiased boundary to the *n*-th power, and their witness page painted an edge at 0.041
where the geometry is 0.827. ISO 32000-2 §8.5.4 settles it without needing the
measurement: the graphics state holds **one** clipping path, set to *the intersection of*
the current path and the new one, and §11.3.7.2's NOTE 1 makes the fractions the
rasterisation of that one hard-edged region. Nothing in the standard composes two
fractional coverages; the place a genuine product of shape values lives is §11.5's soft
mask, a different mechanism with its own clause. So the links take `min`, which is
idempotent the way intersecting a region with itself is, and never below the exact area.
Where the clip meets the *mark* the product stays, as a stated choice: those two
boundaries are usually unrelated, which is the case a product estimates and `min` does
not. On the corpus all three readings measure identically — 915/37/5, no page's numbers
moving — so this is a clause decision that the measurement declines to arbitrate, and it
is written down that way.

**A shelf the CPU lane did not write was the tail of the wide layout** (2026-08-12, the
caller's `QUORRA_FEEDBACK.md` §20.4.1): they measured the GPU lane against the corpus for
the first time and found `transparency_group.pdf` differing by 31.7 of 255 in its worst
tile, asking whether sixteen samples could explain it. They cannot — and it was not the
lane at all. `ScratchPacker::finish` restrides the sheet's rows from the packing width
down to the width the shelves reached (ADR 0021), which leaves the old layout's bytes
behind each moved row; growing the buffer to the sheet's extent afterwards **keeps
whatever of that tail falls inside it**. Nothing while every shelf holds CPU tiles, since
each writes its own rows — and 136 410 texels of another shape's coverage the moment a
shelf below them belongs to the GPU lane, which reserves rows it fills on the device.
Dumping the sheet before the winding pass is what said so. Cutting the tail before the
grow takes **five pages at scale 1 and six at scale 4** from *differs* to *agrees* on
their corpus, with the CPU lane's verdicts unmoved.

**A cache is worth what it is used for, not what it will accept** (2026-08-12,
ADR 0029): ADR 0028 asked the atlas whether it *would hold* a tile. The better question
is what it would *do* with one, and the scene can answer it — `crate::census` counts a
page's solid fills by outline, scale and rule before the walk, and a shape the page places
**once** is one the cache would rasterise, upload and read a single time. The corpus
median is 1.33 placements per distinct outline, so that is the ordinary page and not a
pathology: **its first frame went from 40.3 ms to 14.7** at 200-pixel tiles, 2.5-3× at
every size measured, with no later frame slower and a page of shared glyphs untouched at
0.4 ms. The count is deliberately loose — it ignores the sub-pixel phase, so "placed once"
is a fact and "reused" is a guess in the direction of the cache. Two things were built,
measured and **removed**: a one-frame memory of what the cache declined (it would have
made the third frame of a static page a different picture from the first) and a `has_room`
question to go with `admits` (with the census in place, the atlas stops filling with tiles
nobody reuses, and asking moved nothing on four page shapes or the corpus). What is left
is that the census cannot see the phase, and that a caller who redraws one page for many
frames would still rather have the atlas.

**The winding target spans the lane's own tiles, and the lane is the one the atlas
leaves** (2026-08-12, ADR 0028): ADR 0027's band bounded the target's height and left its
width the *sheet's*, which both lanes share — so thirty shapes of 1 200 pixels still
refused at 194 MB. The target is a **pane** now, a rectangle over this lane's tiles with
its extent fixed from the budget *before* the panes are cut, which is what makes 16 MiB a
bound rather than a hope: those thirty shapes need 16.7 MB. And the band shipped with a
defect nobody could see — `fs_winding` tested a fragment against its tile in *sheet*
coordinates while the fragment's own coordinate had already been moved by the band, so
**every band after the first drew nothing at all** and the frame reported `Ok`. That is
principle 6's failure, it is the third appearance of this one agreement, and the two
end-to-end tests that now cover it draw panes offset in both axes. It was not
hypothetical: on the caller's corpus under `Coverage::Gpu`, `issue9418.pdf` differed from
the oracle by **191 of 255 in its worst tile** and now agrees, and `issue1905.pdf` was
refused for *400 681 916 bytes against a 268 435 456 budget* and now draws — the two real
pages this ADR's halves each fix, found by attributing every verdict the corpus moved.

Then re-deriving ADR 0027's crossover — as that ADR instructed — moved it off tile area
altogether. What the CPU lane has is the **atlas**: a tile it admits is rasterised once
for every placement there will ever be (0.4 ms a page against 16 on the device), and a
tile it refuses is rasterised again every frame (35.5 ms against 15.0). The same
52 000-texel tile is in both rows, so no area separates them. `GPU_LANE_MIN_AREA` is
deleted; the lane is what the atlas leaves, with ADR 0026's triangle floor beneath it.
**A page of 1 200 px tiles: 32.1 ms → 9.6. A page of 200 px tiles the atlas has no room
for: 35.5 → 15.0. A page of cached glyphs under `Coverage::Gpu`: 16 ms → 0.4.** What is
left is that the criterion cannot see how *often* an outline is placed — on a page whose
outlines are each used once, the atlas is admitted to and buys nothing, and the cold
frame pays 38-44 ms where the device would have taken 13-20.

**The winding target holds a band, and the lane's crossover is measured** (2026-08-12,
ADR 0027): the target is scratch — accumulated, resolved into the R8 sheet, dead — so it
never needed to hold the whole sheet at eight bytes a texel, which is what refused sixty
shapes of 500 pixels at 359 MB. It holds one band now, aimed at 16 MiB, with both shader
stages subtracting the band's origin — the same agreement that, in another form, once drew
the right glyph in the wrong place. And banding made both lanes measurable on the same
pages, which showed ADR 0026's criterion wrong in magnitude: at 200 and 500 pixels the GPU
lane was two to four times *slower*, because its winding traffic follows the sheet and its
per-tile overheads only amortise once a tile is big. The crossover is now a measured
constant — half a megapixel, bracketed by 325 000 texels where the processor wins twice
over and 637 000 where the device wins nearly four times. **40 shapes of 700 px: 97 ms →
26. 24 of 900 px: 113 → 20.** What is left is that a band still spans the *sheet's* width,
which both lanes share, so a page of 1 200-pixel shapes still refuses where the CPU lane
draws it.

**The coverage lane is chosen by what each lane would cost** (2026-08-12, ADR 0026): the
corpus profile put `Coverage::Gpu` against the largest real page and it **refused the
frame** — 922 MB, of which **821 MB was vertices**. The GPU lane costs an outline's
triangles *per placement, whatever the tile's size*: a nine-pixel glyph is 12.4 KB of
triangles against ~150 bytes of coverage, eighty times more, on the page shape most
documents are made of. So the lane is now chosen per command by comparing the two costs
the encoder already knows — `width × height` bytes against
`triangle_count × 3 × stride` — and `Coverage::Gpu` means "where it pays" rather than
"everywhere". A page of ordinary text stops paying 62.5 MB and twelve milliseconds to be
drawn *worse*; the corpus's largest page draws instead of refusing; and 200 shapes of
200 px — the shape ADR 0016 was built for — go from **121 ms of encode to 3**. What is
left is the winding texture, sized from the whole sheet at eight bytes a texel, which
still refuses a page of very large shapes: it is scratch, resolved and then dead, so
banding it is the next piece of work on this lane and it wants its own tests, because it
touches the pass that once drew the wrong glyph.

**Our fixtures were invented, and the corpus says what pages are actually shaped like**
(2026-08-12, `doc/corpus-profile.md` and `tests/archetypes.rs`): every performance
fixture here was reasoned out rather than measured, and a measurement over the caller's
995 first pages found three things that matter. **Not one page emits a
`Command::Rect`** — our flagship gate draws 5 933 of them, so it prices a lane no
document takes. **Glyph reuse is 1.33 placements per distinct outline at the median**,
not the 55 the brief's one dense page suggests, so a cold atlas is the normal state of a
page turn and every fixture that measured a warm one was flattering itself. And **the
median page is twelve commands**: the mass is trivial pages and everything interesting is
in the tail — one page holds 66 309 commands, another 15 004 clips.

Six archetypes now stand for those shapes, generated here from the counts and rendered on
a **fresh device** each time. What gates them is `Counters`, not clocks: every field is an
exact function of the scene and the viewport, so the baseline compares by **equality on
any machine**, needs no threshold and cannot flake — `tiles` would have caught the atlas
cliff, `bytes_uploaded` the sheet at maximum width, `layer_textures` the pair-per-plan
frame. Each recorded number is explained beside it, because a baseline nobody can account
for is one nobody can defend. What is checked in is counts and nothing else: no document,
no display list, no reference to that project — delete it from the machine and the file
still compiles and means the same thing.

**Feedback §17 is answered with no change at all — two rasters of one page already
work** (2026-08-11): §11.4.7 puts a colour space under the whole page, and a
four-component one is three plus one, so the page is interpreted twice and the two
rasters are put together by a per-pixel conversion afterwards. §17 offered to close
itself if two `Target::Readback` calls against one device were simply supported and
cheap. They are, and `tests/two_rasters.rs` is the evidence rather than the assertion:
both rasters come back whole, resources are device-scoped so the second display list
references the first's uploads, neither pass changes what the other draws — and **the
second interpretation pays no geometry at all**, because the atlas key is
`(outline, linear part, phase, rule)` and colour is not in it. Two caveats stated rather
than buried: a frame whose tiles overflow the atlas can leave the next pass cold
(ADR 0024 narrowed when that happens), and each pass pays its own readback, which is
irreducible when both rasters are wanted.

**Feedback §14 is answered — §11.4.6's two stages can be asked for by name**
(2026-08-11, ADR 0025): `Compose::Src` reads an element's shape off the alpha it is drawn
with, which is right where they are the same quantity and wrong where §11.6.4.2's shape
and §11.6.4.3's opacity differ — a nested group, or an element under a soft mask.
`Compose::DestOut` and `Compose::Plus` let a caller write `P' = (1 − f) × P + S` in one
mark each. The change is small because **the pipelines already existed**: the knockout
lane is those two operators, `(Zero, OneMinusSrcAlpha)` through `fs_shape` and
`(One, One)`, and this is a vocabulary that can ask for one alone. Measured on a diagonal
edge under a half-opaque object, the pair is within **0.77 of 255** of the clause's line
and source-over is **114.95** away from it. Two positions refuse a staged mark because
they already stage the clause — under a blend mode, and inside a knockout group — and one
thing cannot be refused: `Plus` alone saturates, so the pairing is the caller's
obligation and is documented as the first such item in this vocabulary.

**What the atlas admits is a share of it, and what it keys on now includes the fill
rule** (2026-08-11, ADR 0024 — the atlas-policy question the culling work left recorded):
`MAX_GLYPH_DIM` decided caching by a *dimension* while what it protected was a *budget*,
and the mismatch was a cliff — past about 10× every visible letterform left the atlas and
was rasterised again on every frame. Held at a magnification on `examples/zoom`, encode
went **13.6 → 0.65 ms at 12×, 19.4 → 0.50 ms at 20×**, and the zoom sweep's worst frame
35.8 → 16.2 ms (both runs on a loaded machine, so read them against each other). Two more
things came with it. A pressure reset now happens **only when the frame's own working set
would then fit** — otherwise it throws away the part that fits and hits, which cost 6.0 ms
against 4.8 at 100×. And **the fill rule is part of the glyph key**, which is a
correctness fix: the same outline under §8.5.3.3's two rules is two pictures wherever a
subpath nests, and the cache handed the first to the second — invisible until now only
because the dimension bound kept such shapes out, and the suite's own fill-rule test uses
a shape twelve pixels past the old cap. `Counters::tiles` counts at last, after reading
zero for three milestones. Putting the rule in the key cost 0.19 ms of encode on the
dense page — a key hashed twice per glyph, twelve thousand times a frame — so the two
hot maps got a deterministic multiply-xor-rotate hasher of our own (`keyhash.rs`): the
same page's encode is **0.932 ms before this work, 1.125 with the rule under `SipHash`,
0.746 with the rule under ours**, so the correctness fix is paid for twice over. The
caller's 957-page gate is unmoved, differ list identical page for page.

**Feedback §13 is answered — `encode` says what it spent its time on, and the phases say
which clock they are on** (2026-08-11, ADR 0023): their trace put `encode` at 45% of a
page turn and 3.86 µs a command, and said the thing that decides what to build — whether
that is flattening, binding, buffer writes or recording *"is invisible from here"*. It
subdivides now into **geometry / staging / recording**, through `Timings::phases`, behind
`Options::instrument_encode` — a switch rather than always-on because the parts interleave
per command, so the measurement costs a clock read per seam: ~0.2 ms on a page of 5 933
commands, three times the whole encode of a page of rectangles. A measurement that moves
what it measures by 300% is not an instrument. The `"target acquire"` and `"present"`
phases are always on (two clock reads), and `Timings::host_total()` plus the rustdoc say
which numbers a host may subtract from its own wall clock — `execute` is the adapter's,
and mixing the two is what made their `elsewhere` row a quantity they stopped believing.

**And the first reading answers the question**: on 3 675 curved fills at reading size,
1191×1684 on RADV, encode is **60–70% geometry whenever the atlas is cold** (2.549 ms =
1.533 geometry + 0.155 staging + 0.861 recording for 107 distinct outlines; 8.919 ms =
6.229 + 0.795 + 1.895 for 3 675 distinct) and **entirely recording when it is warm**
(0.995 ms and 1.758 ms, all of it hash lookups and instance writes). The cold
all-distinct row is 2.4 µs a command, the same order as the 3.86 they fitted. So the next
move on that phase is the rasteriser or the atlas being cold — not more cores, which is
also why **quorra spawns no threads**: asked, the caller said take a pool rather than make
one (their `rayon` is already sized to the machine, their confined worker is
single-threaded because of a seccomp-killed `/sys` read in `glibc`'s arena sizing, and
`viewer-core`'s rule 4 is "no threads the core was not handed"). If parallelism is ever
wanted here it arrives as a pool the host supplies, in the shape `create_instance`
already established.

**The readback reads the pixels once, and divides never** (2026-08-11, ADR 0022): the
caller's `performance.md` has quorra at 2.5–3× their multi-threaded `tiny-skia` on an
offscreen corpus, "dominated by a per-frame floor that does not grow with pixels".
Measured here, the floor is **the readback and almost nothing else**: at 1191×1684 a
frame with one rectangle cost 2.44 ms of which 2.05 was readback, a dense page 4.94 ms
of which 3.84 — while the same page to a `Texture` target cost 0.47 ms end to end. Two
things inside it, neither of them the device: the mapped range was copied into a `Vec`
the conversion then read once and discarded (8 MB), and the conversion ran three integer
divisions per pixel — six million on a page — writing its output through eight million
`push` calls. Now it converts straight out of the mapped range, through a 64 KiB table
built at compile time from ADR 0005's rule verbatim (and held to it over all 65 536
pairs, because a table that agreed only on a fixture's pixels would be curve-fitting),
writing through a slice with transparent pixels writing nothing at all. **A dense page's
offscreen frame is three times faster: 4.94 → 1.65 ms**, and the remainder is memory
bandwidth. `tests/perf_gate.rs` has a readback gate now, with both numbers in its
failure message.

**What the three rounds are worth on the caller's own instrument**, their 957-page
corpus gate run back to back on a quiet machine, their CPU backend in the same process
as the control (1.97 s before, 1.95 s after — so the machine did not move): quorra's
total goes **5.22 s → 4.79 s** and the **median page 2.17× → 1.90×** the CPU backend.
§6.2 calls a third of that CPU time a success and a tenth a clear win, so this is
progress rather than arrival. The next candidate is measured and not taken: at half-page
size the readback is still 63% of a frame (0.366 ms of 0.58 ms), it is memory-bandwidth
bound in one thread, and the baseline it is losing to is *multi-threaded* `tiny-skia` —
splitting the conversion across threads is the largest remaining offscreen win, and it
puts threads in a library that deliberately has none, so it is the owner's call rather
than a quiet addition.

**A frame's layer textures are a depth, not a count** (2026-08-11, ADR 0020): every plan
renders into a ping-pong pair of full-target textures, and the compositor created one
pair per plan — all of them at once, and priced that way. A plan is a group *or* an
element with a non-Normal blend mode, and a pair is 16.05 MB at 1191×1684, so the
default 256 MiB budget held **sixteen plans**: eight groups each holding one blended
rectangle — ordinary Illustrator artwork — was `FrameBudgetExceeded { needed: 272767584 }`.
The refusal was honest and its model was wrong. A child's pair is dead the moment its
parent's composite has read it, so siblings share: the peak is the tree's **depth**, the
budget prices that, and `Counters::layer_textures` reports what was actually allocated.
Sixty-four such groups now draw in six textures. What still costs is nesting, which the
builder bounds at 16 — and that is the case bbox-bounded layers would answer, recorded
in the ADR rather than taken now.

**Feedback §16 is answered — a group's buffer can begin as a copy of what is under it**
(2026-08-11, ADR 0019): `GroupSpec` gains `isolated`, Table 145's `/I` and the one entry
the vocabulary was missing. ISO 32000-2 §11.4.4's non-isolated group composites its
elements onto the group's own backdrop and then removes that backdrop's contribution —
by dividing by Table 140's group alpha, which a premultiplied raster does not hold, and
whose NOTE 4 advises a second set of accumulators. **It is not needed**: the quantity
divided out is multiplied straight back in when the result is composited with the same
backdrop under Normal, leaving `result = (1 − w) × B + w × E(B)`. Transcribed from the
clause in `tests/non_isolated_groups.rs` and checked over 200 000 configurations, that
is exact — **5.6 × 10⁻¹⁶** — while the same construction under a non-Normal group blend
is wrong by 0.91 of full scale and applied to an *isolated* group by 0.76, which is why
the builder refuses the three cases rather than approximating them
(`SceneError::NonIsolatedGroupUnsupported`, naming which condition failed). The
implementation is a seeded layer (one scissored blit, no new allocation and no change to
the frame budget) plus a branch in `composite.wgsl` on a flag that fits in its existing
padding — no second pipeline, so no startup cost. Measured at 1191×1684 on RADV: a seed
is 0.11 ms of device time, and the isolated path is unchanged within the run-to-run
spread (0.384 → 0.386 ms). **Held against the caller's 956-page gate, before and after,
three documents move from refused to agreeing and nothing else moves**: `bug1755507`,
`issue13520` and `issue18032` — Illustrator and InDesign artwork that used to be counted
as agreeing while both backends substituted the same wrong initial backdrop — now agree
with their CPU backend's independent reading of §11.4.4, and the `differ` list is
identical page for page (910/35/11/18 → 913/35/8/18). The fourth document §16 named is
still refused, and now says why it really is: a page composited in a four-component
blending space, which is their §17.

**Feedback §12 is answered — a host can name the backend set** (2026-08-07, ADR 0017):
the caller's project owner ran their viewer on a Windows machine with Intel graphics
and it crashed inside the **Vulkan driver**, and nothing here could ask for the DX12
one. `create_instance()` took `Backends::all()`; `Options::adapter` filters on the
*device's* name, which one GPU reports once per backend that can drive it, so it cannot
express "this GPU, through DX12"; and with no filter wgpu's hub order puts Vulkan
first. **`create_instance_with(backends)`** is the whole answer — `create_instance()`
is unchanged and is now that function with `Backends::all()` — plus
`Device::adapter_names_on`, which lists what a *given* instance can see so a host
cannot offer a choice its own constructors could not honour. The environment is
deliberately **not** read: the caller asked that the `WGPU_BACKEND` question be decided
rather than defaulted, and the decision is that the argument is the only route, with
`Backends::from_env()` one line away in a host that wants it *under* its own command
line rather than over it. ADR 0014 §3 is superseded in part — it declined a backend
knob as a *startup* optimisation, that measurement stands, and it never weighed a
driver that crashes. No machine here runs Windows, so the mechanism is exercised with
the backends this one has (`tests/backend_choice.rs`).

**A device no longer outlives its own thread** (2026-08-07, ADR 0018), found while
writing that test and older than it: the warm-up
thread was detached, so a device dropped before it was warm — which is what probing
adapters does, and what a host does when it constructs a device, dislikes a limit and
falls back — could reach `exit()` with a thread still inside the driver, and Mesa's own
atexit teardown then crashed the process. `tests/device_lifecycle.rs` reproduced it at
**13 of 15 runs**, SIGSEGV or SIGABRT in `quorra-warm-up`, always *after* every test in
the binary had passed, which is why nothing had ever caught it. `Device` now holds the
`JoinHandle` and joins it on drop: ~5 ms added to a device dropped before it is warm,
nothing at all to one dropped after, and nothing new blocks during construction.

**A frame is charged for what it allocates, and a CPU-lane frame allocates no winding
texture** (2026-08-05): the caller's 974-document corpus gate went from one refusal to
six the moment its lock moved onto the per-frame coverage lane, and all five new ones
said the same thing — `frame needs N scene-derived bytes, over the stated budget of
268435456`, with `N` between 280 MB and 1.2 GB, for pages that had drawn one revision
earlier. The budget had not moved; what a frame is charged had. ADR 0016's lane prices
its winding texture at encode from the extent of the **scratch** sheet, which both
lanes share: `Sheet::width` and `height` are filled in on every frame that packs a
tile, including one the GPU lane never ran, so a CPU-lane frame was charged
`max_target_size × rows × 8` bytes of `rgba16float` for a texture `upload_scratch`
creates only `if !winding.is_empty()`. A pre-flight that refuses what the allocation
would have drawn is principle 6's failure with the sign flipped — a page that draws,
refused — and it cost five real documents. The condition now lives in
`Sheet::device_bytes`, which is the one place both the charge and the allocation read,
so the two cannot drift apart again; `tests/coverage_lanes.rs` holds the pair, because
the point is that the charge still stands where the texture is real.

**The sheet is as wide as it is used, and is charged for what it is** (2026-08-11,
ADR 0021 — the item the paragraph this replaces called "the next thing to do here"):
`ScratchPacker::finish` committed a texture at the *packing* width, which is the device's
maximum dimension because a narrow one refuses real pages (feedback §3). On this machine
that is 16 384 texels a row, so one 180-pixel tile allocated and moved 2.95 MB to carry
32 KB — and the GPU lane, whose winding target takes its extent from the same sheet at
eight bytes a texel, paid 23.6 MB for it. Every tile sits left of the widest shelf
cursor, so narrowing there moves nothing and keeps the capacity that §3 bought. Measured
at 1191×1684 on RADV, best of nine: **a GPU-lane frame with eight blobs went from
10.54 MB and 3.00 ms to 0.46 MB and 1.96 ms** — a third of the frame, and the shape of
the caller's median page. `Timings::execute` is unmoved, which is what says the cost was
allocation and bandwidth rather than shading. The sheet's own bytes are now charged too:
shelf gaps are allocated, and pricing only the tiles made the largest scene-derived
allocation of a page of path work the one number nobody counted. On the caller's
957-page gate no page changed verdict. What is left is the sheet's *height* — gaps
between shelves that narrowing cannot reach — and that is a packer question with its own
measurement.

**Coverage can come from the GPU, and a caller can now ask for it** (2026-08-05,
ADR 0016): Evan Wallace's method — one triangle per outline segment fanned from an
anchor, accumulated with additive blending so that what lands at a sample *is*
§8.5.3.3's winding number, plus a Loop-Blinn control triangle per curve whose
orientation alone decides whether the bulge is added or bitten out. Cubics become
quadratics **once, at upload**, so no step in the lane knows the device scale: that is
the answer to what the cull uncovered, where a zoom gesture makes every cached tile
cold on every frame. Signed accumulation rather than his parity trick, because parity
is even-odd only and §8.5.3.3.2's non-zero is PDF's default; samples in an
`rgba16float` texel's channels rather than packed into a byte's bits, so sample count
costs time and not memory; and the sample grid stated in our own code rather than taken
from the driver, so ADR 0006's cross-adapter identity survives.

`Options::coverage` sets the default and `Device::set_coverage` changes it **per
frame**, because the crossover is a magnification and a session in which a person zooms
crosses it. Measured on the dense page at 1191×1684, RADV, wall per frame: the CPU lane
holds 0.44–1.05 ms from 1× to 8×, then costs 4.4 ms at 12× and 12.1 ms at 20×; the GPU
lane costs 11.3 ms at 1× and 1.6–2.1 ms everywhere from 8× up. **The crossover is
between 8× and 12× and it is a cliff, not a curve** — it is `MAX_GLYPH_DIM`, where
glyphs stop entering the atlas — so the threshold a caller wants is
`128 ÷ its text height` rather than a fitted constant, which for 10–12 point body text
is a magnification of about ten. Two
findings the measurement forced: **the winding texture is kept between frames** (ADR
0012's deferred pool, now with the measurement it asked for — per-frame allocation and
zero-init was 10.7 ms of a 15 ms frame), and the conversion tolerance is relative to
the outline rather than absolute.

Where the lanes differ is stated rather than hoped: on a **straight-edged** shape they
agree exactly where no edge crosses a pixel, and differ by at most the sample grid's
eighth of a pixel where one does (32 of 255, measured 12); on a curved one they differ
by up to 96 anywhere in the frame — **because the CPU lane flattens to a quarter pixel
and the GPU lane does not flatten at all**, so a pixel the CPU lane calls wholly empty
can still be clipped by the quadratic the device draws. (Corrected 2026-08-12: the
exact-agreement claim was stated of every shape, and its test used a curved fixture that
had been taking the CPU lane in *both* devices — ADR 0028's criterion is what put the
GPU lane under it and the claim was wrong within a frame.)
Tightening `FLATTEN_TOLERANCE` to 0.004 takes the worst difference to zero pixels over
20, which identifies the flattening as the whole of it. The GPU lane is the more
accurate about the shape and the less accurate about the pixel. Still on the CPU lane:
commands under a non-rectangular clip, which fall back and share the one sheet.

**A frame costs what it shows, not what the page holds** (2026-08-04): the caller
zoomed, and found that a frame got *more* expensive the further in a person went —
the encoder flattened all 5 933 commands of a page for a window displaying 24 of
them. ADR 0012's recorded lever is now taken (ADR 0015): a command whose device
bounds, inflated by two pixels for the glyph lane's quantised phase and the coverage
lanes' `floor`/`ceil`, miss `clip ∩ target` is rejected before its geometry is built,
and `Counters::commands_culled` reports how many. A **zoom gesture** — 1× to 20×
over 24 frames, no cached tile helping any of them — went from a worst frame of
**156 ms of encode to 9.3 ms**; a page with nothing off the target pays about 6–10%
more encode for the test, which is written down rather than hidden. Two things it
deliberately does not do: a group is not culled as a unit, and damage still is not a
cull. **What the cull uncovered is the next question**: at 20× the residual 6.8 ms is
30 glyph tiles of ~290 px rasterised again on every frame, because past
`MAX_GLYPH_DIM` a glyph never enters the atlas — a probe raising that constant takes
the same frame to 0.25 ms. That is an atlas *policy* question (what a large tile may
cost against the budget, whether a gesture's key churn can be kept from thrashing
it), not the GPU-coverage question of ADR 0008, and it wants its own ADR and its own
measurement. `examples/zoom.rs` is the harness; `tests/cull.rs` and a
deterministic count gate in `tests/perf_gate.rs` hold the behaviour.

**Feedback §8 is answered — bring-up is measured per step, and its largest step can
start before there is a window** (2026-08-04): the caller's owner decided that page
one goes to the graphics device, which put our bring-up on their time-to-first-page
— 45.1 ms of a 144.6 ms launch, 31% of it — and their §8 asked for the two things
that makes necessary. Both landed (ADR 0014). **`StartupTimings` is five numbers
instead of three**: `instance_creation`, `surface_creation`, `adapter_selection`,
`device_creation`, `pipeline_compilation`, plus `blocking_total()` over the four the
constructor actually waits for. The field it replaces, `adapter_enumeration`, named
one step and measured three, so nothing that moved inside it could be attributed —
which is now the tree's own worked example of the instrumentation rule about counts
versus rates. **`startup::create_instance()` plus `Device::headless_with_instance`
and `Device::for_surface_with_instance`** let a host build the instance on a thread
at `main`'s first line, in parallel with reading its document: measured headless on
RADV here, instance creation is ~80% of what bring-up blocks for (22.9–29.8 ms of
29.4–36.1 ms), and hoisting it leaves **5.1–9.2 ms** to pay after the window exists.
`instance_creation` is then `None` rather than zero — the step happened, on someone
else's clock, and a struct for attribution may not claim otherwise. The backend knob
§8.3 talks a host out of was **not** added *for speed*, and ADR 0014 records the
caller's measurement that says there is none to win; it arrived later and for another
reason entirely, in ADR 0017 above. `examples/startup.rs` is the measurement, one
configuration per process by design.

**Feedback §7 is answered — a refusal costs the surface nothing** (2026-08-04): the
viewer reproduced a permanent wedge (every acquire a 1-second `Timeout`, only a
resize recovering) whose cause was a budget refusal *after* the swapchain acquire —
the dropped, never-presented texture leaves an acquire semaphore no submission waits
on, and enough of those exhaust the swapchain. Three changes, one hardening: the
compositor's internal textures are now priced straight after encode, before the
target is bound, so a refused frame acquires nothing (`Options::max_frame_bytes`'s
"before anything is allocated" is now true of the acquire too, and
`tests/m1.rs::frame_budget_refusal_precedes_target_binding` pins the ordering
through the headless `NoSurface`-vs-`FrameBudgetExceeded` distinction); `Timeout`
now sets `needs_reconfigure` exactly as `Outdated` does, so the wedge is at worst
one bad frame; `Device::invalidate_surface()` is the host's explicit lever
(`NoSurface` on a headless device — a caller bug refused by name); and a frame that
fails *after* its texture was acquired (`run_frame`, the one remaining post-acquire
early return) now invalidates the surface on its way out, bounding that path at one
lost frame too. `FrameBudgetExceeded`'s message no longer claims "instance data"
when what overflowed was internal textures — it names scene-derived bytes, which is
what the shared budget prices. Awaiting the viewer's re-run of their one-drag
reproduction on a real surface; every headless-provable piece is gated.

**The corpus feedback is answered** (2026-08-03): the viewer measured the swapped
backend against its 974-document corpus and wrote up what came back
(`pdf-viewer/doc/QUORRA_FEEDBACK.md`); everything actionable landed the same day.
On this side: the frame's scratch sheet now spans the full device dimension —
capacity, not commitment, since bytes stay budget-charged per tile — and its
exhaustion is its own `RenderError::ScratchExhausted` naming the real limit,
replacing a refusal whose arithmetic contradicted itself (six real pages refused
under a 2048-wide sheet now draw; the corpus's one pathological page still
refuses, truthfully). On the adapter's side: §10.7.4's degenerate fills draw
through the viewer's shared split; the `Arc`-pinned caches gained LRU eviction to
half the resource budget (533 refusals at 4× scale became zero);
anisotropically-transformed strokes outline in path space instead of taking one
scalar width (three corpus pages moved from "differs in shape" to agreement); and
an empty mesh raster draws nothing, as both sibling backends and pdf.js's own fix
for the defective document do. Corpus after: **910 of 957 agree, 46 differ (29 at
the antialiasing floor), 1 refused** — from 900/50/7.

**M9 is done — the swap happened** (2026-08-03): `render-quorra` in the caller's
tree implements their `Rasterizer` over this library and passes their cross-backend
and real-page suites at the Vello backend's own thresholds; the viewer's window now
presents through quorra's surface tier (no readback, −205 lines of host machinery),
verified under Xvfb with real key presses on ISO 32000-2 itself. The full record —
the integration refinements it forced here, the two adapter defects the caller's
instruments caught, and the one owner-level follow-up (the corpus sweep) — is in
the M9 section. The library's home is https://github.com/close2/quorra, and the
viewer consumes it from there.

**M8 is done** (2026-08-02): the rest of the performance contract, decided by
measurement. **Damage is honoured exactly** (ADR 0012): a valid `Viewport::damage`
against a retained `Texture` target renders the frame internally with every pass
scissored to the damage bounding box — sound because every pass is pixel-local —
and patches exactly the listed rectangles onto the target with REPLACE blits over
`LoadOp::Load`, so nothing outside the list is touched and nothing can
double-composite. `Surface`/`Readback` targets redraw fully and say so in a
`Report` naming the kind; malformed rects refuse by index; a list that clamps to
nothing touches no pixel. Measured (dense page, 1191×1684, one 12×18 caret rect):
RADV execute **0.136 → 0.047 ms**, llvmpipe **4.2 → 1.6 ms/frame**; encode still
walks the whole scene (~0.1 ms) — command culling and a bbox-sized root texture are
ADR 0012's recorded levers. **The pipeline-cache question closed against the
`unsafe` exception** (ADR 0013): construction is 19.9 ms adapter + 11.9 ms device,
neither cacheable; the warm set compiles in 9.4 ms on a thread `Device::headless`
never waits for — no user-visible number to win, so principle 3's bar is not met
and `#![forbid(unsafe_code)]` stands. **No texture pool** (and so no shrink
policy): internal-texture creation sits inside the measured 0.37 ms patched frame;
pooling waits for a measurement that says otherwise. **§11.5's verdict: hold the
scenes.** A dense-page `Scene` retains **570 KB** (the figure-laden page 571 KB),
so the dozen-resident-pages target costs ≈ 6.8 MB — noise next to a single
1191×1684 target's 8 MB, and `Scene::cost().retained_bytes` keeps it checkable.
Gates in `tests/m8.rs`.

**M7 is done** (2026-08-02): the rare-case lanes (ADR 0011). An image (§8.9.5), a
ramp shading (§8.7.4.5.2/.3) or a pre-rasterised mesh draws as **one uniform-driven
quad** inside the ordinary passes — no third instance stream for primitives the
brief's §0 calls rare. Both shaders map device pixels back through the inverse
transform: an axis-preserving image gets the rectangle lane's analytic edge coverage,
an oblique one paints centres-inside-the-unit-square (hard edges, stated); nearest
filtering is `textureLoad` and adapter-invariant, linear is the hardware sampler with
its variance stated and shape-gated. Ramps pre-sample on the CPU to 256 RGBA8 texels
indexed at `round(t·255)`, so the sweep arithmetic is ours — the axial projection and
the radial quadratic run in shading space and survive shears. Unextended sweep
regions paint *nothing* (§8.7.4.5.2), and therefore knock nothing out; the shading
question deferred from M2 closed on geometry-on-the-paint (integration note 9). GPU
textures realise lazily on first draw and die with `release`. Every lane rides
clip/mask/blend/knockout machinery unchanged. Measured (release, texture target,
1191×1684): the dense 5 933-rect page carrying 8 images, 6 shadings and a mesh runs
**0.63 ms/frame on RADV** (0.31 ms without the figures) and 4.2 ms on llvmpipe —
both far inside the 5.9 ms CPU baseline. Gates in `tests/m7.rs`: exact nearest
blocks, §8.9.5 orientation, clause-derived axial/radial bytes, extend-off
transparency, mesh anchoring, coverage agreement with the solid lane, unknown-id
refusals by name, and the cross-adapter ±2 bound on the deterministic paths. With
M7, **the refusal list is empty**: every scene command draws.

**M6 is done** (2026-08-02): clause 11, natively (ADR 0010). Groups are layers
composited once through an in-shader implementation of §11.3.6 with all sixteen
§11.3.5 blend functions (REPLACE target state: the arithmetic is ours, not the blend
unit's — which is what ADR 0006 demanded); knockout and `Compose::Src` run as
erase/add pass pairs strictly per element, and the diagonal-edge fixture holds the
result to §11.4.6's own formula; soft masks render through the same machinery and
reduce on the device via a mirror of the caller's `SoftMask::value` — **all 256 bytes
of both rules agree exactly**, non-black backdrop and non-identity transfer included
(`tests/m6.rs`). An element with a non-Normal blend becomes an implicit one-element
group, so §11.3.5 has one implementation. Flat frames still draw straight into the
target — the M1 fast path is untouched — and layered frames price their internal
textures against the frame budget before creating any. The scene API grew `mask()`
and the mask parameter (integration note 8: mask comes last, a recorded divergence
from the brief's illustrative order). At M6's close only images remained refused;
M7 closed that too.

**M4 and M5 are done** (2026-08-02), on one shared foundation: a CPU coverage
rasteriser of our own (`raster.rs`, ADR 0008) — exact trapezoid accumulation, both
fill rules, cubic flattening at a stated tolerance, stroke expansion with §8.4.3's
caps, joins and miter limit — feeding two lanes. The **glyph lane** caches R8 tiles
in a persistent atlas keyed `(outline, linear part bit-exact, quantised phase)` with
the 1/16 quantum settable and off-able (`Options::glyph_quantum`, ADR 0009); the
**path lane** rasterises uncached coverage into a per-frame scratch image — large
fills, strokes, oblique rectangles, and the non-rectangular clip residues that M3
deferred, multiplied in per link. Both draw as instanced quads with `textureLoad`
(no sampler, no filtering) and the analytic clip rectangle in the shader. Scene order
is preserved across lanes by batch breaks, never reordering.

The §11.2 census **remains open** — the path-lane design was chosen as the smallest
correct one the census can overturn, and ADR 0008 names the compute-shader lever if
it does. What the M4/M5 record already shows (release, RADV, texture target,
1191×1684): a dense page of 5 933 *curved* glyph fills runs **1.0 ms/frame** steady
state — warm encode 0.73 ms (inside the caller's 1.1–1.6 ms budget), execute 58 µs —
with a 1.9 ms cold frame to rasterise its 107 tiles; the atlas-hostile page (§11.3:
fresh phases everywhere) pays ~7 ms on its cold frame and is indistinguishable warm,
so the atlas's failure mode is bounded by CPU rasterisation throughput and its win is
the caller's 5.0× reuse made real (5 933 fills → 107 keys → 107 entries, pinned in
`tests/m2.rs`). Cross-lane gates (`tests/m45.rs`): the analytic rectangle and the
rasterised rectangle agree within one premultiplied step; atlas-backed and
atlas-starved frames are byte-identical; the cross-adapter bound holds at ±2 for the
new lanes because the coverage bytes themselves are CPU-made and adapter-invariant.

**M3 is done** (2026-08-02): rectangular clips, analytically. Clip chains resolve at
encode time to one device-space rectangle each — memoised across shared prefixes, the
region (never the identifier) counted in `Counters::clip_distinct_regions` — and the
rectangle lane applies a clip by intersection on the CPU, so a rectangular clip costs
the device nothing at all (ADR 0007; the brief's shader-side comparison arrives with
the glyph lane, which cannot pre-intersect). The M3 fixtures pin the two numbers the
milestone exists for: 303 identical clip states collapse to **1** region on a full
page at 1191×1684, and the 3 608-chain worst page resolves within the ordinary
budgets, every distinct region counted. Empty-admits-nothing versus absent-clip is
tested as two different answers; a non-rectangular clip is refused by name until M5's
residue masks. `SceneBuilder::rect` gained its clip parameter; `axis_aligned_rect` in
`quorra-scene` recognises rectangle outlines once, at upload.

**M2 is done** (2026-08-02): the scene vocabulary of §2.3 minus what later milestones
own — `fill`, `stroke`, `rect`, `clip` chains and bounded `group`s, every input
validated loudly at the builder (§4.7) — plus the device's resource registry:
`upload_outline`/`upload_image`/`upload_ramp`/`upload_mesh`/`release`, each upload
validated and priced against a stated budget (`Options::max_resource_bytes`,
discoverable through `Device::limits`). `Scene::cost()` now reports commands, clips,
group depth and retained bytes, computed once at `finish`. A command whose lane does
not exist yet is refused by name (`RenderError::NotYetDrawable` says which command,
what kind, and which milestone delivers it) — drawn or refused, no third state. The
scene boundary is fuzzed from this milestone on: a deterministic structured fuzzer
(`tests/fuzz_scene.rs`) drives hostile builder/upload/render sequences on every push;
coverage-guided `cargo-fuzz` needs a nightly toolchain and stays outside the pinned
tree, a recorded choice, not an omission. The drawable half of §2.2's round trip —
107 outlines actually painting 5 933 fills — is M4/M5's proof, and its test lands
there. Also landed since the M1 record: the surface path is proven against a real
window (`examples/window_smoke.rs` under Xvfb, pixels verified via `xwd`; in CI on
every push).

**M1 is done** (2026-08-02): a device (headless and surface-attached), all three
targets of §2.4, the analytic rectangle lane, timestamped and truthful frames, the
startup split of §7 — plus the harness (goldens against a CPU reference, byte-equality
and bounded-difference gates, refusal tests, a perf gate with measured thresholds).
Two of §11's questions now have measured answers; the M1 record below has the numbers.
One deviation from the original M1 scope is recorded rather than silent: the pipeline
cache blob moved wholly to M8, because wgpu 30 exposes it only through an `unsafe`
constructor and this tree is `#![forbid(unsafe_code)]` — weighing that exception is
M8's ADR.

Every number quoted in Part 1 below was measured in the caller's tree against the
Vello-based backend this library replaces; the **M1 record** is the first set of
numbers measured in *this* tree.

### The M1 record (fastest of ten, release, this machine, 2026-08-02)

`examples/floor.rs`, 5 933 rectangles (a dense page's command count; rectangles stand
in for glyphs until M4) at 1191×1684, phases from timestamp queries:

| adapter | encode | execute | readback | whole frame, Readback | whole frame, Texture |
|---|---|---|---|---|---|
| RADV (890M) | 0.035 ms | **0.048 ms** | 4.10 ms | 4.58 ms | **0.22 ms** |
| llvmpipe | 0.035 ms | 2.59 ms | 4.26 ms | 8.07 ms | 2.98 ms |

Startup on RADV: adapter enumeration 22.8 ms, device creation 15.4 ms, warm pipeline
compilation 2.65 ms (off the critical path; `headless` returns before it). That first
figure is the pre-split one — instance creation plus adapter selection, and mostly the
driver loader; ADR 0014 re-attributes it and the "Where we are" entry above carries
the current numbers.

Three honest caveats: rectangles are not glyphs (no atlas, no clip states); the encode
translates our own scene rather than the caller's display list; and the caller's
5.9 ms/12.1 ms baselines were measured on their harness, not ours. The *structural*
findings survive the caveats, and they are the two answers below.

**§11.1 answered: the readback is essentially the whole fixed cost.** On the real GPU,
device execution for a dense page at window scale is 48 µs; the readback is 4.1 ms —
roughly 90% of the offscreen frame — and a texture-target frame costs 0.22 ms total.
The brief's ranking (surface and texture paths first) is confirmed, emphatically: tier
2/3 hosts skip what is by far the largest item. Also reproduced on our design: 5 933
fills execute in 48 µs against 7 µs for one rectangle — the per-command device cost is
noise compared to the per-byte costs.

**§11.4 answered: cross-adapter byte identity is not achievable through the
fixed-function raster path** (ADR 0006). The float→unorm8 store conversion rounds
differently on RADV and llvmpipe — measured on a single opaque rectangle, before any
blending. Same-adapter byte identity holds and is gated exactly; cross-adapter output
is gated to a stated bound (±1 unorm step per blend stage, ≤ ±2 after straight-alpha
conversion on the golden). The design lever: identity returns if the compositor owns
final quantisation in shader code, which M6 must weigh anyway for the fifteen
non-Normal blend modes — the caller's CI reliance on identity against the measured
price of shader-side quantisation.

---

# Part 1 — How the library will work

## 1.1 The shape of it: a sorter, five lanes, one compositor

The one-sentence brief calls for a renderer whose fast paths assume what a document
actually contains. The architecture that follows from it is a **sorting renderer**: at
frame time, every command in the scene is classified into one of five lanes by what it
*is*, each lane maps to the cheapest device primitive that draws it exactly, and all
five lanes draw into a compositor that implements clause 11 natively. Vello's design
premise — every fill is a general curve fill, handled by one uniform tile-binned
pipeline — is exactly the premise §6.1 measured and found backwards for this workload;
ours is the opposite premise, held to the same standard of measurement.

| lane | what lands in it | device primitive |
|---|---|---|
| **glyph** | a fill of an uploaded outline whose device-space size fits the atlas — §1.1's dominant case, 5 933 of one dense page's commands over 107 distinct outlines | one instanced quad sampling the R8 coverage atlas (§6.3) |
| **rectangle** | axis-aligned rectangles under axis-preserving transforms: rules, backgrounds, underlines, table cells — and most clips | exact analytic coverage in the fragment shader; no tiling, no binning, no edge list (§6.4) |
| **path** | everything else: large fills, arbitrary transforms, strokes — the rare case, by assumption until §11.2's census makes it a number | the general coverage path, whose design M5 chooses *after* the census (§1.6 below) |
| **image** | decoded RGBA8 with the filter decision already resolved upstream (§4.5, integration note 1) | a textured quad |
| **mesh** | the caller's pre-rasterised mesh, shared between its backends on purpose (integration note 5) | drawn as the raster it already is; never re-triangulated |

Two properties of the sorter matter more than the lanes themselves:

- **Classification happens at encode time, per frame — never at scene-build time.**
  Which lane a command takes is a device-space question: the same glyph outline is a
  quad at 100% zoom and a general path at 6400%, when its device size outgrows what an
  atlas entry can hold. Putting the sorter in `render` is what keeps the `Scene`
  viewport-free (§2.3), which the brief calls the most important property in the
  document. The budget for the whole encode is the number the current backend already
  achieves: **1.1–1.6 ms, flat in resolution** (§6.1). Ours may not regress it, because
  it is a function of the command list and not of the pixels, and that flatness is
  structural, not accidental.
- **The sort is a pure function of the command list and the viewport.** Same scene,
  same viewport → same lanes, same batches, same draw order. Determinism (§4.6) is
  designed in here, not tested in later.

**Overturned by:** §11.2. If the corpus census shows the path lane is not rare — that a
substantial share of real commands miss the glyph and rectangle lanes — then the path
lane's design gets the engineering attention this table currently gives the atlas, and
this section is rewritten with the number in it.

## 1.2 A frame, from call to pixels

`Device::render(scene, viewport, target)` runs five phases, each bracketed by
timestamp queries where the adapter offers them, so that `Timings` reports what §8
requires and §6.1 could not get: the split between encode, upload, execute and
readback, measured rather than inferred.

1. **Classify and count.** One CPU walk over the commands: sort into lanes, resolve
   each clip chain to its rectangle-and-residue form (§1.4), discover the group and
   mask jobs and their dependency order, and **count everything** — instances per
   lane, layer targets, mask targets, bytes to upload. Nothing has been allocated yet.
2. **Allocate and upload.** Every buffer is sized from phase 1's counts and checked
   against the stated budget before creation. This is §5's first preference — count,
   then allocate — and it is why the failure mode this library exists to eliminate
   cannot occur: there is no fixed-size table for a scene to overflow on the device,
   so a page is drawn or the *allocation* fails with an `Err` naming the limit. A count
   of zero is legitimate everywhere (a blank scene is a legitimate scene, §5) and never
   becomes a zero-length buffer handed to `wgpu`.
3. **Execute.** Passes in dependency order: atlas fills for glyphs not yet resident;
   mask groups rendered and reduced (§1.3); then each layer bottom-up, its commands
   drawn as batched instanced draws in scene order. A batch is a maximal run of
   commands in the same lane with `BlendMode::Normal` and the same clip state; a
   non-Normal blend, a group boundary or a clip change cuts it. Batch cuts are a pure
   function of the list, and `Counters` reports how many there were, because a page
   that cuts often is a page this design serves badly and we want to learn that from a
   counter rather than from a regression.
4. **Resolve.** `Surface` and `Texture` targets composite the finished page and are
   done — no readback, which per §6.1 deletes the largest single cost in the current
   backend's frame. `Readback` copies out, maps, and converts premultiplied to
   straight alpha once at the boundary (§3).
5. **Account.** `Timings` from the query results (and saying so when a wall clock had
   to stand in — a number whose provenance is ambiguous cannot gate anything),
   `Counters`, `Report`s, and a `Frame` whose every claim about itself is true.

**Overturned by:** §11.1, answered in M1. If the readback is nearly all of the fixed
cost, tiers 2 and 3 are the whole performance story, and the effort this plan spends on
per-pixel work in phases 3–4 is re-ranked accordingly.

## 1.3 Clause 11 is the compositor, not an effect

This is the part an SVG-shaped model cannot be patched into, so it is the part designed
first and compromised never.

**Groups are layers, painted once.** A `Group` becomes an offscreen premultiplied
target; its children draw into it; the finished layer is composited onto its parent
exactly once, under the group's constant alpha and blend mode (§4.4; clause 11.4.1,
11.4.5). What the layer is *initialised to* is `GroupSpec::isolated`: transparent for
clause 11.4.5's isolated group, which is the default and what the brief's §4.4 promised
would be the only case, or a copy of the backdrop for clause 11.4.4's non-isolated one,
whose composite is then an interpolation rather than §11.3.6 (ADR 0019, from the
caller's feedback §16). Nesting is bounded at 16 (§1.1), so the layer stack is
countable in phase 1 like everything else. The page itself renders onto transparency, always, because clause
11.4.7 makes the page group isolated and compositing onto the medium is the caller's
job (§3).

**Sixteen blend modes, ours.** `Normal` is hardware fixed-function blending and is the
fast path that keeps batches long. The other fifteen — the twelve separable and the
four non-separable, written from clause 11.3.5's `Lum`, `ClipColor`, `SetLum` and
`SetSat` — need the backdrop as an input, which `wgpu` has no framebuffer-fetch for, so
a non-Normal draw costs a batch cut and a backdrop read (a copy of the affected bounds
into a sampled texture, or a ping-pong of the layer — M6 measures which, on scenes
where it matters). Each WGSL blend function carries its clause number in a comment, and
the implementation is deliberately not shared with the caller's CPU backend: a shared
one would make the cross-backend comparison compare an implementation with itself
(§4.3).

**Shape is its own channel.** The knockout rule (§4.1; clause 11.4.6) replaces *a
coverage-fraction* of the accumulated group with the element composited against the
group's initial backdrop — `lerp(accumulated, element, coverage)`, per pixel. The
design consequence is a rule that binds every lane: **coverage is a first-class value
in the fragment shader at the moment of composite** — computed analytically in the
rectangle lane, sampled from the atlas in the glyph lane, produced by the coverage
machinery in the path lane — and is never irreversibly folded into premultiplied alpha
before the compose decision is applied. Two candidate mechanisms, decided at M6 with
the diagonal-edge fixture as judge: dual-source blending where the adapter offers it,
and a per-element compose pass where it does not. Knockout groups are rare; per-element
passes inside one are affordable if that is what correctness costs.

**Soft masks are rendered groups, reduced on the device.** A mask group is built
through the same `SceneBuilder` as everything else and rendered through the same layer
machinery at device resolution; a single reduction pass then produces the R8 mask —
`Alpha` (clause 11.5.2) takes the group's alpha, `Luminosity` (clause 11.5.3)
composites the group onto a fully opaque backdrop of the mask's colour *first* and
takes the luminosity of the *result* (the order is the clause's, and getting it
backwards produces a plausible picture), then the optional 256-entry `/TR` table is
applied by lookup. No readback, no round trip — the current backend's per-mask-per-frame
CPU round trip is the thing §4.2 exists to delete. The reduction arithmetic mirrors the
caller's `SoftMask::value` in the same 8-bit integer domain, because that function is
the shared definition of what the pixels mean and our shader is a second implementation
that must agree with it **to the byte** — the conformance test over all 256 mask values
ships with the shader (M6), not after it.

**Open, and flagged now:** the layer targets' precision. Premultiplied internally is
settled (§3); whether a layer is 8-bit or wider is not, and it is entangled with
byte-agreement obligations that live in 8-bit space. M6 decides it with the blend and
mask conformance tests in hand, and writes the ADR.

## 1.4 Clips are mostly rectangles, and the design says so

A clip chain is an intersection (§4.7). Phase 1 resolves each chain once into two
parts: the intersection of its axis-aligned rectangular links — which is itself a
single rectangle, four floats and a comparison in the fragment shader, or a scissor
when it bounds the whole batch — and the non-rectangular residue, which becomes an R8
clip mask through the path lane's coverage machinery.

The caller's numbers say the residue is the exception: its page 6 states one clipping
rectangle 303 times and its display list already collapses them to one identifier
(§1.1), and §6.4 is blunt that a rectangular clip must never become a mask texture.
Where a residue mask is built, it is cached **keyed by the resolved region under the
current viewport, never by an identifier** — the caller's clip-mask cache once answered
all 303 lookups a page made and built 303 identical page-wide masks because the key was
a name (ADR 0132 lesson, restated in CLAUDE.md) — and `Counters` reports the count of
distinct regions, not a hit rate. An empty clip admits nothing, which is a different
thing from an absent clip, and both have tests.

## 1.5 Memory that grows

The rule is principle 6's, the mechanism is §1.2's phase discipline, and the posture is
worth stating as design rather than leaving implicit in the phases:

- **Per frame:** count, then allocate. No working buffer is sized by a constant.
- **Across frames:** pools persist on the device and grow geometrically when a frame's
  counts exceed them; they never shrink mid-frame, and shrinking at all is a deliberate
  policy decision for M8, not an emergent behaviour.
- **Every allocation derived from scene content** — instance buffers, layer targets,
  mask targets, the atlas — is checked against a stated budget before creation, and
  exceeding it is a typed `Err` naming the limit and the number that hit it, so the
  caller can fall back to its CPU backend, which its window already knows how to do
  (§5).
- **Before the frame:** `Scene::cost()` against `Device::limits` gives the caller the
  same arithmetic we will do, so a refusal can happen before a frame is attempted at
  all — §5's second preference, satisfied in addition to the first, not instead of it.

## 1.6 The path lane: designed after the census, and here is the shortlist

The one lane whose design is deliberately not chosen yet, because §11.2 asks the
question and the honest answer is a measurement we do not have: **how many of a real
corpus's commands miss the glyph and rectangle lanes?** The candidates, so that M5
chooses among named options rather than improvising:

1. **Tile-binned compute, à la Vello but with counted allocation.** Known to scale to
   the hard case; brings the machinery §6.1 measured as overkill for the common case —
   defensible only if the census says the hard case is common enough.
2. **CPU flattening at encode into device-space geometry, GPU coverage accumulation.**
   Flattening tolerance is a device-space question and phase 1 is device-space, so this
   fits the frame anatomy; costs CPU time that scales with the lane's population, which
   is exactly why the census comes first.
3. **Stencil-then-cover with multisample.** The classical fallback; its coverage
   quantisation (an MSAA sample count's worth of levels) risks the oracle's bound where
   analytic coverage would not, so it must clear the oracle before it clears anything
   else.

Strokes take the path lane after expansion to fill outlines at encode time — the caller
has already resolved device widths, dashing and degenerate subpaths (§4.5), so
expansion is caps, joins and miters and nothing else. Whether hairline strokes deserve
their own primitive is a question the census's stroke population answers, not one this
plan decides.

## 1.7 Determinism, stated as a design posture

§4.6 requires byte equality for same scene, same viewport, same adapter — and the
caller's CI currently *relies* on RADV and lavapipe agreeing byte-for-byte across
adapters, which is §11's question 4 and is not assumed here.

Within one adapter, determinism is arranged rather than hoped for: the sort, the
batches and the draw order are pure functions of the list (§1.1); blending happens in
draw order, which the GPU guarantees per draw call sequence; no accumulation whose
result depends on scheduling order (atomics races, workgroup timing) is permitted in
any pass that touches pixels; nothing reads a clock or a random source.

Across adapters, the hypothesis was that simple fragment arithmetic would preserve
the RADV/lavapipe identity the current backend enjoys. **M1 measured it and the
hypothesis failed** — not in the shader arithmetic, which is deterministic, but in the
fixed-function float→unorm8 store conversion, whose rounding is the driver's
(ADR 0006). Same-adapter identity holds and is gated exactly; cross-adapter output is
gated to a stated bound; and the decision about restoring identity by owning the final
quantisation in shader code belongs to M6's compositor ADR, where its cost can be
measured against the caller's CI reliance on it.

## 1.8 Startup: the device returns before it is warm

§7's sequence, as this library will implement it:

1. `Device::headless` / `for_surface` enumerates the adapter and creates the device —
   `pollster` over wgpu's two awaits, on whatever thread called it, which may be a
   background thread and need not be (§2.1).
2. It returns as soon as the device exists. **No pipeline compilation is on the
   critical path of construction.** The warm set — glyph quads, rectangle fills, the
   composite — compiles immediately but asynchronously; a render that arrives before a
   pipeline it needs waits for exactly that pipeline, and `is_warm` answers whoever
   wants to hand over from the CPU backend only when frames will be full-speed.
3. Everything else — shadings, meshes, the fifteen non-Normal blends, mask reduction —
   compiles on first use, so a page of plain text never pays for machinery it does not
   touch.
4. `Options::pipeline_cache` takes a blob from a previous launch; a rejected blob is a
   `Report`, never a silent recompile, because a silently recompiled cache is a startup
   regression nobody can attribute (§7). Whether the backend supports the cache at all
   is the driver's answer through wgpu's door (ADR 0002), and we report which answer we
   got.
5. Startup cost is reported split three ways — adapter enumeration, device creation,
   pipeline compilation — and CI gates the numbers from the first commit that produces
   them.

## 1.9 The scene, and the resources it references

`quorra-scene` is pure data and cannot reach a device by construction (ADR 0001).
A `Scene` is the flat command list plus its side tables — the clip chains, the group
tree, the mask definitions — behind one `Arc`: `Send + Sync` by a compile-time
assertion, cheap to clone, buildable on the caller's interpreter thread while the GPU
is still initialising. Outlines, images, ramps and meshes live on the device, uploaded
once and referenced by `u32` handles (§2.2) — the caller keys uploads by `Arc::as_ptr`
identity, so one dense page's 107 outlines are uploaded once and referenced 5 933
times, and a zoom re-uploads nothing. `Scene::cost()` is computed at `finish` time, so
asking it costs nothing per frame. §11's question 5 — what a scene costs to hold,
against a target of a dozen resident pages — gets its number in M2 and its verdict in
M8.

## 1.10 The five questions, where each is answered, and what turns on each

| §11 question | answered | what turns on the answer |
|---|---|---|
| 1. How much of the fixed cost is the readback? | **Answered at M1**: ~90% of an offscreen dense-page frame on RADV (4.1 ms of 4.6; execute is 48 µs). See the M1 record. | tiers 2–3 are confirmed as the headline; per-pixel work is second-order for tier 2/3 hosts |
| 2. Does the glyph path want tiles at all? | M5, corpus census before design | which of §1.6's three candidates the path lane becomes — and whether §1.1's premise survives |
| 3. What does the atlas cost on a page it cannot help? | M4 | whether the atlas is unconditional, adaptive, or off by default on low-reuse pages |
| 4. Is byte-identical output across adapters achievable? | **Answered at M1**: no, for the fixed-function path (ADR 0006); same-adapter identity holds, cross-adapter is bounded and gated | M6's compositor ADR decides whether shader-owned quantisation buys identity back; the caller's CI model needs the conversation now |
| 5. What does a `Scene` cost to hold? | M2 number, M8 verdict | whether the dozen-resident-pages roadmap item needs a compact encoding or gets it for free |

---

# Part 2 — The steps

## How the order was chosen

Not by intuition, and not from the top of the brief. §6.1 measured the current
Vello-based backend and the result reversed the obvious plan: **between 55% and 92% of
an offscreen frame is paid before any of the page is drawn**, scene encoding is flat in
resolution at 1.1–1.6 ms, and 5 933 glyph fills cost about what one rectangle costs.
The brief's own ranking follows from those numbers, and this plan follows the ranking:

> the surface and texture target paths first, because they delete the largest single
> item; then whatever makes the per-pixel floor cheaper for a target that is mostly
> untouched; then the atlas and the rectangle path; then the retained scene; then
> damage.

Two consequences worth stating, because they are easy to get backwards:

- **The first milestone is a measurement, not a feature.** §11's first question cannot
  be answered with a wall clock, and the answer decides whether the atlas is a headline
  or a second-order effect. It needs timestamp queries, which means it needs a device,
  which is why M1 is a device and a rectangle and nothing else.
- **Correctness work is not deferred to the end.** §4 is the reason this library exists
  at all; what is deferred is *breadth*. The knockout group's diagonal edge, the
  sixteen blend modes and a full page at a real window size are the three scenes that
  will find bugs on day one (§10), so each lands with the milestone that first makes it
  possible rather than in a conformance push afterwards.

## M1 — A device, a rectangle, and the measurement that settles §11.1

**Deliverable:** `Device::headless`, `Device::for_surface`, all three targets of §2.4,
one analytically-covered axis-aligned rectangle, `Timings` with real timestamp queries,
`Counters`, `Report`, and a `Frame` that tells the truth. Nothing else — no path, no
glyph, no group. A rectangle is the primitive that needs no tiling, no binning and no
edge list, so it isolates the per-pixel floor from everything else.

**The work, in order:**

1. `Options`, `DeviceError`, `RenderError` — the error variants name what failed, from
   the first commit.
2. Adapter enumeration and device creation, timed separately; `description`, `limits`.
3. The lazy-pipeline scaffolding of §1.8 — built now while there are two pipelines, not
   retrofitted at M6 when there are ten — and `is_warm`.
4. The rectangle fill pipeline: exact analytic coverage in the fragment shader.
5. The three targets, including the readback path with its one straight-alpha
   conversion at the boundary, `#[must_use]` and named for what it costs (§8).
6. Timestamp queries around every phase of §1.2, with the wall-clock fallback that
   *says it is one*; `Timings`, `Counters`, `Frame::reports`.
7. The measurement: execute versus readback, per target kind, at 1×, 2× and 4× of a
   window-scale target — §6.1's table, re-taken with the instrument it lacked.
8. The harness: headless golden renders to PNG, the byte-equality gate (same scene,
   same viewport, same adapter, repeated renders), the RADV-versus-lavapipe
   cross-adapter gate, and CI perf gates for the startup split and the frame numbers.

**Done when:** §11.1 has a number per target kind and resolution; startup has its
three-way split gated in CI; a blank scene renders to `Ok` on all three targets; the
cross-adapter gate has run on both adapters and its verdict — either way — is written
down; and a failed frame cannot report itself drawn (tested, not asserted).

**Done** (2026-08-02). The record and the two answered questions are in "Where we
are"; the verdicts live in ADRs 0005 and 0006; the gates live in
`crates/quorra-gpu/tests/{m1,perf_gate}.rs`. The surface path is proven end to end:
`examples/window_smoke.rs` presents real frames to a real window under `Xvfb`
(lavapipe, the caller's own CI arrangement), verified by reading the window's pixels
back with `xwd` — the centre and field pixels match the scene within ADR 0006's
bound. CI runs the smoke on every push; presenting on RADV to the user's live display
still awaits the user, since Xvfb has no DRI3 for it.

## M2 — The scene, retained and viewport-free

**Deliverable:** `quorra-scene`'s real types — `geom`, `paint` (solid half), `scene` —
plus `SceneBuilder`, `Scene: Send + Sync`, `Scene::cost()`, and the upload/release path
of §2.2 on the device side. The API integration notes below are settled with the caller
before this milestone freezes signatures.

**The work, in order:**

1. `geom`: `f32` throughout matching the caller; move/line/cubic/close and no
   quadratics; `Affine` with `preserves_axes` and `max_stretch`, because §1.1's sorter
   and §6.3's scale bucket ask for exactly those.
2. Input validation at the boundary (§4.7): coordinates and transforms outside stated
   limits are refused loudly with typed errors; no NaN survives into geometry; no
   allocation is sized from an unchecked number.
3. `SceneBuilder` and `Scene`: the flat command list, clip chains as data, group
   nesting checked against the bound of 16, `finish` computing `cost`.
4. `Device::upload_outline` / `upload_image` / `upload_ramp` / `upload_mesh` /
   `release`, each upload checked against the resource budget.
5. The fuzz target on the builder and the encoder — structured scene input, run from
   this milestone onwards, every crasher a permanent regression test (principle 3).
6. The M2 tests the scene skeleton already names: no viewport anywhere (a scene renders
   byte-identically wherever it was built); `Send + Sync` statically asserted; a blank
   scene is legitimate; encode-order independence.

**Done when:** the caller-shaped round trip works — 107 outlines uploaded once,
referenced thousands of times, a zoom re-uploading nothing; `Scene::cost()` returns the
number §11.5 asks about, recorded for M8; byte equality across repeated encodes of one
scene is gated.

**Done** (2026-08-02), with one honest boundary: the upload/identity/budget half of
the round trip is proven (`tests/m2.rs`); the *drawable* half — the outlines actually
painting — is M4/M5's proof by definition. §11.5's first number: a 5 933-fill scene
costs ~380 KB retained (64 bytes per command), so a dozen resident dense pages are
~5 MB of commands — comfortably inside the brief's 70 MB affordability line, before
any compaction. Deviations and refinements are integration notes 1, 5 and 7.

## M3 — Rectangles and rectangular clips, analytically

**Deliverable:** `SceneBuilder::rect`, clip chains resolved and honoured, the
rectangle-and-residue split of §1.4 (with the residue refused loudly for now — the
path lane that draws it is M5, and a refusal that names the reason beats a silent
approximation, §5).

**The work, in order:**

1. Phase-1 clip resolution: chains to a single intersected rectangle plus residue;
   scissor where the rectangle bounds the batch; four floats and a comparison
   otherwise. Never an R8 mask for a rectangle (§6.4).
2. Empty-clip and absent-clip semantics, tested as distinct.
3. The distinct-region counter (§1.4) — the count of resolved regions, not a hit rate.
4. The scene suite grows its full-page fixture: **one full page at a real window
   size**, because a suite of small scenes tests small scenes, and the first real page
   at a real size came back blank in the caller's tree with nothing able to see it
   (trap 12b).

**Done when:** a page-6-shaped scene — thousands of rect fills, 303 identical clip
states collapsing to one region — renders correctly and the counter proves the
collapse; the caller's worst case of 3 608 chains is synthesised and stays within
budget; perf gates cover the rectangle path.

**Done** (2026-08-02). The fixtures live in `crates/quorra-gpu/tests/m3.rs`; the
resolution design is ADR 0007. One deliberate narrowing to note: the recogniser
accepts line-edged rectangles only — a rectangle drawn as collinear cubics takes the
M5 residue path, and loosening that is a measurement-backed decision for M5 if real
corpora need it.

## M4 — The glyph atlas

**Deliverable:** the R8 coverage atlas with eviction and a caller-set budget, keyed on
`(outline, scale bucket, sub-pixel phase)` with the **quantum settable, documented, and
switchable off** — §4.5's fifth decision, the one that is ours to expose. Default 1/16
of a pixel: 1/16 reused 5.0× on a dense page and left the oracle's verdicts unmoved;
1/8 contradicted pages.

**The work, in order:**

1. The key and the quantum plumbing through `Options` — a silent quantum would move the
   text and nobody could attribute it.
2. The packer, eviction, and the budget check; `atlas_entries` and
   `atlas_distinct_keys` in `Counters` — the distinct-key count, not the hit rate.
3. The rasteriser decision, as an ADR with a measurement: coverage rasterised on the
   GPU, or on the CPU by `tiny-skia` — the latter makes the glyphs come from the same
   code that is the caller's correctness oracle, a correctness argument no other
   arrangement gets for free, against a dependency and a per-glyph transfer this
   library would otherwise not have (§6.3 offers it as persuasive, not prescriptive).
4. The glyph lane in the sorter: fills whose device size fits an entry become quads;
   the atlas-miss path (too large, budget exhausted) falls through to the path lane —
   or, until M5 exists, to a loud refusal.
5. The measurement for §11.3: the atlas's cost on `tracemonkey.pdf`-shaped reuse
   (1.3×) as well as its win on dense-text reuse (5.0×), because a cache that is 5× on
   one page and a net loss on another is a decision, not a feature — and the decision
   needs both numbers.

**Done when:** a dense-text scene at window scale beats its M3-era self by a measured
margin; the low-reuse cost is known and the decision it forces is written down; the
quantum is proven settable and off-able by test.

**Done** (2026-08-02): ADRs 0008/0009, the record in "Where we are", gates in
`tests/{m2,m45}.rs`. The tiny-skia question resolved as "neither": our own
rasteriser, for determinism and the dependency posture — the oracle argument waits
for M9 where the oracle actually is.

## M5 — General path coverage, designed after the census

**Deliverable:** fills and strokes of arbitrary cubic outlines, both fill rules, nested
subpaths wound the same way, non-rectangular clip residues — the path lane, built as
whichever of §1.6's candidates the census justifies.

**The work, in order:**

1. **The census first (§11.2):** over the caller's corpus display lists, count what
   share of commands miss the glyph and rectangle lanes, and what they are. The corpus
   lives in the caller's tree and the two trees stay independent until M9, so the
   fixture is *data* — display lists serialised to a neutral form and carried over —
   never a dependency edge.
2. The design ADR: which candidate, chosen by the census plus the oracle constraint
   (coverage quantisation must not move oracle verdicts).
3. Fills: both rules, the caller's deep-nesting cases, clip residues through the same
   machinery, the mask targets it produces feeding §1.4's region-keyed cache.
4. Strokes: expansion to fill outlines at encode — caps, joins, miter limits only,
   because widths, dashing and degenerate subpaths arrive resolved (§4.5) and our job
   is not to undo any of it.
5. The atlas-miss fall-through from M4 becomes real: a glyph at extreme zoom takes this
   lane and the seam is invisible (tested at the boundary scale).

**Done when:** the census number is in the ADR; the three day-one scenes that involve
paths pass; every command a real corpus page contains renders through some lane with no
refusals left except genuine budget refusals.

**Done in implementation, open in measurement** (2026-08-02): fills (both rules,
nested same-winding pinned end to end), strokes (caps/joins/miter, cross-checked
against the rectangle they degenerate to), residue clips (the triangle-clip fixture
masks correctly), oblique rectangles — all drawable; the remaining M6-refusals are
groups, non-Normal blends and `Compose::Src`, each named. The census (§11.2) has not
run — it needs corpus fixtures from the caller's tree — and stays the recorded
condition for revisiting the lane design (ADR 0008).

## M6 — Clause 11, natively

**Deliverable:** the four things a general vector API lacks, and the reason this
library exists (§1.3's design, made real).

1. **Coverage-modulated Porter-Duff Source** (§4.1) — the compose mechanism decided
   between dual-source blending and a per-element compose pass, with the diagonal-edge
   knockout fixture as judge, because axis-aligned rectangles would agree while being
   wrong.
2. **Soft masks reduced on the device** (§4.2) — `Alpha`, `Luminosity { backdrop }`,
   the optional 256-entry `/TR` table, the mask group built through the same
   `SceneBuilder`. No readback, no round trip. The conformance test is part of the
   definition of done: all 256 mask bytes through both rules against the caller's
   `SoftMask::value`, byte-equal; a luminosity mask with a non-black backdrop; a
   non-identity transfer sampled at both endpoints.
3. **Sixteen blend modes** (§4.3), each WGSL function carrying its clause number, the
   four non-separable ones written from clause 11.3.5's `Lum`, `ClipColor`, `SetLum`
   and `SetSat` — our own implementation, deliberately unshared, tested against the
   caller's sixteen-mode fixture that once found three of `tiny-skia`'s wrong by up to
   113 of 255.
4. **Groups compositing onto transparency** (§4.4), isolated, painted once under the
   group's own alpha and blend mode, depth bounded at 16 — plus the backdrop-read
   mechanism for non-Normal blends, measured on scenes where the batch cuts bite.

The layer-precision ADR (§1.3's open question) is written here, with the conformance
tests in hand.

**Done when:** the caller's three day-one scenes pass — the knockout diagonal, the
sixteen modes, the full page at window scale — and the soft-mask byte-agreement test is
green on both adapters.

**Done** (2026-08-02): ADR 0010, gates in `tests/m6.rs` — the 256-byte agreement is
exact, the sixteen modes match a clause-transcribed reference, the knockout diagonal
matches §11.4.6's formula, isolation and group alpha hold, and the compositor is
byte-deterministic per adapter. The layer-precision question closed on the side of
rgba8 layers (quantisation between commands is the CPU reference's model and the
mask byte-agreement's precondition); full-target layer textures are the recorded
optimisation candidate for a bbox-bounded version, with M8's measurements. Item 4's
"onto transparency" is the isolated group only, which is what §4.4 promised at the
time; clause 11.4.4's other initial backdrop arrived later, in ADR 0019.

## M7 — Shadings, meshes, images

**Deliverable:** axial, radial and function-based shadings from a `RampId` (clause
8.7.4.5); the caller's pre-rasterised mesh consumed as-is (integration note 5); decoded
RGBA8 images with the filter decision arriving resolved (integration note 1). Straight
alpha at the boundary, premultiplied internally, rendered onto transparency, always.
All of these pipelines compile on first use (§1.8) — a page of plain text never pays
for them. The open `ShadingKind` question — geometry on the paint versus resolved at
upload — is decided here with a measurement and an ADR.

**Done** (2026-08-02): ADR 0011, the record in "Where we are", gates in
`tests/m7.rs`. The `ShadingKind` decision landed on **geometry on the paint** (six
floats per placement; the uploaded ramp serves every placement, which is §2.2's
economy — integration note 9). The caller's *sampled* function shadings map to
images on this side, and meshes arrive pre-rasterised at device resolution
(integration note 5) and sample at absolute device pixels. Image, shading and mesh
pipelines compile on first use; the warm set is unchanged, so startup did not move.

## M8 — Damage, and the rest of the performance contract

**Deliverable:** per-tile dirty tracking against a retained scene and a cheap viewport,
so a caret blink redraws a few tiles rather than a page (§6.5) — `Viewport::damage`
honoured exactly, and a damage list we cannot honour meaning a full redraw *and a
`Report`*, never a stale region. The persisted pipeline cache round-trips through a
real second launch — **which requires an ADR first**: wgpu 30's
`create_pipeline_cache` is an `unsafe fn` (the blob is trusted input), and this tree
is `#![forbid(unsafe_code)]`; the ADR weighs a scoped exception (principle 3's
benchmark-plus-invariant route) against not offering the cache, with the startup
measurement in hand. Pool-shrink policy decided. §11.5's verdict: `Scene` memory
against the dozen-resident-pages target, with M2's number as the input.

**Done** (2026-08-02): ADRs 0012 (damage as scissored rendering plus rectangle
patching — honoured exactly on `Texture` targets, reported on the others, refused
when malformed) and 0013 (no pipeline-cache blob: the benchmark showed nothing
user-visible to win, so the `unsafe` exception was declined and
`#![forbid(unsafe_code)]` stands). The record and numbers are in "Where we are";
gates in `tests/m8.rs`. The plan's own phrasing was ahead of the measurement in one
place: there is no texture pool to give a shrink policy to — per-frame creation is
inside the measured budget, and the pool waits for a number that demands it.

## M9 — The swap

**Deliverable:** the caller's `render-gpu` replaced — an adapter in *their* tree
implementing their `Rasterizer` over quorra, which is where the bridging of integration
note 2 lives. Held to §10 in full: the cross-backend scene suite; the 1 794-page
oracle, which has refused two of the caller's own optimisations after they passed every
unit test and will not be kind; byte equality where we claim it; and a window driven by
real key presses under `Xvfb`. Not before: this is the last milestone, and the two
trees stay independent until it.

**Progress** (2026-08-02): the adapter exists and passes the caller's bar.
`crates/render-quorra` in the caller's tree implements their `Rasterizer` over this
library: the display list maps command-for-command (the two vocabularies were shaped
by one contract); dashes cut through the same `kurbo::dash` their Vello backend uses
and §8.5.3.2's zero-length rules through their shared helpers; sampled shadings —
which their GPU backend *refuses* — draw as domain images clipped to the fill;
meshes go through their shared `MeshRaster`; the medium is imposed by their own
function. Two integration findings became fixes on this side: `Paint::Shading` grew
its own transform (a shading anchors to the page, §8.7.4.3 — the command-space
geometry M7 chose could not express the caller's model), and `Stroke::width`'s doc
now tells the truth (device pixels). Two adapter defects were found by the caller's
own instruments and fixed: an ABA bug in the `Arc`-identity caches (pointer keys
without pinning served stale outlines by allocator mood — the caches now hold the
`Arc`), and stroke widths mistaking their `device_width()` for device units (it
answers in path units; ×`max_stretch` carries it over, and page 6's rules stopped
being 2× thin at window scale). **Gates, all green on RADV:** the eleven
cross-backend scenes at the Vello suite's own thresholds (all sixteen blend modes,
knockout, soft masks, an interpreted PDF); the real-page suite — with exact phases
quorra sits at the Vello backend's distance from the CPU oracle on every case
(worst tile ≤ 3.0 vs their 3.2; one case *better*: 2.6 vs 4.2), and the 1/16
quantum's cost is pinned as its own envelope. **Perf at the trait boundary** (page
6, fastest of 12): 4.4 ms vs CPU 4.9 / Vello 6.9 at scale 1.0; 10.4 ms vs CPU 6.1 /
Vello 11.6 at window scale — both GPU backends readback-bound exactly as §11.1
measured, which is why the remaining work is the part that escapes the trait.
**Done** (2026-08-03): the swap itself landed. `render-quorra` grew the tier-2
`QuorraPresenter` — quorra's device owns the window's surface, and one call draws
the page, the selection, the sidebar and the modal card (all display lists through
the same translation the headless gates exercise) and presents, with no readback
anywhere. To make one scene carry several lists at their own placements, the
adapter bakes each list's target transform into its commands and leaves the
viewport at identity. `viewer-ui`'s render path moved onto the presenter
(−205 lines net: the RenderContext, intermediate texture, blitter and banding
machinery all left with the backend that needed them); the CPU backend keeps its
oracle and fallback roles, the fallback now presenting through the same quorra
surface as an image. **Verified under Xvfb with real key presses**: the ISO
32000-2 cover (images, chrome) and Page-Down navigation to the dense table of
contents, zero fallback notes. One follow-up remains with the owner: a
corpus-scale quorra-vs-oracle sweep (the 1 794-page oracle itself judges the CPU
backend by design, so this is an additional gate, not a missing one). The
CI-reachability question closed on 2026-08-03: this library lives at
https://github.com/close2/quorra, and the viewer consumes it as a git dependency —
revision pinned by its Cargo.lock, source named in its deny.toml, with a
documented `[patch]` route for developing against a local checkout.

## What we must build ourselves, and when

§10 tells us what we will be judged by, which means the harness is ours to write rather
than to wait for:

| | lands with |
|---|---|
| Headless golden renders, PNG artefacts, byte-equality gate | M1 |
| Adapter-to-adapter byte equality, RADV against lavapipe (§11.4) | M1, re-checked every milestone |
| Perf gates: startup split, encode, execute, readback, one real page at a window size | M1 |
| A scene suite that starts small and includes **one full page at a real window size** | M3 |
| Fuzzing the scene boundary (principle 3) | M2 |
| The corpus fixture: the caller's display lists as neutral data, for the census and beyond | M5 |
| The knockout group with its diagonal edge; the sixteen-mode scene; the soft-mask agreement test | M6 |

## Which milestone fills which module

The skeleton's modules each state their contract (ADR 0003); this is the map of who
deletes which `text` block.

| module | filled by |
|---|---|
| `quorra-gpu`: `device`, `pipeline`, `target`, `frame`, `viewport`, `report`, `error`, `surface`, `readback`, `timing` | M1 ✓ (and every milestone after adds to `pipeline`) |
| `quorra-gpu`: `resources` | M2 ✓ |
| `quorra-scene`: `geom`, `scene`, `paint` (solid half), `image`/`mesh` (upload specs) | M2 ✓ (geom landed with M1) |
| `quorra-gpu`: `atlas` | M4 ✓ |
| `quorra-scene`: `mask`; `quorra-gpu`: `mask` | M6 ✓ |
| `quorra-scene`: `paint` (shading half), `image`, `mesh` | M7 ✓ |

## Integration notes against the caller — settle these before M2 freezes the API

These are places where the brief and `pdf-render` as it stands today do not line up
exactly. None is a problem; each is a question whose answer belongs in the API rather
than in a translation layer's judgement (§4.5: a decision either side can make alone is
a decision neither side has made). The numbering is load-bearing — module docs in the
tree cite these notes by number.

1. **The image filter flags are not flags.** §4.5 names `Image::is_smoothed` and
   `Image::area_averaged` as flags to honour. In the tree they are *methods taking the
   placement transform* — `is_smoothed(placement)`, `area_averaged(placement) ->
   Option<Image>`. So what must reach us is the **resolved** decision for the placement
   the command carries, not the flag it was derived from. Refined at M2, from the
   caller's types: since the decision is per *placement* and an upload is per *image*,
   the resolved filter belongs on the image **command** (M7), and `ImageSpec` carries
   pixels only — which is also what lets one upload serve every zoom level.
2. **`Viewport` versus `TargetSpec`.** §2.4's viewport carries a full affine — scale,
   y-flip and tile offset. The trait in the tree carries a scale and a pixel budget
   (`TargetSpec::for_page(list, scale, max_pixels)`). The affine is the more general
   of the two and is what tiled output and damage need; the bridging belongs in the
   caller's adapter, and the rounding rule for a fractional page stays theirs.
3. **The soft-mask transfer function.** The brief says a 256-entry lookup table because
   a mask value is one byte. The tree has a `Transfer` type; we take `[u8; 256]` and
   the caller samples. Confirm the sampling convention — inclusive of both endpoints —
   in the conformance test, not in prose.
4. **Which `raw-window-handle`.** §2.1 asks for `raw-window-handle` and nothing more
   specific, but the version has to be the one `wgpu` was built against or a surface
   cannot be created. Reach it through a `wgpu` re-export if one exists in 30.0.0;
   otherwise pin the same version here and say so.
5. **`MeshRaster` is shared on purpose.** Both of the caller's backends share the
   pre-rasterised mesh because neither rasteriser has the primitive and a second copy
   would drift. We inherit that: we consume the mesh, we do not re-triangulate it.
   Confirmed at M2 from the caller's type: a `MeshRaster` is a **device-resolution**
   positioned raster, so a mesh upload is viewport-dependent and a zoom re-uploads its
   meshes — the cost of the shared-rasteriser correctness argument, taken upstream and
   inherited knowingly.
6. **Colour is not ours.** Device RGB arrives; `ColourSpace::to_rgb` upstream is the
   only place a colour becomes RGB, and adding a second one is forbidden. If we offer
   colour management the caller cannot use us.
7. **`release` returns a `Result`, deviating from §2.2's `()` signature.** A release
   of an id the device never issued — or issued and already released — is
   `DeviceError::UnknownResource`, because a double release is a caller bug and a
   no-op would hide it. The brief calls its signatures illustrative; the property
   (no silent error swallowing) outranks the shape, and this is flagged for the API
   conversation before M9.
8. **The `mask` parameter comes last**, in every builder method, rather than beside
   `clip` as the brief's illustrative signatures place it: growing the vocabulary
   milestone by milestone was then a mechanical widening at every call site. Flagged
   with note 7 for the API conversation before M9.
9. **Shading geometry travels on the paint; sampled shadings become images.**
   Decided at M7 (ADR 0011), refined at M9: `Paint::Shading { ramp, kind,
   transform }` carries the axial or radial geometry (six floats) in the
   **shading's own space**, with the paint's transform mapping it into the scene —
   §8.7.4.3's shading matrix, anchoring a shading to the page rather than to the
   path it fills, exactly as the caller's `Shading { kind, transform }` states it.
   One uploaded ramp serves every placement and zoom — §2.2's economy, and the
   scene stays viewport-free (§2.3). The caller's `ShadingKind::Sampled` — its
   display list holds a *grid*, not a function — maps to an image upload plus an
   image command in the M9 adapter; `ShadingKind::Mesh` maps to `upload_mesh` +
   `Paint::Mesh` (note 5's device-resolution caveat applies).

## Not planned, ever

§9's non-goals: colour management, font loading, shaping, hinting, text layout, bidi,
filters and effects beyond §4, any document format, a scene graph or animation, hit
testing. `deny.toml` enforces as much of the list as a dependency policy can express.
