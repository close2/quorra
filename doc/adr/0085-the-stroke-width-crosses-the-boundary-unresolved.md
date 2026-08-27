# 0085 — The stroke width crosses the boundary unresolved

Date: 2026-08-27. Status: **accepted, and built** — the §4.5 contract amendment the
project owner approved on 2026-08-27 ("why wouldn't I? what is the downside?"), asked
for by ADR 0084's staged plan, where it is stage one: nothing later in that plan is
buildable while any entry in a scene depends on the view. The caller's side is their
ADR 0701.

## What changed

`quorra_scene::Stroke::width` is now stated in the **command's own space**, zero is a
legitimate value, and the stroke carries §10.7.5's `adjust` flag. The encode resolves
the device width per placement (`raster::resolve_width`), where the composed transform
is known:

> A line width of 0 shall denote the thinnest line that can be rendered at device
> resolution: 1 device pixel wide. (ISO 32000-2 §8.4.3.2)

plus §10.7.5's automatic stroke adjustment — a width under half a device pixel drawn
at one — when `adjust` asks for it. The comparison mirrors the caller's own
`Stroke::device_width` statement for statement (theirs in path space,
`width < 0.5 · (1/stretch)`; ours the same inequality multiplied through), and
`DeviceTransform::max_stretch` mirrors their `Transform::max_stretch` bit for bit, so
the width the encode resolves is the width their resolution produced — exactly for the
common case, within an ulp for the substituted one (ADR 0082's contract covers the
ulp; the caller's windowed A/B measured no moved pixel).

**What §4.5 still settles upstream**: dashing (cut in path space, viewport-independent,
already right where it is), degenerate-subpath splitting and `/Interpolate` and area
averaging. The degenerate split still consumes a path-space width upstream and is the
next entry to move — it sizes dots by the resolved width, so it is the remaining
view-dependence in a stroked scene — deliberately its own change, because it carries
§8.5.3.2's cap semantics with it.

## Why

A scene that states a device width is a scene that dies with the magnification — the
consequence the old field's doc stated in its own words. With the width scene-space,
a stroked scene is true at every viewport, which is what ADR 0084's stages two and
three (arrangement-space scenes, record replay) require of every command they retain.
The cost the caller's ADR 0701 records: one clause's arithmetic now lives in two
trees, held together by mirrored statements and the cross-lane pixel gates.

## Held by

Every existing stroke gate, restated in scene units where it had baked the viewport in
(`coverage_lanes`, `scale_invariance`, `thin_marks`, `shading_space` — each edit is the
amendment's meaning made concrete: the fixture states its band once, the encode carries
the magnification). The full suite is green, clippy clean, and the caller's sixty
`render-quorra` tests pass against this change unmodified.
