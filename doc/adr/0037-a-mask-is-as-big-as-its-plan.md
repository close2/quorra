# ADR 0037 — A mask is as big as its plan, and outside it a constant

Status: accepted, 2026-08-13, and landed in two commits — the placement everywhere at the
target's rectangle (verified to change nothing), then the sizing that makes it smaller.

## Context

ADR 0036 sized every layer pair to its plan and left one thing whole: a **soft mask is
realised at the target**, whatever its group covers, as an R8 the frame samples in device
space. On the corpus page that still refused for bytes at 4× that is 93 MB of a 291 MB
frame, against a 268 MiB budget — and it is a per-frame cost on any page that soft-masks
at zoom, not only on the ones that refuse.

§11.5 is what makes it wrong rather than merely wasteful: a soft mask **is** a
transparency group rendered at device resolution, and a group covers what it draws. There
was never a clause reason for the mask's texture to be the page.

ADR 0036 also left the two halves of §5's count-then-allocate disagreeing about masks. A
mask group's plan was *priced* by `peak_pair_bytes` at its own bounds and *realised* by
`realise_masks` at the target, so the budget check passed frames that then allocated more
than they had promised — sixteen times more on the test page below. Nothing observed it,
because a budget that is generous enough is a budget nobody hits.

## Decision

**A mask is realised at its plan's device bounds**, like every other layer, and every
sampler of it carries three numbers instead of relying on the texture being the page:

| | |
|---|---|
| `origin` | the device pixel the mask's texel (0, 0) is |
| `size` | its extent in texels; `(0, 0)` for a mask that does not exist |
| `outside` | what the mask is beyond that rectangle |

### The reduce needs no origin

The mask's group renders into a texture at the plan's rectangle and the reduce writes an
R8 **of the same size**, reading at the fragment's own position and writing at the same
one. Two textures of one size map 1:1 wherever that rectangle sits, so `reduce.wgsl` is
untouched by this ADR. Only the five sites that *sample* a finished mask move.

### What `outside` is, and why it is not zero

The mask's group marks nothing outside its bounds, so what a full-target realisation held
out there is exactly **what the reduce writes for a fully transparent pixel**:

- §11.5.2's alpha rule derives 0 from an absent group, so the value is `transfer[0]`;
- §11.5.3's rule composites the group onto *a fully opaque backdrop of a specified colour*
  and takes the luminosity of the result — with no source, that is the luminosity of the
  backdrop, through the transfer.

So a luminosity mask over white admits everything outside its group and one over the
caller's default black admits nothing, which is the sentence `quorra-scene`'s `mask` module
already had. **Zero would have been a plausible-looking wrong page** — every luminosity
mask over a light backdrop, cropped to its group's rectangle — and §5 names that the worst
outcome either project has a word for. Clamping to the nearest edge texel would have been
a second one, smearing the group's boundary across the page.

An **absent** mask becomes the same case rather than a special one: size `(0, 0)`, outside
1. The 1 × 1 white texture is still bound, because a bind-group entry is not optional, but
nothing reads it.

### Where the parameters live

`Globals` is per **region** — one pass's attachment — and a mask is per **batch**: one
pass draws rect batches under different masks. So the lanes carry the placement in group 1
beside the mask texture, whose bind group is already keyed by mask, and the layout grows a
32-byte uniform. The three single-quad passes (image, shading, composite) carry it in their
own per-op uniforms, which grow by 32 bytes each.

## Why it lands in two commits

ADR 0036's staging, for ADR 0036's reason. The first commit adds the placement to all five
shaders, the `mask.rs` module that owns it, and the constant — with the masks still
realised whole, so the frame must be what it was. It was: 212 tests and the caller's corpus
at 919 agree / 37 differ / 1 refused, the same verdicts and the same per-page numbers as
the commit before it.

The second turns the size on. `tests/mask_regions.rs` is written in the first commit and
states the property from the clause rather than from the implementation, which is what lets
it be the check for the second: outside a mask group's marks the device must produce
§11.5's transparent reduction, inside them the group's own, and a group that marks nothing
at all must be transparent everywhere — both rules, a non-black backdrop, and a transfer
that inverts so that "admits nothing" and "admits everything" are both covered.

## What it bought

`issue16287.pdf` at 4×, a 2 448 × 9 504 page, in bytes the frame prices before it allocates:

| | root pair | chain below it | masks | total |
|---|---:|---:|---:|---:|
| before | 186 126 336 | 12 009 600 | 93 063 168 | 291 199 104 |
| after | 186 126 336 | 12 009 600 | **5 052 238** | **203 188 174** |

The masks cost **5.4 %** of what they did, and the page draws inside the caller's
268 435 456 budget — and *agrees* with the oracle rather than merely drawing.

The caller's corpus, against the commit before this ADR in the same copy of their tree:

| | agree | differ | refused |
|---|---:|---:|---:|
| scale 4, before | 931 | 16 | 5 |
| scale 4, after | **932** | 16 | **4** |
| scale 1, either | 919 | 37 | 1 |

The 37 differing pages are the same 37 at both scales, to the last digit of every mean,
worst tile and SSIM. That equality is the evidence that this moved memory and not pixels;
the one page that moved is the one the arithmetic above predicted.

## What it cost

**A per-sample branch, everywhere.** Every masked fragment in every lane now tests its
device position against a rectangle before the texture fetch, where it used to clamp. The
clamp was not free either (a `textureDimensions` and a `min`), and no page of the corpus
changes timing measurably — but this is a cost paid by pages that gain nothing, and it is
recorded as one rather than argued away.

**A constant computed twice.** `mask::transparent_value` is a second implementation of
`reduce.wgsl`'s own arithmetic for the transparent case, on the CPU because five uniforms
need the number before any pass runs. That is the same shape as the reduction's own
relationship to the caller's `SoftMask::value` (§4.2), and it is held the same way — by a
test that renders the value on the device and compares, over both rules and a spread of
backdrops and tables. The alternative, feeding the constant *into* the reduce so the two
cannot disagree, was rejected: an independent implementation that a test compares is
stronger evidence than an agreement by construction, and it catches a divergence in either
direction.

**`Device::warm_for` predicts one size less well again.** ADR 0035 warms a target-sized
pair; ADR 0036 made that the right size only for the root; this makes it the right size for
no mask. Unchanged in substance — what it warms is still right for the one plan that is
always target-sized.

## What is left, and what this changes about it

The page's frame is now **91.6 % one texture**: the root's ping-pong pair, at the target's
size because the root *is* the target. Whether it needs two full-target textures, or can
ping-pong against the target the frame is drawing into, is the next question on this path
and it is unmeasured. Nothing else on that page is worth shrinking.
