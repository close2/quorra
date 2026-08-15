//! Where each field of a WGSL uniform struct sits, computed from the shader source.
//!
//! # What this exists to catch
//!
//! Every uniform in this crate is written as a fixed-size byte array at literal
//! offsets, and **nothing in the toolchain checks one of those offsets.** `wgpu`
//! validates the buffer's *size* against the layout's `min_binding_size`, so a uniform
//! that grew or shrank is refused loudly; two fields of the same width exchanged inside
//! it is a picture, drawn without a complaint, of a page that is wrong. That is
//! principle 6's forbidden third state wearing a byte offset, and it is the one shape
//! of defect the tree had no instrument for.
//!
//! So the offsets are derived here from the source the adapter itself compiles
//! (`super`'s constants), by the layout rules of the WGSL specification, and the test
//! beside each writer feeds that writer a distinct value per field and reads each one
//! back at the offset this module computes. Neither half copies a number from the
//! other: the writer's own arithmetic produces the bytes, and the shader's own
//! declaration says where they should be.
//!
//! # The rules, and why they are the uniform ones
//!
//! WGSL §14.4.4 gives each type an alignment and a size — a scalar 4 and 4, `vec2` 8
//! and 8, `vec3` 16 and 12, `vec4` 16 and 16 — and lays a struct out by rounding each
//! member up to its own alignment. §14.4.6 then adds what the *uniform* address space
//! requires and the storage one does not: a struct's alignment and an array's element
//! stride are both rounded up to 16. Every uniform in this crate is bound as
//! `var<uniform>`, so the uniform rules are the ones that apply.
//!
//! # A type this module does not know fails the gate
//!
//! Unrecognised syntax panics rather than being skipped, and the message names what it
//! could not measure. A gate that quietly ignores the field it does not understand is a
//! gate that reports success about the half it read, which is the failure mode
//! `Counters` and the ratchet exist to prevent one directory up. This module is
//! `#[cfg(test)]`, so the panic is a failed test and nothing else.

// Offsets and strides of a struct a shader declares in this repository: every operand
// is a small literal or a sum of them, and an overflow here would be a shader nobody
// could compile. The alternative — `checked_add` on each of six lines — would hide the
// layout rule this module exists to state.
#![allow(clippy::arithmetic_side_effects)]

/// One field of a WGSL struct, placed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Field {
    /// The field's name, as the shader declares it.
    pub(crate) name: String,
    /// Its byte offset from the start of the struct.
    pub(crate) offset: usize,
    /// Its size in bytes — the type's own size, not its stride to the next field.
    pub(crate) size: usize,
}

/// A WGSL struct laid out in the uniform address space.
#[derive(Debug)]
pub(crate) struct Layout {
    /// The fields, in declaration order.
    pub(crate) fields: Vec<Field>,
    /// The whole struct's size, which is what a uniform buffer of it must be.
    pub(crate) size: usize,
}

impl Layout {
    /// Lay out `struct_name` as `source` declares it.
    ///
    /// # Panics
    ///
    /// When the struct is absent, malformed, or holds a type these rules do not cover —
    /// see the module comment on why silence would be worse.
    pub(crate) fn of(source: &str, struct_name: &str) -> Self {
        let mut fields = Vec::new();
        let mut offset = 0_usize;
        let mut struct_align = 16_usize;
        for (name, ty) in declarations(source, struct_name) {
            let (align, size) = measure(&ty, struct_name, &name);
            offset = round_up(offset, align);
            struct_align = struct_align.max(align);
            fields.push(Field { name, offset, size });
            offset += size;
        }
        assert!(
            !fields.is_empty(),
            "`struct {struct_name}` declares no fields"
        );
        Self {
            size: round_up(offset, struct_align),
            fields,
        }
    }
}

