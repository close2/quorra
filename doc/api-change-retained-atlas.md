# API change — ADR 0050

For `/home/cl/projects/pdf-viewer`, which consumes this library as a git dependency. Two
additions and one behaviour change; **nothing was removed or renamed**, and a caller that
does nothing keeps compiling and keeps drawing the same pixels.

## 1. `Counters` gains two fields

```rust
pub struct Counters {
    // ...
    /// Atlas bytes this frame's distinct glyph keys asked for, hits included.
    pub atlas_working_set_bytes: u64,
    /// Whether the atlas was repacked after this frame.
    pub atlas_repacked: bool,
    // ...
}
```

**What the caller must do: nothing, unless it builds a `Counters` with a struct literal.**
The fields are `pub` and the struct is not `#[non_exhaustive]`, so `Counters { commands: n,
.. }` still compiles and `Counters { a, b, c, d, e, f, g, h, i, j, k }` naming every field
positionally does not. We return this type and never take it; the only plausible literal
is in a test fixture or a mock. `Counters: Default` is unchanged, so `..Default::default()`
absorbs both.

**What they are for.** `render-quorra/src/present.rs` already copies `commands`,
`commands_culled` and `bytes_uploaded` out of `Counters` beside `encode_source`. These two
belong in the same place, because they answer the question that row cannot:

- `atlas_working_set_bytes` is what holding **all** of a page's distinct glyph tiles would
  cost. Compare it against the `atlas_budget` the device was built with. A page whose
  working set exceeds the budget cannot keep its glyphs cached however the packer behaves,
  and will re-rasterise the remainder into the scratch sheet on every frame that encodes.
  Raising `Options::atlas_budget` is the lever, and this is the number to size it from.
- `atlas_repacked` is true on a frame after which the atlas was thrown away and re-packed.
  **That is the one event that makes a `RetainedScene`'s encode stale**, so a viewer
  watching its retained frames re-encode can now tell whether the atlas is the reason. A
  page that changes settles after at most one; true on frame after frame means the atlas
  is thrashing between two pages that do not fit beside each other, and the answer is a
  larger budget.

## 2. `DeviceError` gains one variant

```rust
DeviceError::ResourceIdsExhausted { limit: u32 }
```

Returned by `Device::upload_outline`, `upload_image`, `upload_ramp` and `upload_mesh` when
this device has issued all `u32::MAX` resource identifiers. Ids are never reused, because a
reissued one would make a retained encode draw a resource it never named while every
staleness check agreed nothing had moved (ADR 0050's audit of ADR 0048's key). The counter
used to wrap silently.

**What the caller must do:** nothing, unless it matches `DeviceError` exhaustively without
a `_` arm. No document reaches four billion uploads; this is a bound that is now stated
rather than a wrap that was not.

## 3. Behaviour: a page that overflows the atlas now replays

Before, a frame whose glyph tiles overflowed the atlas could repack it afterwards, and the
repack invalidated the encode of the frame that caused it — so, in one measured band, a
still page re-encoded on every frame for ever. It now repacks only when there is space to
reclaim from an *earlier* frame's tiles, which bounds a page at **one repack and two
encodes, then replays**.

**No pixel changes.** A tile drawn through the atlas and the same tile drawn through the
scratch sheet are the same bytes. What changed is when a layout is discarded.

**What the caller should expect to see:** on magnified text with a modest atlas budget,
`Frame::encode_source()` becoming `Replayed` where it used to stay `Encoded`, and
`Timings::encode` dropping to zero on those frames. If their corpus run has per-page
timings at scale 4, that is where it will show.

Nothing in `QUORRA_RETAINED_FRAME.md`'s adoption instructions changes: the obligation is
still to call `set_scene` when the content changes, and nothing else.
