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
use crate::encode::{Encoded, Op};
use crate::pipeline::WARM_FORMAT;

/// One plan's ping-pong textures.
pub(crate) type Pair = [wgpu::Texture; 2];

/// The frame's layer textures, reused across siblings.
///
/// Not a cache: there is nothing to look up, and a pair carries no identity between
/// tenants. It is a free list whose length is the answer to "how many did this frame
/// need at once".
pub(crate) struct LayerPool {
    free: Vec<Pair>,
    live: usize,
    peak: usize,
}

impl LayerPool {
    pub(crate) const fn new() -> Self {
        Self {
            free: Vec::new(),
            live: 0,
            peak: 0,
        }
    }

    /// A pair for a plan about to render. Reuses a released one when there is one;
    /// creates textures only when the depth of this frame's tree has grown past
    /// anything seen so far, which is what [`peak_pairs`] priced.
    pub(crate) fn acquire(&mut self, device: &Device, width: u32, height: u32) -> Pair {
        self.live = self.live.saturating_add(1);
        self.peak = self.peak.max(self.live);
        self.free.pop().unwrap_or_else(|| {
            [
                device.create_internal_texture("quorra layer", width, height, WARM_FORMAT),
                device.create_internal_texture("quorra layer", width, height, WARM_FORMAT),
            ]
        })
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

/// The most pairs that are alive at one moment: the depth of the plan tree, counting
/// the root, and the deepest mask group's tree beside it.
///
/// Masks realise one at a time before the root renders and release their pairs to the
/// same pool, so the peak is the deepest single tree rather than the sum.
pub(crate) fn peak_pairs(encoded: &Encoded) -> usize {
    let mut depth = vec![0_usize; encoded.layers.len()];
    // A child plan is created by `plan_child` *during* its parent's body, so its index
    // is always greater than its parent's: computing backwards, every child a plan
    // names is already done. `debug_assert` states the invariant where it is relied on.
    for index in (0..encoded.layers.len()).rev() {
        let mut deepest_child = 0;
        for op in &encoded.layers[index].ops {
            if let Op::Child(child) = op {
                debug_assert!(
                    child.layer > index,
                    "plan_child pushes a child after its parent, so indices increase"
                );
                deepest_child = deepest_child.max(depth.get(child.layer).copied().unwrap_or(0));
            }
        }
        depth[index] = deepest_child.saturating_add(1);
    }

    let tree_depth = |ops: &[Op]| {
        ops.iter()
            .filter_map(|op| match op {
                Op::Child(child) => depth.get(child.layer).copied(),
                _ => None,
            })
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    };

    let root = tree_depth(&encoded.root.ops);
    encoded
        .mask_plans
        .iter()
        .flatten()
        .map(|plan| depth.get(plan.root).copied().unwrap_or(1))
        .fold(root, usize::max)
}

/// What the compositor's internal textures cost this frame, for the budget check
/// before any of them exist (§5: count then allocate; the refusal names both numbers).
///
/// [`peak_pairs`] pairs of full-target RGBA, plus one R8 per used mask — a mask's
/// reduced bytes are read by draws all over the frame, so unlike a layer it lives from
/// its reduction to the last pass. `force_layers` prices the root pair a
/// damage-patched flat frame renders through (ADR 0012).
pub(crate) fn internal_texture_bytes(
    encoded: &Encoded,
    width: u32,
    height: u32,
    force_layers: bool,
) -> u64 {
    let per_layer = u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(4);
    let masks_used = encoded.mask_plans.iter().flatten().count() as u64;
    let needs_layers = !encoded.layers.is_empty() || masks_used > 0 || force_layers;
    if !needs_layers {
        return 0;
    }
    (peak_pairs(encoded) as u64)
        .saturating_mul(2)
        .saturating_mul(per_layer)
        .saturating_add(masks_used.saturating_mul(per_layer / 4))
}
