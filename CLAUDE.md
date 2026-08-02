# Project: quorra — a GPU document renderer

> **New here?** Read `doc/RENDER_LIBRARY.md` first: it is the brief this library exists
> to satisfy, written by the PDF viewer that will consume it, and every requirement in
> it has a measurement behind it. Then `doc/PLAN.md` for where we are and what is next.

A 2D renderer for documents, in Rust, on wgpu. The one-sentence brief, quoted from
`doc/RENDER_LIBRARY.md` §0:

> Vello is a general 2D vector renderer that happens to be pointed at documents; what
> this viewer wants is a **document** renderer — one whose fast paths assume that most
> of a page is the same few glyph outlines repeated at many sub-pixel phases plus
> axis-aligned rectangles, and which treats general curve filling as the rare case
> rather than the uniform one — and which expresses ISO 32000-2 clause 11's
> transparency model *natively*, because that is the part an SVG-shaped model cannot be
> patched into.

## Who our caller is, and what that means

The consuming project is the PDF viewer at `/home/cl/projects/pdf-viewer`. Its
`pdf-render` crate defines a display list and a `Rasterizer` trait; **that is the
contract, and it does not change for us.** Three consequences that shape everything
here:

1. **We never see a PDF byte.** Everything reaching us is data that tree produced from
   untrusted input, never the untrusted input itself. That is why the viewer's
   `#![forbid(unsafe_code)]`-because-of-hostile-input rule does not bind us — but see
   principle 3 for the rule that does.
2. **Their CPU backend is our oracle.** `render-cpu` (`tiny-skia`, plus its own blend
   modes and soft-mask reduction) renders the same display list. A difference between
   us and it is a defect in one of the two, and the comparison is the instrument that
   finds it. It has refused two of that project's own optimisations after they passed
   every unit test.
3. **A decision either side can make alone is a decision neither side has made.**
   Four things are settled upstream on purpose — `/Interpolate`, area averaging, a
   `0 w` stroke's device width, and degenerate subpaths (§4.5). We honour them; we do
   not re-take them. One decision is ours to make and to expose: the sub-pixel quantum
   of the glyph cache.

## Non-negotiable principles

These are stated by the project owner and override convenience, velocity, and any
default habit. When a suggestion conflicts with one of these, say so explicitly rather
than quietly compromising.

### 1. Quality first — no shortcuts

- No placeholder implementations, no `todo!()` left in merged code, no "we'll fix it
  later" paths. If something cannot be done properly now, it is not started now.
- No silent error swallowing. Every error is typed, propagated, and handled somewhere
  deliberate. No `unwrap()` outside tests and provably-infallible cases (and then with
  a comment naming why it cannot fail).
- Every public item documented, with `missing_docs` enforced.
- `clippy::pedantic` clean. Warnings are errors in CI.
- If a shortcut is genuinely the right call, it is documented as a deliberate decision
  with its cost written down — never taken silently.

### 2. Fast — including startup

- Performance is a feature owned from day one, not an optimization phase later.
- "Genuinely faster" is decided by measurement, never by assumption. For an
  interactive viewer, *latency* usually matters more than throughput.
- **The baseline is not Vello.** It is a multi-threaded `tiny-skia` — 5.9 ms for a
  dense text page at 1191×1684 — *including* the cost of getting the pixels to
  whoever asked for them. §6.2 calls a third of that a success and a tenth a clear
  win. A GPU path that loses to a CPU rasteriser is not a GPU path worth having.
- **Measure the frame, not the shader.** Between 55% and 92% of an offscreen frame in
  the current Vello-based backend is paid before any of the page is drawn (§6.1). The
  ranking that follows from those numbers — surface and texture targets first, then
  the per-pixel floor, then the atlas and the rectangle path, then the retained scene,
  then damage — is the brief's, and it reverses what intuition suggested.
- Perf gates run in CI with numbers attached: device creation, encode, execute,
  readback, and a frame on a real page at a real window size. A regression fails the
  build. Wall clocks lie under load; where a gate needs to be deterministic, count
  instructions or use timestamp queries rather than a stopwatch.

#### Startup is a first-class requirement

The caller renders page one on the CPU *while we initialise*, and takes our output
over once we are ready. A device that blocks a main thread for 200 ms is a design that
project cannot use even if every frame afterwards is free.

- **Few pipelines, compiled lazily.** Vello compiles about twenty compute shaders up
  front. Only the pipelines a page of text needs may be on the critical path;
  shadings, meshes and the exotic blend modes compile on first use.
