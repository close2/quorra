//! A hasher for the frame's hot key maps, and the measurement that made it necessary.
//!
//! A page of text asks the glyph cache two hashed questions per fill — "have I seen this
//! key this frame" and "is it resident" — so a dense page hashes about twelve thousand
//! times inside `encode`. The standard library's default is `SipHash` 1-3 with a
//! per-process random seed: a good default, chosen against hash-flooding by hostile
//! keys, and neither property is ours. Our keys are a handful of `u32`s produced by our
//! own encoder from a display list a friendly process built (CLAUDE.md's position 1),
//! and the seed's randomness is a liability rather than a defence — §4.6 wants a frame
//! to be a function of its inputs.
//!
//! **The measurement**, `examples/zoom` held at 1× — the dense page, 5 933 fills over
//! 107 outlines, warm atlas, 1191×1684 on RADV, fastest of five on an idle machine,
//! `encode` alone:
//!
//! | | encode |
//! |---|---|
//! | `SipHash`, key without the fill rule | 0.932 ms |
//! | `SipHash`, key with the fill rule (ADR 0024's correctness fix) | 1.125 ms |
//! | this hasher, key with the fill rule | **0.746 ms** |
//!
//! The correctness fix cost 0.19 ms on the path that matters most — a fifth of the
//! phase, for one more field in a key hashed twice per glyph — and this takes 0.38 ms
//! back, leaving the page faster than it was before either change.
//!
//! **And a page of 65 978 *distinct* glyphs is where the first version of this hasher
//! was wrong.** A cheap accumulator without a final avalanche leaves its low bits weak,
//! which is exactly where `hashbrown` takes the bucket index; a table of hundreds never
//! notices and a table of tens of thousands clusters. The corpus's largest page went
//! from 394 ms before this work to 468 with the weak version and **397 with the
//! avalanche** — level with where it started, while an ordinary page of the same size
//! went 56 ms to 43. The lesson is in the shape of the test rather than the constant: a
//! hasher measured only on the fixture it was written for is measured at one table
//! size.
//!
//! # The algorithm, and why it is allowed to be this simple
//!
//! One multiply-xor-rotate per 8 bytes — the shape `rustc` itself uses for its interned
//! keys. It is not a cryptographic hash and must never be used where an adversary
//! chooses keys; here the keys are outline indices, transform bit patterns and a phase
//! numerator, and a collision costs a comparison rather than a picture.
//!
//! Deterministic by construction: no seed, no randomness, same keys hash the same way in
//! every process. Nothing in this crate iterates a map whose order would then reach the
//! output — the atlas is looked up by key and its uploads are a `Vec` — so this changes
//! speed and nothing else.

use std::hash::{BuildHasherDefault, Hasher};

/// A `HashMap`/`HashSet` type alias for the frame's hot key maps.
pub(crate) type FastMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<KeyHasher>>;
/// The set form of [`FastMap`].
pub(crate) type FastSet<K> = std::collections::HashSet<K, BuildHasherDefault<KeyHasher>>;

/// The multiply-xor-rotate hasher described in this module's comment.
#[derive(Default)]
pub(crate) struct KeyHasher(u64);

impl KeyHasher {
    /// An odd 64-bit constant with a well-mixed bit pattern: the fractional part of the
    /// golden ratio, which is the usual choice because its bits are as far from
    /// structured as a constant gets.
    const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

    #[inline]
    fn mix(&mut self, value: u64) {
        // Rotate before combining so that high bits of one word reach the low bits of
        // the next; multiply to diffuse. Wrapping is the arithmetic, not an overflow.
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(Self::SEED);
    }
}

impl Hasher for KeyHasher {
    #[inline]
    fn finish(&self) -> u64 {
        // A final avalanche, and it is not decoration. Multiplication propagates
        // entropy leftwards, so the accumulator's *low* bits stay weak — and those are
        // the bits `hashbrown` takes the bucket index from, while the high ones become
        // its control byte. A table of a few hundred keys never notices; a table of
        // sixty-six thousand clusters, which is where a page of that many distinct
        // glyphs found it. Two multiplies and two shifts, once per lookup.
        let mut x = self.0;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
        x ^= x >> 29;
        x.wrapping_mul(0xc4ce_b9fe_1a85_ec53)
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            // The slice is exactly eight bytes by `chunks_exact`.
            let word = u64::from_le_bytes(chunk.try_into().unwrap_or([0; 8]));
            self.mix(word);
        }
        let mut tail = 0_u64;
        for (index, byte) in chunks.remainder().iter().enumerate() {
            // At most seven bytes, so the shift is under 64.
            tail |= u64::from(*byte) << (index.min(7).saturating_mul(8));
        }
        self.mix(tail);
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.mix(u64::from(value));
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.mix(value);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.mix(value as u64);
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.mix(u64::from(value));
    }

    #[inline]
    fn write_u16(&mut self, value: u16) {
        self.mix(u64::from(value));
    }
}

#[cfg(test)]
mod tests {
    use std::hash::{BuildHasher, BuildHasherDefault};

    use super::KeyHasher;

    /// Determinism, which is the property §4.6 needs and the one a randomly seeded
    /// hasher does not have: the same key hashes the same way, every time, everywhere.
    #[test]
    fn the_same_key_hashes_the_same_way() {
        let build = BuildHasherDefault::<KeyHasher>::default();
        let key = (7_u32, [1_u32, 2, 3, 4], 9_u16);
        assert_eq!(build.hash_one(key), build.hash_one(key));
        assert_eq!(
            BuildHasherDefault::<KeyHasher>::default().hash_one(key),
            build.hash_one(key)
        );
    }

    /// And it separates the inputs the glyph key actually varies in — a wrong answer
    /// here is a cache that returns another glyph's tile, which is the defect ADR 0024
    /// found in the key itself.
    #[test]
    fn neighbouring_keys_do_not_collide() {
        let build = BuildHasherDefault::<KeyHasher>::default();
        let mut seen = std::collections::HashSet::new();
        for outline in 0_u32..64 {
            for phase in 0_u16..16 {
                for rule in 0_u8..2 {
                    assert!(
                        seen.insert(build.hash_one((outline, [0_u32; 4], phase, rule))),
                        "outline {outline}, phase {phase}, rule {rule} collided"
                    );
                }
            }
        }
    }
}
