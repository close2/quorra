# ADR 0013 — No pipeline-cache blob, and no `unsafe` exception for one

Status: accepted, 2026-08-02. Decided at M8, as the plan required.

## Context

§7 of the brief asks for "a persistable pipeline cache, with the rejection of a
stale blob reported rather than silently swallowed". In `wgpu` 30 that is
`Device::create_pipeline_cache`, an **`unsafe fn`** — the blob is trusted input the
driver may crash on — and every crate in this tree is `#![forbid(unsafe_code)]`
(CLAUDE.md principle 3). The principle allows an exception, but only as "an ADR
with a benchmark, a written invariant and a `// SAFETY:` comment". This is that
ADR, and the benchmark decides against the exception.

## Decision

**Do not offer the cache. Keep `forbid(unsafe_code)` intact.**

The benchmark (release, this machine, `examples/floor.rs`): device construction is
adapter enumeration **19.9 ms** + device creation **11.9 ms**, both untouchable by
a pipeline cache. The warm pipeline set compiles in **9.4 ms on a background
thread** that `Device::headless` never waits for — the caller renders page one on
its CPU backend during exactly this window, by design (§7). Every remaining
pipeline compiles on first use at a measured few milliseconds, once, attributed in
that frame's `Timings::phases`. A cache blob would shave milliseconds off a
background thread nobody blocks on: there is no user-visible number for the
benchmark column, so principle 3's bar for `unsafe` is not met — the exception
would trade a memory-safety guarantee for nothing measurable.

RADV also builds its own shader cache below us (`~/.cache/mesa_shader_cache`), so
second launches already skip the expensive half of compilation without our help —
part of why the warm set is this cheap.

## Revisit when

Any of: a backend where PSO compilation is user-visible (DX12 and Metal are the
usual offenders, neither is a target today); the shader set growing past "few"
(this design holds at nine kinds; Vello's twenty-plus is the counter-example the
brief cites); or a measured startup regression attributing real latency to
compilation. The trigger is a *measurement*, and the route is this ADR's successor
with the benchmark that meets principle 3's bar.

## What holds it

`Device::startup` reports the three-way split (§7), so the "nothing measurable"
claim stays continuously checkable; `deny.toml` and `#![forbid(unsafe_code)]`
enforce the decision itself.