- **`Device::headless` must be able to return before every pipeline exists**, with the
  rest compiled in the background and a way to ask whether the device is fully warm.
- **Constructible on a background thread, and not requiring one.**
- **A persistable pipeline cache**, with the rejection of a stale blob reported rather
  than silently swallowed.
- **Report what startup cost**, split into adapter enumeration, device creation and
  pipeline compilation, so a regression can be attributed rather than argued about.

### 3. Secure from the start

Our position in the process is different from the viewer's, and the rule follows the
position rather than the habit:

- **`#![forbid(unsafe_code)]` on every crate here.** Not because hostile bytes reach
  us — they do not — but because a memory-safety defect in a library linked into a PDF
  viewer is a defect in that viewer's security posture, and `wgpu` already provides a
  safe API over the driver. If a measured win ever requires `unsafe`, it is an ADR
  with a benchmark, a written invariant and a `// SAFETY:` comment, not a quiet
  `#[allow]`.
- **Data is not trusted just because a friend produced it.** Coordinates of 1e30,
  degenerate transforms, 40 000-element dash arrays and a 60 000×60 000 image all
  arrive from real files by way of a correct interpreter. §4.7's rule is ours: refuse
  them loudly, never produce NaN geometry, never allocate from an unchecked number.
- **Explicit memory and time budgets.** A GPU buffer sized from document-derived
  arithmetic is a decompression bomb with a different name. Every allocation derived
  from scene content is checked against a stated budget, and exceeding it is an error
  that names the limit.
- **Fuzz the scene boundary from the first commit.** The scene builder and the encoder
  take structured input from another process's parser; every crasher found becomes a
  permanent regression test.

### 4. Exemplary — a project others can learn from

- Architecture is legible: clear layer boundaries, no circular dependencies, each
  crate with one stated responsibility.
- Names say what things are. Comments say *why*, never *what*.
- Every non-obvious decision gets an ADR in `doc/adr/` — the reasoning matters as much
  as the result, and a decision whose cost is not written down has not been made.
- Prefer the clear construction over the clever one.
- A GPU renderer is where this principle is hardest and matters most: a shader is
  write-only code unless the invariant it relies on is stated beside it.

### 5. The specification is the only source of truth

Stated by the project owner, and absolute. In the consuming project's words:

> Never use the other libraries as source of truth. The truth is the spec only. If we
> have the same results as the other libraries, we can assume that we understood the
> spec correctly — but if not, we don't try to match what the others do, we find out
> what the spec says.

For us the specification is **ISO 32000-2**, clause 11 above all, plus the clauses
§4 of the brief names. Vello, Skia, `tiny-skia`, Cairo and every GPU renderer whose
source is on the internet are **evidence about our reading**, never the definition of
correct. Three of `tiny-skia`'s sixteen blend modes are wrong; one of them was wrong by
113 of 255. A renderer that agreed with it would have been wrong in exactly the same
place.

In practice:

- A test's expected value must be derivable from the specification, and its comment
  must say *from where*. "This is what the CPU backend produces" justifies nothing on
  its own — it is a cross-check between two readings, which is why a disagreement sends
  us to the clause rather than to the other implementation's source.
- **Every item implementing a normative requirement cites its clause** —
  `ISO 32000-2 §11.4.6` — in its doc comment, its module comment, or the comment above
  the block. A shader is not exempt; a WGSL blend function carries the clause number in
  a comment.
- **Quotation marks mean verbatim.** A load-bearing normative sentence goes in as a
  rustdoc blockquote, exact, under its clause number. Paraphrase is fine and often
  clearer; paraphrase that claims to be a quote is not.
- Curve-fitting to another renderer's output is forbidden outright. Tuning constants
  until a corpus matches produces neither correctness nor knowledge.
- Where the specification genuinely defines nothing, say so plainly, make a deliberate
  choice, and document it *as a choice*. And remember that "the specification defines
  nothing here" is itself a claim about the specification, and it decays — read the
  clauses around the subject before recording a silence.

### 6. A frame is drawn, or it is refused. There is no third state.

This is the brief's §5, it is the requirement the library it replaces fails, and it is
promoted to a principle here because everything about a GPU renderer's failure modes
pushes the other way.

> A backend may refuse a scene. It may not silently draw nothing.

