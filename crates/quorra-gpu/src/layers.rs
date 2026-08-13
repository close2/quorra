//! The frame's layer textures: how many exist at once, and what that costs.
//!
//! A group renders into a ping-pong **pair** of full-target textures — a pass cannot
//! read its own attachment, so a child's composite writes the other one (ADR 0010).
//! Until ADR 0020 a frame held one pair per plan, all of them alive from the first pass
//! to the last, and priced that way: `(plans + 1) × 2` full-target textures. At
//! 1191×1684 that is 16.05 MB per plan, so seventeen plans exceeded the default 256 MiB
//! budget — and a plan is created per group *and* per element with a non-Normal blend
//! mode, because §11.3.5 for a single element is an implicit one-element group.
//!
//! **The lifetime is a depth, not a count.** The compositor walks the plan tree
//! depth-first: a child's pair is needed while it renders and while the parent's
//! composite pass reads it, and never again. Siblings therefore never need pairs at the
//! same time. [`LayerPool`] hands out the same textures again, and [`peak_pairs`]
//! prices exactly that peak, which is the depth of the plan tree.
//!
//! **Why handing back a texture with someone else's pixels in it is safe:** every
//! acquired pair is fully written before it is read — the first draw pass clears it, a
//! seeded non-isolated group blits its backdrop over it (ADR 0019), a composite writes
//! its whole attachment, and a plan with no ops at all clears once. Under a damage
//! scissor the written region and the read region are the same region. Nothing ever
//! reads a texel this frame did not write.
//!
//! Passes recorded into one command encoder execute in order, so a texture reused by a
//! later sibling is written after the earlier sibling's composite has read it; `wgpu`
//! inserts the usage transitions between passes.

use crate::device::Device;
use crate::encode::{Encoded, LayerPlan, Op};
use crate::pipeline::WARM_FORMAT;

/// One plan's ping-pong textures.
pub(crate) type Pair = [wgpu::Texture; 2];

/// The frame's layer textures, reused across siblings.
///
/// Not a cache: there is nothing to look up, and a pair carries no identity between
/// tenants. It is a free list whose length is the answer to "how many did this frame
/// need at once".
#[derive(Debug)]
pub(crate) struct LayerPool {
    free: Vec<Pair>,
    live: usize,
    peak: usize,
}

/// One layer-sized texture, for [`crate::device::Device::warm_for`] to make and let
/// go (ADR 0035).
///
/// Here rather than at the call site so that what a warm-up allocates and what a
/// frame allocates cannot drift apart: both are `WARM_FORMAT` at the target's size,
/// and a warm-up of another format would warm nothing.
pub(crate) fn warm_texture(device: &Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_internal_texture("quorra layer warm-up", width, height, WARM_FORMAT)
}

impl LayerPool {
    /// A pool for one frame, holding what a host warmed if anything (ADR 0035).
    ///
    /// Per frame, not per device: ADR 0012 declined to keep the compositor's textures
    /// "until a measurement says otherwise", and the measurement that prompted this — a
    /// first frame costing 25 ms against a steady 5 — turned out to be about the *first*
    /// allocation of a size rather than about reuse. Keeping pairs between frames was
    /// implemented and measured and moved nothing either way.
    pub(crate) fn warmed(pair: Option<Pair>) -> Self {
        Self {
            free: pair.into_iter().collect(),
            live: 0,
            peak: 0,
        }
    }

    /// A pair for a plan about to render. Reuses a released one when there is one;
    /// creates textures only when the depth of this frame's tree has grown past
    /// anything seen so far, which is what [`peak_pairs`] priced.
    /// **A pair is reused only at its own size** (ADR 0036). Before layers were sized to
    /// their plans every pair was the target's, and popping any free one was the same as
    /// popping a matching one; now it is not, and handing a plan a texture of somebody
    /// else's size draws its content in the wrong place — twelve pages of the caller's
    /// corpus, with a highlight sitting above the line it belongs to.
    pub(crate) fn acquire(&mut self, device: &Device, width: u32, height: u32) -> Pair {
        self.live = self.live.saturating_add(1);
        self.peak = self.peak.max(self.live);
        let matching = self
            .free
            .iter()
            .position(|pair| pair[0].width() == width && pair[0].height() == height);
        matching.map_or_else(
            || {
                [
                    device.create_internal_texture("quorra layer", width, height, WARM_FORMAT),
                    device.create_internal_texture("quorra layer", width, height, WARM_FORMAT),
                ]
            },
            |at| self.free.swap_remove(at),
        )
    }

    /// Give a pair back, once every pass that reads it has been **recorded**. The
    /// module comment has the ordering argument; the pixels in it are dead from here.
    pub(crate) fn release(&mut self, pair: Pair) {
        self.live = self.live.saturating_sub(1);
        self.free.push(pair);
    }