/// The `name: type` pairs of one struct, in declaration order, comments removed.
fn declarations(source: &str, struct_name: &str) -> Vec<(String, String)> {
    let opening = format!("struct {struct_name} {{");
    let body = source
        .split_once(&opening)
        .unwrap_or_else(|| panic!("the shader declares no `{opening}`"))
        .1
        .split_once("\n}")
        .unwrap_or_else(|| panic!("`struct {struct_name}` is never closed"))
        .0;
    let mut out = Vec::new();
    for line in body.lines() {
        let line = line.split("//").next().unwrap_or_default().trim();
        let line = line.trim_end_matches(',').trim();
        if line.is_empty() {
            continue;
        }
        // A field is `name: type`; the type may itself carry `<...>`, which is why the
        // split is on the *first* colon only.
        let (name, ty) = line.split_once(':').unwrap_or_else(|| {
            panic!("`struct {struct_name}` has a line that is not a field: `{line}`")
        });
        out.push((name.trim().to_owned(), ty.trim().to_owned()));
    }
    out
}

/// A type's `(alignment, size)` in the uniform address space (WGSL §14.4.4, §14.4.6).
fn measure(ty: &str, struct_name: &str, field: &str) -> (usize, usize) {
    if let Some(inner) = ty.strip_prefix("array<").and_then(|t| t.strip_suffix('>')) {
        let (element, count) = inner.rsplit_once(',').unwrap_or_else(|| {
            panic!("`{struct_name}.{field}` is a runtime-sized array, which no uniform may be")
        });
        let (element_align, element_size) = measure(element.trim(), struct_name, field);
        // §14.4.6: an array in the uniform address space has its element stride rounded
        // up to 16, so `array<vec4<u32>, 16>` is 16 tightly packed elements while
        // `array<f32, 16>` would be sixteen floats each alone in its own 16 bytes.
        let stride = round_up(element_size, element_align.max(16));
        let count: usize = count
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("`{struct_name}.{field}` has no literal element count"));
        return (element_align.max(16), stride * count);
    }
    match ty {
        "f32" | "u32" | "i32" => (4, 4),
        "vec2f" | "vec2<f32>" | "vec2u" | "vec2<u32>" | "vec2i" | "vec2<i32>" => (8, 8),
        "vec3f" | "vec3<f32>" | "vec3u" | "vec3<u32>" | "vec3i" | "vec3<i32>" => (16, 12),
        "vec4f" | "vec4<f32>" | "vec4u" | "vec4<u32>" | "vec4i" | "vec4<i32>" => (16, 16),
        other => panic!(
            "`{struct_name}.{field}` is a `{other}`, whose uniform layout this gate does not \
             know — add it here rather than letting the field go unchecked"
        ),
    }
}

/// `value` rounded up to the next multiple of `align`.
fn round_up(value: usize, align: usize) -> usize {
    value.div_ceil(align) * align
}

/// What one field is expected to hold, in the units the shader declares it in.
#[derive(Debug)]
pub(crate) enum Lane<'a> {
    /// A `u32` field.
    Word(u32),
    /// An `f32` field.
    Float(f32),
    /// A `vec2f`.
    Vec2([f32; 2]),
    /// A `vec4f`.
    Vec4([f32; 4]),
    /// Anything whose expectation is easier stated as the bytes themselves — a
    /// transfer table, or a `vec4f` a writer fills from two places.
    Bytes(&'a [u8]),
}

impl Lane<'_> {
    /// The little-endian bytes this lane expects to find.
    fn bytes(&self) -> Vec<u8> {
        match *self {
            Self::Word(v) => v.to_le_bytes().to_vec(),
            Self::Float(v) => v.to_le_bytes().to_vec(),
            Self::Vec2(v) => v.iter().flat_map(|c| c.to_le_bytes()).collect(),
            Self::Vec4(v) => v.iter().flat_map(|c| c.to_le_bytes()).collect(),
            Self::Bytes(v) => v.to_vec(),
        }
    }
}

