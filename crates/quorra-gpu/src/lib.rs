//! The device: pixels, and the truth about how they were made.
//!
//! This crate turns a [`Scene`] plus a viewport into a frame, on a GPU, through `wgpu`.
//! It is the half of quorra that knows about resolution — and the scene crate beside it
//! is the half that must never learn (`doc/adr/0001`).
//!
//! [`Scene`]: ../quorra_scene/scene/index.html
//!
//! # The two rules that outrank everything else here
//!
//! **A frame is drawn, or it is refused. There is no third state.** The library this one
//! replaces sizes its GPU working buffers from a table of constants whose own comment says
//! they were "hand picked to accommodate the vello test scenes"; a scene needing more
//! overflows them *on the device*, which sets a flag, stops filling, and returns `Ok(())`
//! over a blank target. A page of ISO 32000-2 fitted to a laptop window is such a scene —
//! it needed 4% more tile records than the buffer held. A person reported a black page and
//! nothing in the test suite could see it. So: memory that grows, limits that are
//! discoverable before the frame, and failures that are an `Err` naming what overflowed.
//!
//! **Whatever a [`Frame`] says about itself must be true.** A window in the caller's tree
//! once answered "presented" when its GPU path had refused the page, so the core recorded
//! the page as shown, never asked again, and kept the *previous* page under a title bar
//! naming the new one.
//!
//! [`Frame`]: frame
//!
//! # State
//!
//! Skeleton: every module below states what it owns, what it must never do, and the
//! signatures it will hold. No module contains code yet, and there is deliberately no
//! empty `Device` struct standing in for one — see `doc/adr/0003`. M1 in `doc/PLAN.md` is
//! a device, a rectangle, and the timestamp queries that answer §11's first question.

#![forbid(unsafe_code)]

pub mod atlas;
pub mod device;
pub mod frame;
pub mod mask;
pub mod pipeline;
pub mod report;
pub mod target;
pub mod viewport;
