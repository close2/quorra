# ADR 0042 — A pipeline that cannot be built is refused, not survived

Status: accepted, 2026-08-13. Fixes the trap `doc/HANDOVER.md` has carried since ADR
0018's neighbourhood: "a WGSL compile error hangs the test binary; it does not fail it."

## Context

Rename the local `at` in `blit.wgsl`'s `fs_main` back to `from` — a WGSL reserved
keyword — and `cargo test --release -p quorra-gpu --lib` produces **no output and never
ends**. The mechanism, measured rather than guessed, is a chain of four links, and only
the last one is the bug:

1. `Device::create_shader_module` cannot fail by return value. `wgpu` hands back a
   `ShaderModule` either way and reports the validation error out of band, to the
   device's uncaptured-error handler. That handler's default is `panic!`
   (`wgpu-30.0.0/src/backend/wgpu_core.rs:1084`).
2. The thread that was compiling is `quorra-warm-up`, because `PipelineStore::base`
   made all eight modules together on first need and the warm set is the first need.
   Nothing was listening: ADR 0018 gave the device the `JoinHandle` for the thread's
   *lifetime*, and joins it only in `Drop`, discarding the result.
3. `warm_up_now` therefore never reached `state.warm = true` and never called
   `self.warmed.notify_all()`. The store's `Mutex` was left poisoned, which changed
   nothing — `PipelineStore::lock` already absorbed poisoning with
   `PoisonError::into_inner`, deliberately and correctly.
4. **`wait_until_warm` then waited on a `Condvar` with no notifier left alive.** A
   `gdb` thread dump of the hung process is unambiguous: the test thread is inside
   `winding::tests::coverage` on a futex, the libtest main thread is blocked on
   `Channel<CompletedTest>::recv`, and `quorra-warm-up` is gone. Everything else in the
   process was healthy.

So the panic was loud and the *hang* was what made it invisible: nineteen of the
crate's test files call `wait_until_warm`, so the first one to reach it stopped the
harness before it could print a single result.

This is only reachable through our own invalid shader, which no scene can provoke. It
is still a library that stops answering, and §5's rule does not have a clause for
"unless the cause was ours".

## Decision

**Three changes, in the order they close the chain.**

**1. Every compile runs inside a `wgpu` validation error scope.** `pipeline::captured`
pushes `ErrorFilter::Validation`, creates the module or the pipeline, and pops the
scope — which is what turns a panic on an arbitrary thread into a value. The pop is
resolved by `pollster`; `wgpu`'s own backend pops synchronously and returns an
already-ready future, so nothing blocks and CLAUDE.md's "a thread is not a runtime"
position is unchanged.

**2. `PipelineStore::get` is fallible, and the failure is typed.** A captured error
becomes `error::PipelineProblem` — `Shader { shader, detail }` or
`Pipeline { pipeline, format, detail }`, naming the module or the pipeline label and,
for a pipeline, the colour format it was asked for. `RenderError::PipelineUnavailable`
carries it out to the caller, so a frame that needed the pass is **refused** rather than
drawn without it. With the reproduction in place the suite now ends in 1.17 s with five
named failures, each quoting the WGSL span:

> shader module 'quorra blit' was refused: … `Shader 'quorra blit' parsing error: name
> `from` is a reserved keyword`

The ripple is seven call sites and five functions in `compose.rs` and `winding.rs` that
gain a `Result` they mostly already had one caller deep. **The layout getters stay
infallible**, and that is what kept the ripple to that size: a bind-group or pipeline
layout is a description `wgpu` checks against nothing but itself, no WGSL reaches it,
and no adapter can refuse one. The split is now structural: `pipeline/layouts.rs` is
the half that cannot fail, `Modules` in `pipeline.rs` the half that can, and
`pipeline/spec.rs` — which came out of the same seam — is what each `Kind` is, leaving
`pipeline.rs` at 460 lines of the store's lock, laziness and warm-up rather than 779 of
all three.

