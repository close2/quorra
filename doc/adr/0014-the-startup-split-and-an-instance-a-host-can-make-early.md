# ADR 0014 — The startup split, and an instance a host can make early

Status: accepted, 2026-08-04. Decided from the caller's feedback §8, which is a
request rather than a defect report.

## Context

The caller's project owner decided that **page one goes to the graphics device** —
no CPU first frame, no probe, no `wait_until_warm` — which moved our bring-up onto
their time-to-first-page. They then measured the whole launch as a timeline (their
ADR 0179) and found bring-up is **31% of it**: 45.1 ms of a 144.6 ms launch under
Xvfb with lavapipe. §7 of the brief always said startup was a first-class
requirement; this is the first time a number a person waits through is made of it.

Two things they asked for, and one they explicitly did not.

**The number they had to reason with named one step and measured three.**
`Device::build` took its clock from before `wgpu::Instance::new` in both
constructors, so `StartupTimings::adapter_enumeration` was instance creation *plus*
surface creation *plus* adapter selection. Those have different causes — the driver
loader, the window system, physical-device enumeration — so a host watching that one
number for a regression could not say which had moved. Measuring `wgpu` directly,
one configuration per process, they saw the single figure split roughly two-to-three
between two of them.

**Instance creation needs no window, and only we could let them overlap it.** It
needs no surface and no event loop either, so it can run on a thread started at
`main`'s first line while the document is read and the window opened. It could not,
because it happened inside `Device::for_surface`, which needs the window. They
measured the overlap at about 20 ms of their 145 ms launch.

## Decision

### 1. One field per step that can regress on its own

`StartupTimings` becomes five numbers: `instance_creation`, `surface_creation`,
`adapter_selection`, `device_creation`, `pipeline_compilation`. The first four are
what the constructor blocked for, in the order they happen, and
`blocking_total()` sums exactly those — pipeline compilation is excluded because no
constructor waits for it, and a caller that chooses to wait is adding a cost rather
than discovering one.

**`instance_creation` is an `Option<Duration>`; `surface_creation` is a plain
`Duration`.** The asymmetry is the point and it is the honest reading of two
different silences:

- A headless device performs no surface creation. The step did not happen, its cost
  really is zero, and `Duration::ZERO` says so.
- A device built from a supplied instance did not *perform* instance creation, but
  the step certainly happened — on someone else's thread, on someone else's clock.
  Reporting zero would be a number that lies about what it measured, in a struct
  whose entire purpose is attribution. `None` says "not mine to report", and the
  host that made the instance is the one holding its stopwatch.

### 2. An entry point per constructor, not a handle in `Options`

`startup::create_instance()` returns the instance quorra would have made for itself;
`Device::headless_with_instance` and `Device::for_surface_with_instance` take one.
The caller offered `Options::instance: Option<wgpu::Instance>` as an alternative and
we declined it: `Options` is a plain value a host may clone, store and compare, and
putting a live GPU resource inside it makes every clone share one — a lifetime
surprise in a struct that reads like configuration. A named constructor puts the
sharing in the signature, which is the rule about making a cost visible in the API
rather than in a doc comment.

`create_instance` exists rather than letting hosts call `wgpu::Instance::new`
themselves so that the descriptor stays ours: surface creation is only guaranteed
against an instance built the way our own constructors build one, and a host that
guessed the descriptor would find out at `create_surface`.

**What is not claimed:** `request_adapter` takes the surface as
`compatible_surface`, so adapter selection is genuinely downstream of the window and
cannot be hoisted. The honest claim is the instance's share, not bring-up's.

### 3. No backend knob in `Options`

The obvious guess — `Backends::all()` loads the GL backend for nothing on a Vulkan
machine — is wrong here, and the caller measured it rather than arguing it:
restricting the instance to Vulkan halves `Instance::new` (21–32 ms → 9–16 ms) and
gives every millisecond of it back in `request_adapter` (34–36 ms → 39–43 ms). The
total is the invariant. So the knob is **not added**, and this paragraph is the
record that the silence is deliberate — the measurement exists, and it says there is
nothing to win.

## What it cost

A breaking change to a public struct, at 0.1.0, with one consumer who asked for it.
`adapter_enumeration` is gone rather than deprecated: keeping a field whose name is
known to misdescribe its contents would preserve exactly the defect this ADR fixes.
The caller's `QuorraPresenter::startup` forwards the struct whole, so their change is
in the one place that prints it.

## The measurement, this tree, this machine

`examples/startup.rs`, release, headless, **one device per process** — a second
`wgpu::Instance` in a process finds the driver loader warm and reports a fraction of
the true cost, which is the mistake the caller's first bring-up harness made (26.0 ms
against 4.4 ms for the same work in the other order). The example measures one
configuration and exits for that reason; a sweep is a shell loop.

Five processes per configuration (three for llvmpipe), min–max:

| configuration | instance | adapter selection | device | blocking total |
|---|---|---|---|---|
| RADV, `Options::default` | 22.9–29.8 ms | 3.2–4.4 ms | 1.9–2.0 ms | 29.4–36.1 ms |
| RADV, adapter filter `"RADV"` | 23.3–35.7 ms | 3.5–6.2 ms | 1.8–2.8 ms | 30.0–41.0 ms |
| llvmpipe, adapter filter | 26.6–27.9 ms | 4.0–5.0 ms | 8.5–11.2 ms | 39.4–42.8 ms |
| RADV, **hoisted instance** | — | 3.3–6.9 ms | 1.7–2.2 ms | **5.1–9.2 ms** |

**Instance creation is roughly 80% of what bring-up blocks for on this machine**, and
a host that creates it in parallel with reading its document is left with 5–9 ms.
That is a larger share than the caller's ~20 ms estimate, measured on a different
path: theirs is surface-attached under Xvfb, where `request_adapter` with a
`compatible_surface` costs an order of magnitude more than our headless selection
does. Both numbers are right about their own configuration — which is the argument
for splitting the field, made by the field itself.

Two side findings, recorded because both were guesses worth killing:

- **The adapter filter costs about the same as `request_adapter`.** Enumerating every
  adapter and matching a substring might have been expected to cost more than letting
  wgpu choose; across five processes each the two overlap (3.2–4.4 ms against
  3.5–6.2 ms) and the run-to-run spread is larger than the difference. A single
  earlier reading suggested otherwise and was an outlier — five processes per
  configuration is what this table is for.
- **ADR 0013's benchmark is re-attributed, not overturned.** Its "adapter enumeration
  19.9 ms + device creation 11.9 ms" was the pre-split figure: most of that 19.9 ms
  was the driver loader. A pipeline cache still cannot touch either number, so that
  ADR's decision stands unchanged.

## Revisit when

A host reports a step that still cannot be attributed — the candidates are inside
`request_device` (queue creation versus feature negotiation) and inside surface
creation on a backend that does real work there. The trigger is a host holding a
timeline it cannot explain with these five numbers, which is exactly how this ADR
started.
