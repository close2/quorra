# ADR 0017 — The backend set is the host's to name, and only by argument

Status: accepted, 2026-08-07. Decided from the caller's feedback §12, which is a
request rather than a defect report. Supersedes ADR 0014 §3 in part: that ADR declined
a backend knob *as a startup optimisation*, and that reasoning stands unchanged.

## Context

The caller's project owner ran their viewer on a Windows machine with Intel graphics
and **it crashed inside the Vulkan driver**. The crash is nobody's code in either
tree. What makes it ours is that the machine has a second driver stack — DX12 — and
nothing in this library could ask for it:

- `create_instance()` built `InstanceDescriptor::new_without_display_handle()`, whose
  `backends` is `Backends::default()` = `Backends::all()`, and did not call
  `.with_env()`. No parameter, no second entry point, no environment.
- `Options::adapter` is not a way round it. `select_adapter` filters on a
  case-insensitive substring of `get_info().name`, and one GPU is enumerated **once per
  backend that can drive it**, under the *device's* name each time. On a machine with
  one Intel GPU, `"Intel"` matches the Vulkan adapter and the DX12 adapter equally. The
  filter selects hardware; the question here is which driver stack talks to it.
- With no filter, `request_adapter` with `PowerPreference::HighPerformance` breaks ties
  among adapters of equal device type in wgpu's hub order, where **Vulkan precedes
  DX12**. That is how the machine above reached the driver that crashed it.

`HighPerformance` is right and is not what changes. What changes is that the set it
chooses from becomes something a host can state.

## Decision

### 1. One parameter, at the instance

```rust
pub fn create_instance_with(backends: wgpu::Backends) -> wgpu::Instance;
```

`create_instance()` keeps its exact meaning and becomes `create_instance_with(
Backends::all())`. A host told by its user to use DX12 passes `Backends::DX12`; a host
that was not told anything calls the function it already calls. `Backends` is already
in our public surface through the `wgpu` re-export, so this adds no type.

**At the instance, not in `Options`**, for two reasons that agree. The mechanical one:
backends are an instance-level choice and the instance is constructed before an
`Options` exists — `Device::headless` builds one *from* the options it is given, so a
field there could only apply to instances we make for ourselves. The stated one: ADR
0014 §2 already refused to put instance-shaped things in `Options`, which is a plain
value a host may clone, store and compare.

The consequence a host should know: choosing backends means choosing the
`*_with_instance` constructors, and `StartupTimings::instance_creation` is then `None`
because the step was theirs. That is ADR 0014 §1's rule, unchanged and working.

### 2. The environment is not read — the argument is the only route

`wgpu-types` offers `Backends::from_env()` and `InstanceDescriptor::with_env()`, and we
call neither. The caller had no preference and asked that it be decided rather than
defaulted; this is the decision, and the reason is the same one §4.6 gives for
determinism: **a frame's inputs should be the arguments the host passed, not the
environment the process happened to inherit.** A library that renders through a
different driver because a variable was exported has a failure mode that reproduces
nowhere and is diagnosed by nobody.

This costs the person debugging a driver the habit that every other wgpu program on
the machine honours `WGPU_BACKEND`, which is a real cost. It is bounded by one line in
the host, which we document at `create_instance_with`, and which keeps the host's own
command line above the environment rather than under it:

```rust
let instance = match from_flag.or_else(wgpu::Backends::from_env) {
    Some(backends) => create_instance_with(backends),
    None => create_instance(),
};
```

The caller said the one thing they did not want was the environment being the *only*
route. Here it is not a route at all, and the flag it asked for is one call.

### 3. `Device::adapter_names_on`, so a host cannot offer what it cannot honour

`Device::adapter_names()` makes its own all-backends instance. A host that restricted
its instance and then listed adapters with it would offer a choice its own constructors
could not honour — the new parameter would have created that trap. `adapter_names_on(
&instance)` answers for the instance the host will actually build from. `adapter_names`
stays, unchanged, for the cross-adapter gate (§4.6, §11.4) that genuinely wants every
adapter on the machine.

## What it cost

Two added functions and no changed behaviour: `create_instance()` produces the
descriptor it always produced, which `tests/backend_choice.rs` holds by comparing its
enumeration against `create_instance_with(Backends::all())`.

`Backends::all()` includes `Backends::NOOP`, which reads alarmingly and is inert: the
noop backend is compiled only under `wgpu`'s `noop` feature and refuses to initialise
unless `NoopBackendOptions::enable` is set, neither of which is true here. It has never
been in the set that could be chosen; naming `all()` explicitly does not put it there.

**Naming a backend set this machine cannot supply is not an error at the instance.** It
becomes `DeviceError::NoAdapter` at the constructor, with an empty `available` list —
and on a machine with a visible GPU that emptiness is the signature of this mistake
rather than of a broken driver, which is why the doc comment says so.

## The measurement, and what could not be measured

Nothing here is a speed claim: ADR 0014 §3's numbers still say restricting the instance
to Vulkan halves `Instance::new` and gives every millisecond back in `request_adapter`.
This is an escape hatch from a driver, and the doc comment says that so no one reads it
as a knob to tune.

**No machine in this project runs Windows, and neither adapter here is reachable
through DX12**, so the mechanism is exercised with the backends this machine does
have: Vulkan against everything (`tests/backend_choice.rs` asserts the device it builds
is a Vulkan one) and the empty set against nothing (the typed refusal, not a panic and
not a silent fallback onto the backend the host was avoiding). Whether the Intel
Vulkan driver's crash goes away under DX12 is the project owner's machine to answer;
what this ADR is certain of is that until now they could not ask the question.

## Revisit when

A host needs to name a backend *and* keep `instance_creation` attributed to us — that
would mean a constructor taking backends directly, and the honest answer today is that
the host timing `create_instance_with` itself is one `Instant::now()`. Or when a
platform arrives where the backend cannot be decided before the window exists, which
would break the "instance-level choice" premise this ADR rests on.
