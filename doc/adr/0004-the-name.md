# ADR 0004 — The name

Status: accepted, 2026-08-02.

## Context

The library needed a name, and a name is cheap to choose and expensive to change: it is
in every module path, every manifest, the repository and the caller's dependency list.
Three things had to be true of it. It must not already be a crate — `crates.io` names are
first-come and unrenameable. It must not be obscene or absurd in another language, since
the caller's own project is developed in a German-speaking environment and the library is
public. And it should mean something about the work, because a name that teaches is worth
more than a name that merely identifies.

`folio` was the first candidate and is taken on `crates.io`. `bartleby` — the literary
patron saint of principled refusal, which is exactly §5 of the brief — is taken.
`rinzler` and `korben` are taken. `tuttle`, from *Brazil*'s bureaucracy of documents, was
rejected on the second criterion: *Tuttl* is Bavarian slang for breasts, which is the
kind of thing that is only funny once. `montag`, from *Fahrenheit 451*, is free and fits
— he is a man who becomes a book — but it reads as "Monday" to a German speaker.

## Decision

**quorra**, after Tron: Legacy's Quorra, the last surviving ISO. The library implements
ISO 32000-2, on a grid, on a GPU.

Checked before adoption: no crate of that name on `crates.io` (the API returns 404 for
`quorra`, and a name search returns nothing); no collision with an existing graphics or
document library; no obscene or unfortunate reading found in the languages checked — the
name-origin references give it "heart" or "dawn" by association with *cuore* and
*Aurora*.

## Consequences

Crates are `quorra`, `quorra-scene` and `quorra-gpu`, and the module paths read
`quorra::Device`, `quorra_scene::BlendMode`.

The name is a character in a Disney film, which is worth stating plainly: character names
are not generally protectable as trademarks outside the classes they are used in, and a
Rust rendering library is not a class Disney trades in, so the risk is low rather than
zero. If it ever needs to be non-zero-free, `qorra` is the one-character retreat and was
kept in reserve for that reason.

The pun is load-bearing in one direction only. It explains what the library renders to
someone who knows the film, and to everyone else it is a short, pronounceable name with
no other meaning in this field — which is the property that actually matters.
