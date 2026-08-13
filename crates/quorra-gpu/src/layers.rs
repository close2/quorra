//! The frame's layer textures: how many exist at once, and what that costs.
//!
//! A plan renders into **one** texture the size of its own bounds (ADR 0036), and a
//! child's composite writes into that same texture: a pass cannot read its own
//! attachment, so the pixels it is about to cover are copied out first — into a texture
//! the size of the *child*, which is the only part the composite writes (ADR 0038).
//!
//! It was a ping-pong **pair** per plan until then, each texture the full target. Until
//! ADR 0020 a frame held one pair per plan alive from the first pass to the last, and
//! priced that way: `(plans + 1) × 2` full-target textures. At 1191×1684 that is 16.05 MB
//! per plan, so seventeen plans exceeded the default 256 MiB budget — and a plan is
//! created per group *and* per element with a non-Normal blend mode, because §11.3.5 for
//! a single element is an implicit one-element group.
//!
//! **The lifetime is a depth, not a count.** The compositor walks the plan tree
//! depth-first: a child's texture is needed while it renders and while the parent's
//! composite pass reads it, and never again. Siblings therefore never need textures at
//! the same time. [`LayerPool`] hands out the same ones again, and [`peak_layer_bytes`]
//! prices exactly that peak — the heaviest root-to-leaf chain of the plan tree, each plan
//! at its own size (ADR 0036).
//!
//! **Why handing back a texture with someone else's pixels in it is safe:** every
//! acquired texture is fully written before it is read — the first draw pass clears it, a
//! seeded non-isolated group blits its backdrop over it (ADR 0019), a copied backdrop is
//! written whole by its blit, and a plan with no ops at all clears once. A composite is
//! the one pass that writes only part of its attachment, and it writes into the plan's
//! own accumulator, which was cleared before anything drew. Under a damage scissor the
//! written region and the read region are the same region. Nothing ever reads a texel
//! this frame did not write.
//!
//! Passes recorded into one command encoder execute in order, so a texture reused by a
//! later sibling is written after the earlier sibling's composite has read it; `wgpu`
//! inserts the usage transitions between passes.

use crate::device::Device;
use crate::encode::{Encoded, LayerPlan, Op};
use crate::pipeline::WARM_FORMAT;

/// The frame's layer textures, reused across siblings.
///
/// Not a cache: there is nothing to look up, and a pair carries no identity between
/// tenants. It is a free list whose length is the answer to "how many did this frame
/// need at once".
#[derive(Debug)]
pub(crate) struct LayerPool {
    free: Vec<wgpu::Texture>,
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
    /// "until a measurement says otherwise", and keeping them between frames was
    /// implemented and measured and moved nothing either way. ADR 0040 re-measured the
    /// allocation those decisions were arguing about and found it worth **0.06 ms**,
    /// which settles the question in the direction ADR 0012 had already taken.
    pub(crate) fn warmed(texture: Option<wgpu::Texture>) -> Self {
        Self {
            free: texture.into_iter().collect(),
            live: 0,
            peak: 0,
        }
    }

    /// A texture for a plan about to render, or for the backdrop a composite copies out.
    /// Reuses a released one when there is one; creates one only when this frame has
    /// needed more at once than anything before it, which is what [`peak_layer_bytes`]
    /// priced.
    ///
    /// **A texture is reused only at its own size** (ADR 0036). Before layers were sized
    /// to their plans every one was the target's, and popping any free one was the same
    /// as popping a matching one; now it is not, and handing a plan a texture of somebody
    /// else's size draws its content in the wrong place — twelve pages of the caller's
    /// corpus, with a highlight sitting above the line it belongs to.
    ///
    /// A *larger* texture could be made to serve a smaller plan — every pass into it
    /// would have to set a viewport, since the lane shaders divide by the attachment's
    /// extent — and ADR 0040 measured what that would buy before building it: **0.06 ms**,
    /// the cost of the allocation it would save. Not taken.
    pub(crate) fn acquire(&mut self, device: &Device, width: u32, height: u32) -> wgpu::Texture {
        self.live = self.live.saturating_add(1);
        self.peak = self.peak.max(self.live);
        let matching = self
            .free
            .iter()
            .position(|texture| texture.width() == width && texture.height() == height);
        matching.map_or_else(
            || device.create_internal_texture("quorra layer", width, height, WARM_FORMAT),
            |at| self.free.swap_remove(at),
        )
    }

    /// Give a texture back, once every pass that reads it has been **recorded**. The
    /// module comment has the ordering argument; the pixels in it are dead from here.
    pub(crate) fn release(&mut self, texture: wgpu::Texture) {
        self.live = self.live.saturating_sub(1);
        self.free.push(texture);
    }

    /// How many textures existed at once at the worst moment — what
    /// [`peak_layer_bytes`] priced, and the number a `Frame` reports as
    /// `Counters::layer_textures`.
    pub(crate) const fn peak(&self) -> usize {
        self.peak
    }
}

