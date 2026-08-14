# API change — two counters on `Counters` (ADR 0049)

Date: 2026-08-15. For the owner to carry to the caller, as `QUORRA_UPGRADE.md`'s process
asks: an API change is written down for them rather than merely shipped.

## What moved

`quorra::Counters` gains two fields:

```rust
pub struct Counters {
    // …
    /// Distinct residue clip regions this frame rasterised.
    pub clip_residue_regions: u32,
    /// Residue rasterisations charged to a single command's tile.
    pub clip_residue_tiles: u32,
}
```

Nothing else changes: no method's signature, no error variant, no option, no behaviour a
caller asks for by name. `Counters` still derives `Debug, Clone, Copy, Default, PartialEq,
Eq`.

## What the caller must do

**Nothing, unless they construct a `Counters` themselves.** Reading fields, copying the
struct, comparing two of them and `..Default::default()` all keep compiling. The one
construction that breaks is an exhaustive struct literal —

```rust
let counters = Counters { commands: 12, distinct_outlines: 9, /* …every field… */ };
```

— which stops compiling until the two fields are added or `..Default::default()` is used.
A `Counters` in their tree is a value we hand them, so this is expected to bite nothing;
it is written down because a struct without `#[non_exhaustive]` makes a new field a
breaking change whether or not anybody trips over it.

## What they are for

`clip_residue_regions` is the count of **distinct clip regions rasterised** for chains
whose links are not all rectangles, and `clip_residue_tiles` the number of times a chain
was rasterised over one command's tile instead — the work the regions did *not* remove.
Both are exact functions of the scene and the viewport, so both compare by equality across
machines and adapters, and both are keys rather than a hit rate for the reason their own
ADR 0132 states.

They answer, on any page of theirs, the question their `QUORRA_FEEDBACK.md` §15 asks from
the other side: a page reporting `600` tiles and `1` residue region states one curved clip
and draws six hundred marks under it, and the clip cost one rasterisation. A page
reporting `0` regions and `40` tiles has forty chains each used once, or clips so much
larger than their marks that keeping the region would have cost more than it saved — and
that is a page shape worth telling us about, because it is the one the next lever on this
seam is for (ADR 0049's "revisit when").

## Whether it is worth a version bump on its own

No. It travels with the round that adds it; the release note is the ADR.
