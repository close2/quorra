//! **Knowing that the window shows what was just presented — a criterion, not a clock.**
//!
//! # What synchronisation exists between a present and a readable window: none
//!
//! `Presenter::present` submits a pass and calls `Queue::present`, which returns as soon
//! as the image is *queued*. Between that return and the moment `xwd` can read the pixels
//! back stand, on this machine, four things — and not one of them has an API this example
//! can wait on:
//!
//! 1. **wgpu.** `SurfaceTexture::present` returns `()`. There is no fence, no callback and
//!    no query; `Queue::on_submitted_work_done` answers a different question — that the
//!    *device* finished the work, not that anything reached a window. Vulkan does have an
//!    answer, `VK_KHR_present_wait`'s `vkWaitForPresentKHR`, and wgpu 30 exposes none of
//!    it. `src/surface.rs` asks for `desired_maximum_frame_latency: 2`, so two presents
//!    may be in flight before the third blocks.
//! 2. **The X Present extension.** `PresentCompleteNotify` says exactly what is wanted —
//!    and it is delivered on the X connection that issued the present, which is the one
//!    `wgpu` opened inside itself. This example's connection is winit's; it never sees
//!    that event, and an `XSync` on it orders nothing against another client's requests.
//! 3. **`XWayland`.** The X server here is not what scans out. It turns the presented pixmap
//!    into a Wayland surface commit, which the compositor takes at its own cadence.
//! 4. **The compositor.** It composites when it composites.
//!
//! So the honest statement is that **no synchronisation is available at this seam** — not
//! "none was looked for". Under `Xvfb` stages 2 to 4 do not exist and a 300 ms wall clock
//! was always enough; through a real compositor at load average 25 it was not, and
//! `doc/notes-present-rate.md` §4 has the capture that read one picture behind. A
//! convergence criterion is therefore not a workaround for a missing wait. It is the
//! instrument.
//!
//! # The criterion, and why it is not "retry until it passes"
//!
//! It terminates on a property of the captures rather than on a stopwatch:
//!
//! > **Present until two consecutive captures agree, and what they agree on is not what
//! > the window was last *proven* to show.**
//!
//! Both halves are load-bearing and the second is the one that is easy to leave out.
//! "Two consecutive captures agree" alone converges on a **stale** window: while the new
//! picture is in flight every capture reads the old one and every pair of them agrees —
//! which is the defect this replaces, made permanent. So the criterion carries the
//! window's last proven contents and refuses to settle on them.
//!
//! That makes this a **chain**, and a chain needs a first link. It is [`Settle::erased`]:
//! a present carrying no layers leaves the presenter's own clear (ADR 0056;
//! `src/present/pass.rs` loads the swapchain image with `Clear(TRANSPARENT)`), and "all of
//! the window is the clear" is a state named by the *library* rather than by a previous
//! capture. A stale capture during an erase reads the picture, which is not the clear, so
//! an erase cannot converge early either.
//!
//! **It is not "retry until it passes"** — ADR 0052's gate that cannot fail for its own
//! regression. Nothing here knows what the picture should look like: the criterion is
//! satisfied by any picture that is stably not the previous one, so a window that settles
//! on the *wrong* picture settles immediately and fails the assertions that follow. That
//! is the division of labour — this module answers "is the window showing the present I
//! issued", `main` answers "is that present right", and neither can quietly do the
//! other's job.
//!
//! # What it cannot see, stated rather than hidden
//!
//! Two consecutive agreeing captures of a torn or half-composited window would be
//! accepted. Captures are separated by one present, which under `Fifo` is at least one
//! refresh, plus an `xwd` process and its round trip — so such a state would have to
//! survive more than a refresh with presents still arriving, at which point it *is* the
//! window's contents rather than a tear. If it ever happens the assertions in `main`
//! report it as a wrong picture, which is a hole and a sentence rather than a plausible
//! lie.

use crate::Display;
use crate::xwd::{self, Shot};

/// How many capture rounds a settle is allowed before it gives up.
///
/// **A count, not a duration** (ADR 0052). It is a ceiling a healthy settle never
/// approaches — two or three rounds — rather than a wait that is always paid, so it wants
/// to be far above what the pipeline can hold and its exact value then costs nothing:
///
/// - `src/surface.rs` asks for `desired_maximum_frame_latency: 2`, so at most two presents
///   are in flight before the next one blocks;
/// - `XWayland` holds one pending presentation per window;
/// - the compositor adds a frame of its own.
///
/// That is about **five refreshes** between a present and a readable window. Sixty-four is
/// an order of magnitude above it, and since each round carries one present the bound is
/// also at least 64 refreshes — 533 ms at the 119.96 Hz of `doc/notes-present-rate.md` —
/// *plus* 64 `xwd` round trips. A window that has not settled by then is not slow; it is
/// not settling.
const ROUNDS: usize = 64;

