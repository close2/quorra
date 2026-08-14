//! A global allocator that counts target-sized allocations on the calling thread.
//!
//! One responsibility: answer *how many buffers of consequence did this block of code
//! allocate, and how many bytes were they*. It is the instrument
//! `tests/readback_cost.rs` gates on, and it exists because that claim is a count and
//! not a duration (ADR 0052).
//!
//! # The one `unsafe` in this tree, and the terms it is here on
//!
//! CLAUDE.md principle 3 puts `#![forbid(unsafe_code)]` on every crate here, and names
//! the escape hatch this module uses: an ADR, a written invariant, and a `// SAFETY:`
//! comment rather than a quiet `#[allow]`. The terms:
//!
//! - **`GlobalAlloc` cannot be implemented safely.** The trait is `unsafe` by
//!   definition, so there is no version of this instrument without it. The alternative
//!   was a hand-maintained byte tally inside `readback.rs`, which is exactly the shape
//!   CLAUDE.md's first instrumentation rule rejects: a cost written down beside one call
//!   is not a cost anybody adds up, and the next staging buffer would not be added to it.
//!   An allocator sees the allocation whether or not anyone remembered.
//! - **It is a test crate.** An integration test is its own crate and is never linked
//!   into the library, so nothing here reaches the PDF viewer whose security posture
//!   principle 3 is about.
//! - **The invariant, stated once and checkable by reading forty lines:** every method
//!   forwards to [`System`] with its arguments unchanged and returns what `System`
//!   returned. The additions are three thread-local `Cell<usize>` updates on the way
//!   past. This allocator therefore has exactly `System`'s memory behaviour, and a
//!   defect in the counting can make a *test* wrong but cannot make a *program* unsound.
//!
//! # Why thread-local, and not a global counter
//!
//! A software adapter rasterises on worker threads that allocate buffers of their own —
//! llvmpipe is pinned by name in much of this suite, so that is not hypothetical. A
//! global counter would report those, and the count would then depend on the adapter and
//! on how many cores it decided to use. Counting only the thread that opened the
//! [`Watch`] makes the number a property of the code under test. The cells are
//! `const`-initialised, so touching them inside `alloc` cannot itself allocate and
//! recurse.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// The size at or above which an allocation is *of consequence*.
///
/// A megabyte: a frame's bookkeeping — command vectors, instance buffers, the hash maps
/// — is orders below it, and a full page target at any scale this library draws is
/// orders above. The threshold exists so the count is stable against an allocator's
/// growth policy for small vectors, which is not what any gate here is about.
const TARGET_SIZED: usize = 1 << 20;

thread_local! {
    /// Whether this thread is inside a [`Watch`].
    static WATCHING: Cell<bool> = const { Cell::new(false) };
    /// Allocations of at least [`TARGET_SIZED`] since the watch opened.
    static COUNT: Cell<usize> = const { Cell::new(0) };
    /// Their total size in bytes.
    static BYTES: Cell<usize> = const { Cell::new(0) };
}

/// Record an allocation of `size` if this thread is watching.
///
/// `try_with` rather than `with`: a thread tearing down its locals can still allocate,
/// and a panic in the allocator is not recoverable. A dead cell means "not watching",
/// which is the truth at that point in a thread's life.
fn note(size: usize) {
    if size < TARGET_SIZED {
        return;
    }
    let watching = WATCHING.try_with(Cell::get).unwrap_or(false);
    if !watching {
        return;
    }
    let _ = COUNT.try_with(|c| c.set(c.get().saturating_add(1)));
    let _ = BYTES.try_with(|b| b.set(b.get().saturating_add(size)));
}

/// [`System`], plus a count of the target-sized allocations passing through it.
struct Counting;

// SAFETY: every method forwards to `System` with its arguments unchanged and returns
// `System`'s result unchanged, so this allocator's memory behaviour *is* `System`'s.
// `note` runs before the forward, touches only `const`-initialised thread-local `Cell`s
// (so it cannot allocate and recurse) and cannot unwind (`try_with` swallows the one
// failure mode, and `saturating_add` cannot overflow).
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        // SAFETY: `layout` is forwarded exactly as the caller built it.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        // SAFETY: `layout` is forwarded exactly as the caller built it.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` and `layout` are forwarded exactly as the caller supplied them,
        // and `ptr` came from this allocator, which is `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // The grown size is what the program now holds, so it is what a reallocation
        // costs. A `Vec` that reaches target size by doubling is counted where it
        // crosses, which is the event this instrument is about.
        note(new_size);
        // SAFETY: all three arguments are forwarded exactly as the caller supplied them.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// An open counting window on the current thread. [`Watch::finish`] closes it and
/// answers what happened inside.
///
/// Not `Send`: the counters are the opening thread's, and finishing on another one
/// would read cells nothing wrote. `PhantomData<*const ()>` is what says so.
#[derive(Debug)]
pub(crate) struct Watch {
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Watch {
    /// Start counting this thread's target-sized allocations, from zero.
    pub(crate) fn start() -> Self {
        COUNT.set(0);
        BYTES.set(0);
        WATCHING.set(true);
        Self {
            _not_send: std::marker::PhantomData,
        }
    }

    /// Stop counting, and answer `(allocations, bytes)`.
    ///
    /// Takes `self` by value because closing the window *is* consuming the handle: the
    /// destructuring below is what says so to the compiler, and it is why there is no
    /// second reading of a watch that has already been read.
    pub(crate) fn finish(self) -> (usize, usize) {
        let Self { _not_send } = self;
        WATCHING.set(false);
        (COUNT.get(), BYTES.get())
    }
}