- **Memory that grows, rather than a table of hand-picked constants.** Count then
  allocate, or a growable arena with a fence that reruns on overflow. Vello's working
  buffers are sized by constants whose own comment says they were "hand picked to
  accommodate the vello test scenes"; a page of ISO 32000-2 fitted to a laptop window
  needed 4% more tile records than the buffer held, and the result was a black page,
  an `Ok(())`, and a person having to report it because no test could see it.
- **If a limit must exist, it is discoverable before the frame** — through
  `Device::limits` and a `Scene::cost()` a caller can compare against it.
- **A failure is an `Err` that names what overflowed**, so the caller can fall back.
- **Anything we cannot draw as asked is a `Report`, not an approximation.** A gradient
  drawn opaque, a blend mode substituted, a mask ignored: each is a plausible-looking
  wrong page, which is the worst outcome either project has a name for. A hole and a
  sentence beat a plausible lie.
- **Whatever a `Frame` says about itself must be true.** A failed frame reported as a
  drawn one cost the viewer a session's debugging: the core recorded the page as shown,
  never asked again, and kept the previous page under a title bar naming the new one.
- **A blank scene is a legitimate scene**, and so is a zero-length buffer slice that
  follows from one.

### On the tension between 2 and 4

Speed and exemplary clarity partly conflict. They conflict less than they appear to,
and the resolution is a rule:

**An optimization must be justified by a benchmark and explained by a comment.**

Clean architecture at the top, optimized code in measured hot spots, with every
optimization carrying (a) the benchmark number that justifies it and (b) a comment
explaining what it buys and what it costs in readability. Optimized code that is
*explained* teaches more than naive code does. What is forbidden is unexplained
cleverness and speculative optimization of code nobody measured.

Where the conflict is real and unresolvable, clarity wins in cold paths, speed wins in
hot paths, and the choice is written down.

## Two rules about instrumentation, both learned in the caller's tree

- **A cost written down beside one call is not a cost anybody adds up.** If an
  operation is expensive, make it expensive *in the API* — a distinct type, a
  `#[must_use]`, a name with `slow` in it — not in a doc comment.
- **Instrument the count of distinct keys, not the hit rate.** A clip-mask cache
  answered all 303 lookups a page made and built 303 identical page-wide masks,
  because the key was a name rather than the region. A hit rate is a statement about
  the lookups you made, never about the ones you should have made.

## Stack

| Area | Choice |
|---|---|
| Language | Rust, edition 2024, toolchain pinned in `rust-toolchain.toml` |
| GPU | `wgpu` 30 (Vulkan on this machine; the other backends are free) |
| Shaders | WGSL, in-tree, one clause citation per blend function |
| Errors | `thiserror` |
| Async | none. `pollster` for wgpu's two awaits; a thread is not a runtime |
| Test artefacts | `png` (straight-alpha RGBA in, which is what §3 hands back) |

**Not used, by design:** any colour-management crate, any font or shaping crate, any
second 2D scene model. `deny.toml` enforces the list, because a non-goal that is only
written in prose is a non-goal that arrives as a transitive dependency.

## Working agreements

- You are running as your own user. Obviously not a real sandbox, but you do not need
  to ask before deleting files, and so on. You are not able to modify global config or
  install anything globally. Evaluate whether asking the human to install something
  globally or creating a user-local copy is the better choice.
- If a proposed fix looks wrong for this setup, say so instead of running it.
- Verify claims by running them. Report failures with their output; never assert that
  something works without having checked. For anything on the GPU this is not a
  formality: a wrong shader produces a picture, and a picture looks like a result.

## Environment notes

- Arch Linux. GPU: AMD Strix (Radeon 880M/890M, RDNA 3.5) — RADV. Session: X11.
- Claude Code may run as user `AI` via `sudo -u AI`, reaching this tree through the
  `coders` group. That user has no X authority cookie, so it cannot open a window on
  *the user's* display — but it can run headless on its own: `Xvfb` and `lavapipe` are
  installed, `xdotool` sends real key presses and `xwd` reads a window's pixels back.
  Hand a run on the real GPU to the user; everything else is testable here.
- **Two adapters, and the difference is a feature.** RADV and lavapipe produce
  byte-identical output from the current Vello-based backend, which is what lets the
  viewer's CI use a software rasteriser. Whether our design can promise the same is
  §11's question 4, and it is to be answered early because the answer changes how CI
  works.
- The sibling checkout at `/home/cl/projects/pdf-viewer` is the caller, the oracle and
  the source of every measurement quoted here. It is not a dependency of this
  workspace and this workspace is not a dependency of it — the two are wired together
  only when the viewer's `render-gpu` is replaced, which is the last milestone, not the
  first.