    /// How many pairs existed at once at the worst moment — the number
    /// [`peak_pairs`] predicted, and the one a `Frame` reports as
    /// `Counters::layer_textures` (doubled: a pair is two textures).
    pub(crate) const fn peak(&self) -> usize {
        self.peak
    }
}

/// What the compositor's internal textures cost this frame, for the budget check
/// before any of them exist (§5: count then allocate; the refusal names both numbers).
///
/// [`peak_pairs`] pairs of full-target RGBA, plus one R8 per used mask — a mask's
/// reduced bytes are read by draws all over the frame, so unlike a layer it lives from
/// its reduction to the last pass. `force_layers` prices the root pair a
/// damage-patched flat frame renders through (ADR 0012).
/// The bytes of layer pairs a frame will hold at its worst moment (ADR 0036).
///
/// Not `peak_pairs × the target`, because a pair is as big as its plan: what is alive at
/// once is a root-to-leaf *chain* of plans, each holding its own pair while its children
/// render, so the peak is the heaviest chain rather than the deepest one. A plan with two
/// children pays for the heavier of them, not for both, because a sibling's pair is
/// released before the next is acquired.
fn peak_pair_bytes(encoded: &Encoded, width: u32, height: u32) -> u64 {
    let pair_bytes = |plan: &LayerPlan, root: bool| -> u64 {
        let region = if root {
            crate::compose::Region::whole(width, height)
        } else {
            crate::compose::Region::of(plan.bounds, width, height)
        };
        u64::from(region.width)
            .saturating_mul(u64::from(region.height))
            .saturating_mul(4)
            .saturating_mul(2)
    };
    // Backwards, so every child a plan names is already costed (`peak_pairs` states the
    // ordering invariant this relies on).
    let mut chain = vec![0_u64; encoded.layers.len()];
    for index in (0..encoded.layers.len()).rev() {
        let plan = &encoded.layers[index];
        let heaviest_child = plan
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Child(child) => chain.get(child.layer).copied(),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        chain[index] = pair_bytes(plan, false).saturating_add(heaviest_child);
    }
    let below_root = encoded
        .root
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::Child(child) => chain.get(child.layer).copied(),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    // A soft mask realises before the root draws and gives its pairs back to the same
    // pool, so the peak is the heavier of the two rather than their sum.
    let masks = encoded
        .mask_plans
        .iter()
        .flatten()
        .filter_map(|plan| chain.get(plan.root).copied())
        .max()
        .unwrap_or(0);
    pair_bytes(&encoded.root, true).saturating_add(below_root.max(masks))
}

pub(crate) fn internal_texture_bytes(
    encoded: &Encoded,
    width: u32,
    height: u32,
    force_layers: bool,
) -> u64 {
    let masks_used = encoded.mask_plans.iter().flatten().count() as u64;
    let needs_layers = !encoded.layers.is_empty() || masks_used > 0 || force_layers;
    if !needs_layers {
        return 0;
    }
    // Pairs by the heaviest chain of plans, each at its own size (ADR 0036); masks are
    // still realised at the target's size, which is what ADR 0037 takes off this number.
    let per_mask = u64::from(width).saturating_mul(u64::from(height));
    peak_pair_bytes(encoded, width, height).saturating_add(masks_used.saturating_mul(per_mask))
}

#[cfg(test)]
#[allow(clippy::expect_used)] // a fixture that cannot run must fail loudly
mod tests {
    use super::LayerPool;
    use crate::device::Device;
    use crate::startup::Options;

    /// **A released pair is reused only at its own size** (ADR 0036).
    ///
    /// While every layer was the target's size, popping any free pair was the same as
    /// popping a matching one. It stopped being the same the moment a layer became as big
    /// as its plan, and the difference is not an inefficiency: a plan handed somebody
    /// else's texture draws its content in the wrong place. Twelve pages of the caller's
    /// corpus said so, with a highlight sitting above the line it belongs to — which is
    /// why this is a test rather than a comment.
    #[test]
    fn a_pair_is_reused_only_at_its_own_size() {
        let device = Device::headless(&Options {
            adapter: Some("llvmpipe".into()),
            ..Options::default()
        })
        .expect("llvmpipe is present wherever this suite runs");
        let mut pool = LayerPool::warmed(None);

        let big = pool.acquire(&device, 64, 64);
        assert_eq!((big[0].width(), big[0].height()), (64, 64));
        pool.release(big);

        let small = pool.acquire(&device, 32, 16);
        assert_eq!(
            (small[0].width(), small[0].height()),
            (32, 16),
            "the 64x64 pair in the pool is the wrong shape for this plan"
        );
        pool.release(small);

        // And the big one is still there for a plan that wants it.
        let again = pool.acquire(&device, 64, 64);
        assert_eq!((again[0].width(), again[0].height()), (64, 64));
    }
}