**3. The warm-up records its outcome on every exit path, including an unwind.**
`WarmUpGuard`'s `Drop` writes the state and calls `notify_all` whatever happens, so
`wait_until_warm` returns even for a thread that left by panicking. The state it writes
is the new public `startup::WarmUp`: `Running`, `Warm(Duration)`, `Refused(problem)`,
`Abandoned`. `Device::warm_up()` reports it and `Device::is_warm()` is that question
narrowed to `Warm`.

## What it costs

**A public enum and a public accessor**, where there was one boolean. That is not
decoration: `is_warm` has two outcomes in which `false` is the *final* answer, and a
host polling it — which §7 invites, by asking for "a way to ask whether the device is
fully warm" — would otherwise spin forever on exactly the failure this ADR is about. A
boolean cannot say "never". `Device::wait_until_warm` keeps its signature and its
meaning is widened by one sentence: it returns when the warm-up has *finished*, and
`warm_up()` says how.

**A refused module is cached as refused.** A module a backend rejects is rejected
identically every time, so retrying per frame would cost the parse and answer nothing
new — and keeping it is what lets every later frame be refused in the same words. A
refused *pipeline* is not cached, because it is keyed by format and one bad format must
not take the others with it; `tests` asserts that directly.

**Two error scopes per compile, on a path that runs at most 17 × formats times per
device.** Pushing and popping a scope is a `Vec` push and pop on a thread-local under
`wgpu`'s error-sink lock. Not measured, and deliberately not: the warm set compiles in
~9 ms and this is two vector operations beside it, so a benchmark would be measuring the
clock.

**`WarmUp::Abandoned` is a state with no reason to give**, and it is honest about that.
An out-of-memory or an internal error is not a validation error, so it is *not* caught
by the scope in change 1 and still reaches the handler that panics. The guard bounds the
damage of that to "the warm set does not exist and nobody is told why", instead of a
process that stops.

## Alternatives

**`Device::on_uncaptured_error` instead of error scopes.** One handler for the whole
device, set at construction. It would catch the out-of-memory and internal errors the
scope does not — but it delivers them with no idea which call produced them, so the
error could not name the shader or the format, and the store would have to correlate a
handler callback with a compile in flight on another thread. A scope around the one call
that can fail gives a better error for less machinery.

**Make `wait_until_warm` fallible.** Louder, and it is the shape CLAUDE.md's
instrumentation rule usually prefers. Declined because the reason already reaches the
caller twice over — from `warm_up()` without blocking, and from the first `render` that
needs the pipeline — while a `Result` return would change the signature for the caller's
tree to say something `warm_up()` says better.

**Validate every shader module in `Device::build`.** It would turn this into a
`DeviceError` at construction, which is where a defect in our own shaders belongs. It
also puts eight WGSL parses on the blocking startup path, which is precisely what §7 and
`PipelineStore`'s whole design exist to keep off it. Refused.

**Leave the panic and document it.** The panic was never the problem; nobody could see
it. And a doc comment carrying a correctness obligation is the shape CLAUDE.md names
outright.

## Tests

`pipeline.rs`'s test module states the property that failed rather than the reproduction
that found it, because the reproduction cannot live in the tree:

- **a warm-up that panics still releases its waiters** — a `WarmUpGuard` on a thread
  that panics, then `wait_until_warm` on another. Against the unfixed store this hangs,
  which is why the wait runs on a thread and is given up on after 30 s: a test that can
  hang CI is worse than the bug it guards. The failure is an `assert!`, not a timeout in
  the harness.
- **a running warm-up is reported as running**, so "it returns" is not passing by
  returning always.
- **a pipeline the adapter refuses is an error and not a panic**, and **a refusal names
  the pipeline and the format**. `Rgba8Snorm` is the instrument: WebGPU gives it no
  `RENDER_ATTACHMENT` usage, so a colour target in that format is a validation error
  every backend agrees on — reachable without shipping a shader that does not parse. The
  first of the two also asserts that a later `get` for a format that *is* renderable
  still succeeds.

## Revisit when

`wgpu` gives `create_shader_module` a fallible form, which would make `captured`
unnecessary for change 1 and would let the scope narrow to pipeline creation. Or when
something needs `WarmUp::Abandoned` to carry a reason — which means catching the panic,
and that is a different decision with a different cost.