/// The bytes of layer textures a frame will hold at its worst moment (ADR 0036, 0038).
///
/// Not `plans × the target`, because a texture is as big as its plan: what is alive at
/// once is a root-to-leaf *chain* of plans, each holding its own accumulator while its
/// children render, so the peak is the heaviest chain rather than the deepest one. A plan
/// with two children pays for the heavier of them, not for both, because a sibling's
/// texture is released before the next is acquired.
///
/// One term is not a plan's: while a child is being **composited** its own texture is
/// still alive and a copy of the backdrop it covers is alive beside it, at the size of
/// `child ∩ parent` (ADR 0038). So the cost of a child is the heavier of rendering it and
/// compositing it, and that is what the max below is.
fn peak_layer_bytes(encoded: &Encoded, width: u32, height: u32) -> u64 {
    let region_of = |plan: &LayerPlan| crate::compose::Region::of(plan.bounds, width, height);
    let bytes_of = |region: crate::compose::Region| {
        u64::from(region.width)
            .saturating_mul(u64::from(region.height))
            .saturating_mul(4)
    };
    // What a child costs its parent at the worst moment: rendering its own subtree, or
    // holding its result beside the backdrop copy the composite reads.
    let child_peak = |chain: &[u64], index: usize, parent: crate::compose::Region| -> u64 {
        let Some(plan) = encoded.layers.get(index) else {
            return 0;
        };
        let own = region_of(plan);
        let backdrop = own.meet(parent).map_or(0, bytes_of);
        chain
            .get(index)
            .copied()
            .unwrap_or(0)
            .max(bytes_of(own).saturating_add(backdrop))
    };
    let heaviest_child = |chain: &[u64], plan: &LayerPlan, parent: crate::compose::Region| {
        plan.ops
            .iter()
            .filter_map(|op| match op {
                Op::Child(child) => Some(child_peak(chain, child.layer, parent)),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    };
    // Backwards, so every child a plan names is already costed: the encoder appends a
    // child's plan before the plan that names it, so a child's index is always the lower.
    let mut chain = vec![0_u64; encoded.layers.len()];
    for index in (0..encoded.layers.len()).rev() {
        let plan = &encoded.layers[index];
        let region = region_of(plan);
        chain[index] = bytes_of(region).saturating_add(heaviest_child(&chain, plan, region));
    }
    // The root is as big as what the page marks, like every other plan (ADR 0039).
    let root_region = region_of(&encoded.root);
    let below_root = heaviest_child(&chain, &encoded.root, root_region);
    // A soft mask realises before the root draws and gives its textures back to the same
    // pool, so the peak is the heavier of the two rather than their sum. A mask's group is
    // never composited onto a parent, so it costs its own chain and no backdrop copy.
    let masks = encoded
        .mask_plans
        .iter()
        .flatten()
        .filter_map(|plan| chain.get(plan.root).copied())
        .max()
        .unwrap_or(0);
    bytes_of(root_region).saturating_add(below_root.max(masks))
}

/// The bytes the frame's reduced soft masks hold at once.
///
/// One R8 texel per pixel of each mask's own plan (ADR 0037), and *summed* rather than
/// maximised: unlike a layer, a mask lives from its reduction to the last pass that
/// samples it, because draws all over the frame read it.
fn mask_bytes(encoded: &Encoded, width: u32, height: u32) -> u64 {
    encoded
        .mask_plans
        .iter()
        .flatten()
        .map(|mask| {
            let bounds = encoded.layers.get(mask.root).and_then(|plan| plan.bounds);
            let region = crate::compose::Region::of(bounds, width, height);
            u64::from(region.width).saturating_mul(u64::from(region.height))
        })
        .fold(0, u64::saturating_add)
}

/// What the compositor's internal textures cost this frame, for the budget check before
/// any of them exist (§5: count then allocate; the refusal names both numbers).
///
/// The heaviest chain of layer textures, plus every reduced mask — each at its own plan's
/// rectangle rather than at the target (ADR 0036 for the layers, ADR 0037 for the masks,
/// ADR 0038 for the backdrop a composite copies). `force_layers` prices the root texture a
/// damage-patched flat frame renders through (ADR 0012).
pub(crate) fn internal_texture_bytes(
    encoded: &Encoded,
    width: u32,
    height: u32,
    force_layers: bool,
) -> u64 {
    let masks_used = encoded.mask_plans.iter().flatten().count();
    let needs_layers = !encoded.layers.is_empty() || masks_used > 0 || force_layers;
    if !needs_layers {
        return 0;
    }
    peak_layer_bytes(encoded, width, height).saturating_add(mask_bytes(encoded, width, height))
}

#[cfg(test)]
#[allow(clippy::expect_used)] // a fixture that cannot run must fail loudly
mod tests {
    use super::LayerPool;
    use crate::device::Device;
    use crate::startup::Options;

    /// **A released texture is reused only at its own size** (ADR 0036).
    ///
    /// While every layer was the target's size, popping any free one was the same as
    /// popping a matching one. It stopped being the same the moment a layer became as big
    /// as its plan, and the difference is not an inefficiency: a plan handed somebody
    /// else's texture draws its content in the wrong place. Twelve pages of the caller's
    /// corpus said so, with a highlight sitting above the line it belongs to — which is
    /// why this is a test rather than a comment.
    ///
    /// It matters twice over since ADR 0038, because the pool now also hands out the
    /// backdrop copy a composite reads — which is the child's size, not the plan's, and so
    /// asks for a second size in the middle of a frame as a matter of course.
    #[test]
    fn a_texture_is_reused_only_at_its_own_size() {
        let device = Device::headless(&Options {
            adapter: Some("llvmpipe".into()),
            ..Options::default()
        })
        .expect("llvmpipe is present wherever this suite runs");
        let mut pool = LayerPool::warmed(None);

        let big = pool.acquire(&device, 64, 64);
        assert_eq!((big.width(), big.height()), (64, 64));
        pool.release(big);

        let small = pool.acquire(&device, 32, 16);
        assert_eq!(
            (small.width(), small.height()),
            (32, 16),
            "the 64x64 texture in the pool is the wrong shape for this plan"
        );
        pool.release(small);

        // And the big one is still there for a plan that wants it.
        let again = pool.acquire(&device, 64, 64);
        assert_eq!((again.width(), again.height()), (64, 64));
    }
}