/// Assert that `bytes` is `struct_name` of `source`, field for field.
///
/// Three things at once, and the second is the one that keeps the gate honest as the
/// shaders grow: the buffer is exactly the struct's size; `expected` names every field
/// the struct declares, in order, so a field added to the WGSL fails here until a
/// writer is shown to fill it; and each field's bytes are at the offset the layout
/// rules put them at.
///
/// # Panics
///
/// On any of the three, naming the field and the offset.
pub(crate) fn check(source: &str, struct_name: &str, bytes: &[u8], expected: &[(&str, Lane)]) {
    let layout = Layout::of(source, struct_name);
    assert_eq!(
        bytes.len(),
        layout.size,
        "the host writes {} bytes where `struct {struct_name}` is {}",
        bytes.len(),
        layout.size
    );
    let declared: Vec<&str> = layout.fields.iter().map(|f| f.name.as_str()).collect();
    let named: Vec<&str> = expected.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        declared, named,
        "`struct {struct_name}` declares {declared:?}; the gate names {named:?}"
    );
    for (field, (_, lane)) in layout.fields.iter().zip(expected) {
        let want = lane.bytes();
        assert_eq!(
            want.len(),
            field.size,
            "`{struct_name}.{}` is {} bytes; the gate expects {}",
            field.name,
            field.size,
            want.len()
        );
        let at = field.offset;
        assert_eq!(
            &bytes[at..at + field.size],
            want.as_slice(),
            "`{struct_name}.{}` is at byte {at}, and the host wrote something else there",
            field.name
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{Lane, Layout, check};

    /// Offsets are the specification's, not the declaration order's: a `vec4f` after a
    /// `u32` starts at 16 and not at 4, and the struct's own size rounds up to 16.
    ///
    /// The expected numbers are derived from WGSL §14.4.4's alignment table — scalars
    /// align to 4, `vec2` to 8, `vec4` to 16 — and §14.4.6's uniform rule that a
    /// struct's alignment is at least 16.
    #[test]
    fn a_field_is_placed_at_its_own_alignment() {
        let source = "\nstruct Params {\n    kind: u32,\n    pair: vec2f,\n    quad: vec4f,\n    tail: f32,\n}\n";
        let layout = Layout::of(source, "Params");
        let placed: Vec<(&str, usize)> = layout
            .fields
            .iter()
            .map(|f| (f.name.as_str(), f.offset))
            .collect();
        assert_eq!(
            placed,
            vec![("kind", 0), ("pair", 8), ("quad", 16), ("tail", 32)]
        );
        assert_eq!(layout.size, 48);
    }

    /// An array in the uniform address space has a stride of at least 16 (§14.4.6),
    /// which is what makes `array<vec4<u32>, 16>` 256 bytes and tightly packed.
    #[test]
    fn a_uniform_array_strides_by_sixteen() {
        let source = "\nstruct Params {\n    table: array<vec4<u32>, 16>,\n}\n";
        let layout = Layout::of(source, "Params");
        assert_eq!(layout.fields[0].size, 256);
        assert_eq!(layout.size, 256);
    }

    /// A comment between two fields is not a field, and a trailing comma is not part of
    /// a type — the shaders in this crate carry a paragraph above most fields.
    #[test]
    fn comments_and_trailing_commas_are_not_fields() {
        let source = "\nstruct Params {\n    // A clause citation, and a blank line under it.\n\n    only: vec4f, // trailing\n}\n";
        let layout = Layout::of(source, "Params");
        assert_eq!(layout.fields.len(), 1);
        assert_eq!(layout.fields[0].name, "only");
    }

    /// The gate fails when a field's bytes are at another field's offset — the exact
    /// defect it exists for, induced here so that the check is known to be able to
    /// fail (ADR 0052: a test that passes proves only that a test exists).
    #[test]
    #[should_panic(expected = "`Params.alpha` is at byte 0")]
    fn two_fields_of_one_width_exchanged_fail_the_check() {
        let source = "\nstruct Params {\n    alpha: f32,\n    beta: f32,\n}\n";
        let mut bytes = [0_u8; 16];
        // Written the wrong way round: beta's value into alpha's slot and back.
        bytes[0..4].copy_from_slice(&2.0_f32.to_le_bytes());
        bytes[4..8].copy_from_slice(&1.0_f32.to_le_bytes());
        check(
            source,
            "Params",
            &bytes,
            &[("alpha", Lane::Float(1.0)), ("beta", Lane::Float(2.0))],
        );
    }

    /// And when the shader has a field the host never writes.
    #[test]
    #[should_panic(expected = "declares [\"alpha\", \"beta\"]")]
    fn a_field_the_gate_does_not_name_fails_the_check() {
        let source = "\nstruct Params {\n    alpha: f32,\n    beta: f32,\n}\n";
        check(source, "Params", &[0; 16], &[("alpha", Lane::Float(0.0))]);
    }
}