/// The colour an opaque X visual shows the presenter's transparent clear as.
const CLEAR: [u8; 3] = [0, 0, 0];

/// How close to [`CLEAR`] a pixel must be for the window to count as erased.
///
/// ADR 0006's store-conversion bound, the same one [`crate::same`] applies and for the
/// same reason: the clear is stated as transparent and read back through a visual. It is
/// used **only** for the erase's terminal state. Every other comparison here is between
/// two captures of one window and is exact.
const CLEAR_TOLERANCE: u8 = 2;

/// What a settle is waiting for the window to become.
#[derive(Clone, Copy)]
enum Wanted<'a> {
    /// The presenter's own clear, everywhere: the chain's first link, and the one state
    /// this file can recognise without a proven capture before it.
    Clear,
    /// Anything other than what the window was last proven to show.
    ChangedFrom(&'a Shot),
}

/// What one round concluded, from three captures and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Round {
    /// The window still shows what it showed before. Present again.
    NotYet,
    /// It changed, but the last two captures do not agree yet. Capture again.
    Unstable,
    /// Two consecutive captures agree on something new: the present is on the window.
    Landed,
}

/// The criterion, over three captures. `previous` is `None` on the first round.
///
/// Pure and total, which is what lets [`the_criterion_refuses_a_stale_window`] state it
/// where there is no window.
fn judge(wanted: Wanted<'_>, previous: Option<&Shot>, current: &Shot) -> Round {
    let arrived = match wanted {
        Wanted::Clear => current.is_uniform(CLEAR, CLEAR_TOLERANCE),
        Wanted::ChangedFrom(known) => current != known,
    };
    if !arrived {
        Round::NotYet
    } else if previous == Some(current) {
        Round::Landed
    } else {
        Round::Unstable
    }
}

/// Why a settle gave up. Both arms name the count they gave up after, because the bound
/// is a count.
#[derive(Debug, thiserror::Error)]
pub(crate) enum NotSettled {
    /// Every capture read what the window showed before: the picture never arrived.
    #[error(
        "the window never showed {what}: {rounds} presents, {rounds} captures, and every \
         one of them read what the window showed before ({difference})"
    )]
    Unchanged {
        /// What was being presented, as the caller named it.
        what: &'static str,
        /// Rounds spent, which is [`ROUNDS`].
        rounds: usize,
        /// How the last capture compared with what the settle was waiting to leave.
        difference: String,
    },
    /// The window changed, but no two consecutive captures ever agreed on the change.
    #[error(
        "the window never held still on {what}: {rounds} captures, and no two consecutive \
         ones agreed on anything new; the last two differ by {difference}"
    )]
    Unstable {
        /// What was being presented, as the caller named it.
        what: &'static str,
        /// Rounds spent, which is [`ROUNDS`].
        rounds: usize,
        /// How the last two captures differ.
        difference: String,
    },
}

/// The window's last **proven** contents, and the criterion that keeps that true.
///
/// Every capture it hands back has been shown to be the window as it is rather than as it
/// was, and becomes the baseline the next settle must differ from.
pub(crate) struct Settle {
    /// What the window was last proven to show.
    known: Shot,
}

impl Settle {
    /// Erase the window and prove the erase landed: the chain's first link.
    ///
    /// `present_nothing` must present an empty slice. The window may already be the clear
    /// — a freshly mapped one usually is — in which case this converges on its first two
    /// captures and has still proven what it claims: the criterion states the window's
    /// contents, not who put them there.
    ///
    /// # Panics
    ///
    /// When the window never reads as the clear within [`ROUNDS`] rounds, which means the
    /// presenter is not reaching the window at all.
    pub(crate) fn erased(display: &mut Display, present_nothing: impl FnMut()) -> Self {
        let known = run(
            display,
            "the presenter's own clear",
            Wanted::Clear,
            present_nothing,
        )
        .unwrap_or_else(|why| panic!("{why}"));
        Self { known }
    }

    /// Present until the window is **proven** to show it, and hand back that capture.
    ///
    /// `what` names the picture for the message a failure carries. `present` is called
    /// once per round and must present the same layers every time, because what the
    /// criterion waits for is the window holding still.
    ///
    /// # Errors
    ///
    /// [`NotSettled`] when the window never changed from its proven contents, or never
    /// held still, within [`ROUNDS`] rounds.
    pub(crate) fn converge(
        &mut self,
        display: &mut Display,
        what: &'static str,
        present: impl FnMut(),
    ) -> Result<&Shot, NotSettled> {
        let was = self.known.clone();
        self.known = run(display, what, Wanted::ChangedFrom(&was), present)?;
        Ok(&self.known)
    }
}

/// The loop both entry points share: present, pump, capture, judge.
fn run(
    display: &mut Display,
    what: &'static str,
    wanted: Wanted<'_>,
    mut present: impl FnMut(),
) -> Result<Shot, NotSettled> {
    let mut previous: Option<Shot> = None;
    let mut round = Round::NotYet;
    for taken in 1..=ROUNDS {
        present();
        display.pump();
        let current = xwd::capture(crate::TITLE);
        round = judge(wanted, previous.as_ref(), &current);
        if round == Round::Landed {
            // Printed rather than only counted, because it is the one number that says
            // how much headroom [`ROUNDS`] actually has on this machine — a bound nobody
            // ever sees the distance to is a bound nobody can defend.
            eprintln!("settled on {what} after {taken} captures");
            return Ok(current);
        }
        previous = Some(current);
    }
    let last = previous.expect("ROUNDS is not zero, so at least one capture was taken");
    let difference = match wanted {
        Wanted::Clear => last
            .difference(&Shot::uniform(last.size(), CLEAR))
            .to_string(),
        Wanted::ChangedFrom(known) => last.difference(known).to_string(),
    };
    Err(match round {
        Round::NotYet => NotSettled::Unchanged {
            what,
            rounds: ROUNDS,
            difference,
        },
        // `Landed` returns from the loop, so the only other way out is a window that kept
        // moving under the capture.
        Round::Unstable | Round::Landed => NotSettled::Unstable {
            what,
            rounds: ROUNDS,
            difference,
        },
    })
}

/// **The control on this file**, and a control rather than a test because the criterion's
/// whole job is to *refuse* something: an assertion that is an absence needs a case where
/// the absence is not there (`HANDOVER.md`).
///
/// It states all three conclusions over synthetic captures, and the first is the defect of
/// `doc/notes-present-rate.md` §4: a window that is stale **and stable** — every capture
/// agreeing with every other, all of them showing the previous picture — must read
/// `NotYet` for ever. A criterion that said `Landed` there is the 300 ms wall clock with
/// more code, and would accept exactly the capture that failed once in five real-display
/// runs.
///
/// Runs first of everything and under `--check`, for
/// [`crate::arrangement::the_shapes_are_the_ones_adr_0058_counted`]'s reason: it needs no
/// display, so nothing about a display can excuse it not running.
pub(crate) fn the_criterion_refuses_a_stale_window() {
    let size = (4, 4);
    let was = Shot::uniform(size, [0, 128, 0]);
    let now = Shot::uniform(size, [200, 0, 0]);
    let other = Shot::uniform(size, [200, 0, 1]);
    let clear = Shot::uniform(size, CLEAR);
    // Within ADR 0006's bound of the clear, which the erase must accept, and which
    // nothing else in this example is: a near-black window is still an erased one.
    let nearly_clear = Shot::uniform(size, [2, 0, 1]);

    assert_eq!(
        judge(Wanted::ChangedFrom(&was), Some(&was), &was),
        Round::NotYet,
        "a stale window that is perfectly stable must never settle: that is the 300 ms \
         wall clock's failure wearing a convergence criterion's name"
    );
    assert_eq!(
        judge(Wanted::ChangedFrom(&was), Some(&was), &now),
        Round::Unstable,
        "one capture of a new picture may have caught it half-composited"
    );
    assert_eq!(
        judge(Wanted::ChangedFrom(&was), Some(&now), &now),
        Round::Landed,
        "two consecutive captures agreeing on something new is what a settle is"
    );
    assert_eq!(
        judge(Wanted::ChangedFrom(&was), Some(&other), &now),
        Round::Unstable,
        "two different captures are not an agreement"
    );
    assert_eq!(
        judge(Wanted::ChangedFrom(&was), None, &now),
        Round::Unstable,
        "the first capture of a run has nothing to agree with"
    );

    // The chain's first link has the same two failure modes, against a state the library
    // names rather than one a previous capture named.
    assert_eq!(
        judge(Wanted::Clear, Some(&now), &now),
        Round::NotYet,
        "an erase must not settle on a stable picture, however stable it is"
    );
    assert_eq!(
        judge(Wanted::Clear, Some(&clear), &clear),
        Round::Landed,
        "two captures agreeing on the clear is the chain's first proven state"
    );
    assert_eq!(
        judge(Wanted::Clear, Some(&nearly_clear), &nearly_clear),
        Round::Landed,
        "the erase's terminal state is the clear within ADR 0006's bound, not an exact zero"
    );

    // The bound is a count, and a red run's reader needs it named before anything else.
    let refused = NotSettled::Unchanged {
        what: "the page",
        rounds: ROUNDS,
        difference: was.difference(&was).to_string(),
    };
    let message = refused.to_string();
    assert!(
        message.contains(&ROUNDS.to_string()) && message.contains("the page"),
        "a failed settle must name its bound and what it was waiting for, got: {message}"
    );
}
