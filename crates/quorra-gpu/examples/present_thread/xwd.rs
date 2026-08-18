//! A window's pixels, read back from the X server through `xwd`.
//!
//! The only instrument this account has for "did the picture land where the affine
//! says": there is no readback from a swapchain texture, and there is no person to look
//! at the window. `xwd -name` dumps exactly the named window, so the capture needs no
//! window id, no `xdotool`, and no assumption about where a window manager put it —
//! under `Xvfb` there is no window manager at all.
//!
//! The format is X11's window dump: a 25-word header in **network byte order** (`xwd`
//! byte-swaps it on a little-endian machine), then the window's name, then the colour
//! map, then the pixels — `bytes_per_line` apart, `bits_per_pixel` wide, with a mask per
//! channel. Everything below is that layout and nothing else.

use std::fmt;
use std::process::Command;

/// One window's pixels, as RGB triples.
///
/// **Equality is exact, and that is deliberate.** The tolerance [`crate::same`] applies is
/// about the store conversion between the colour a scene stated and the one the window
/// shows (ADR 0006); it is not a difference two dumps of one unchanged window can have,
/// because they are the same bytes read twice. So [`crate::settle`] compares whole shots
/// with `==` and means it.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Shot {
    width: usize,
    height: usize,
    /// Row-major, `width * height` triples.
    pixels: Vec<[u8; 3]>,
}

/// The header's 25 words, named as X11 names them. Only the ones this reader uses are
/// pulled out; the rest are skipped by the header's own stated size.
struct Header {
    /// X11 calls this field `header_size`; the prefix is dropped here because the
    /// struct already says whose it is. It is the offset of the colour map, because the
    /// window's name sits inside it.
    size: usize,
    file_version: u32,
    pixmap_format: u32,
    width: usize,
    height: usize,
    byte_order: u32,
    bits_per_pixel: usize,
    bytes_per_line: usize,
    masks: [u32; 3],
    ncolors: usize,
}

/// Capture the window with this exact name, or say why it could not be done.
///
/// # Panics
///
/// When `xwd` is absent, when it fails, or when what it produced is not a window dump
/// this reader understands. All three are failures of the proof rather than of the
/// library, and a proof that quietly does not run is worse than one that fails.
pub(crate) fn capture(window_name: &str) -> Shot {
    let output = Command::new("xwd")
        .args(["-silent", "-name", window_name])
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "`xwd` could not be run ({error}); this example verifies its own pixels \
                 and cannot report a result without it (Debian/Ubuntu: x11-apps)"
            )
        });
    assert!(
        output.status.success(),
        "xwd failed on window '{window_name}': {}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse(&output.stdout)
}

impl Shot {
    /// The pixel at `(x, y)`, in device coordinates with the origin at the top-left.
    pub(crate) fn at(&self, x: usize, y: usize) -> [u8; 3] {
        assert!(
            x < self.width && y < self.height,
            "({x}, {y}) is outside the {}x{} window that was captured",
            self.width,
            self.height
        );
        self.pixels[y * self.width + x]
    }

    /// The window's size, so a caller can assert it is the one it asked for.
    pub(crate) fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// A shot of one colour, of a stated size.
    ///
    /// It exists for [`crate::settle`]'s own gate, which states what the criterion
    /// concludes from three captures and has to run where there is no window — under
    /// `--check` on a machine with no display, and for
    /// [`crate::arrangement::the_shapes_are_the_ones_adr_0058_counted`]'s reason.
    pub(crate) fn uniform((width, height): (usize, usize), color: [u8; 3]) -> Self {
        Self {
            width,
            height,
            pixels: vec![color; width * height],
        }
    }

    /// Whether every pixel is within `tolerance` of `color`.
    ///
    /// The one predicate a settle can apply to a capture **without knowing what the
    /// picture should look like**: the presenter clears the window before it draws
    /// anything (ADR 0056), so "all of it is the clear" is a state the library names
    /// rather than a state a previous capture named.
    pub(crate) fn is_uniform(&self, color: [u8; 3], tolerance: u8) -> bool {
        self.pixels.iter().all(|pixel| {
            pixel
                .iter()
                .zip(color)
                .all(|(a, b)| a.abs_diff(b) <= tolerance)
        })
    }

    /// How this shot differs from another, as a failed settle reports it.
    pub(crate) fn difference(&self, other: &Self) -> Difference {
        if self.size() != other.size() {
            return Difference::Size(self.size(), other.size());
        }
        let mut differing = 0_usize;
        let mut first: Option<FirstDifferent> = None;
        for (index, (got, want)) in self.pixels.iter().zip(&other.pixels).enumerate() {
            if got != want {
                differing += 1;
                if first.is_none() {
                    first = Some(FirstDifferent {
                        at: (index % self.width, index / self.width),
                        got: *got,
                        want: *want,
                    });
                }
            }
        }
        Difference::Pixels {
            differing,
            total: self.pixels.len(),
            first,
        }
    }
}

/// How two shots differ — the diagnostic a settle that never converged carries.
///
/// A count and a coordinate rather than a verdict: the reader of a failed settle needs to
/// tell "the window never changed" from "the window changed under the capture", and those
/// are two different shapes in this one line.
pub(crate) enum Difference {
    /// Two shots of different sizes, which nothing else about them can be compared past.
    Size((usize, usize), (usize, usize)),
    /// Same size: how many pixels differ of how many, and the first one that does.
    Pixels {
        /// Pixels whose triples are not identical.
        differing: usize,
        /// Pixels in the shot.
        total: usize,
        /// The first differing pixel in row-major order. `None` exactly when `differing`
        /// is zero.
        first: Option<FirstDifferent>,
    },
}

/// The first pixel two shots disagree about: where it is, and both of its colours.
pub(crate) struct FirstDifferent {
    /// Its position, with the origin at the top-left.
    at: (usize, usize),
    /// What the shot the comparison was made *from* holds there.
    got: [u8; 3],
    /// What the other shot holds there.
    want: [u8; 3],
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Size((aw, ah), (bw, bh)) => {
                write!(f, "a {aw}x{ah} shot against a {bw}x{bh} one")
            }
            Self::Pixels {
                differing: 0,
                total,
                ..
            } => write!(f, "all {total} pixels identical"),
            Self::Pixels {
                differing,
                total,
                first,
            } => {
                write!(f, "{differing} of {total} pixels differ")?;
                if let Some(FirstDifferent {
                    at: (x, y),
                    got,
                    want,
                }) = first
                {
                    write!(f, ", first at ({x}, {y}): {got:?} against {want:?}")?;
                }
                Ok(())
            }
        }
    }
}

fn parse(dump: &[u8]) -> Shot {
    let header = read_header(dump);
    assert_eq!(header.file_version, 7, "not an X11 window dump");
    assert_eq!(
        header.pixmap_format, 2,
        "only ZPixmap dumps are read here; xwd writes one for every visual this test uses"
    );
    assert!(
        header.bits_per_pixel == 24 || header.bits_per_pixel == 32,
        "a {}-bit visual is not one this reader knows",
        header.bits_per_pixel
    );
    // The header, the window's name inside it, then the colour map: the header's stated
    // size covers the first two, and a TrueColor visual has no colours in the third.
    let start = header
        .size
        .checked_add(header.ncolors * 12)
        .expect("a window dump's offsets fit a usize");
    let data = &dump[start..];
    let stride = header.bits_per_pixel / 8;
    let mut pixels = Vec::with_capacity(header.width * header.height);
    for y in 0..header.height {
        let row = &data[y * header.bytes_per_line..];
        for x in 0..header.width {
            let bytes = &row[x * stride..x * stride + stride];
            let raw = assemble(bytes, header.byte_order);
            pixels.push([
                channel(raw, header.masks[0]),
                channel(raw, header.masks[1]),
                channel(raw, header.masks[2]),
            ]);
        }
    }
    Shot {
        width: header.width,
        height: header.height,
        pixels,
    }
}

fn read_header(dump: &[u8]) -> Header {
    assert!(dump.len() >= 100, "a window dump is at least its header");
    let word = |index: usize| -> u32 {
        let at = index * 4;
        u32::from_be_bytes([dump[at], dump[at + 1], dump[at + 2], dump[at + 3]])
    };
    Header {
        size: word(0) as usize,
        file_version: word(1),
        pixmap_format: word(2),
        width: word(4) as usize,
        height: word(5) as usize,
        byte_order: word(7),
        bits_per_pixel: word(11) as usize,
        bytes_per_line: word(12) as usize,
        masks: [word(14), word(15), word(16)],
        ncolors: word(19) as usize,
    }
}

/// One pixel's bits, in the order the dump's `byte_order` says they were written
/// (0 = least significant byte first, 1 = most significant first).
fn assemble(bytes: &[u8], byte_order: u32) -> u32 {
    let mut value = 0_u32;
    if byte_order == 0 {
        for (shift, byte) in bytes.iter().enumerate() {
            value |= u32::from(*byte) << (shift * 8);
        }
    } else {
        for byte in bytes {
            value = (value << 8) | u32::from(*byte);
        }
    }
    value
}

/// One channel of a pixel, scaled to eight bits from whatever width its mask is.
fn channel(raw: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let value = (raw & mask) >> mask.trailing_zeros();
    let bits = mask.count_ones();
    // Eight-bit channels, which every visual these runs use has, need no scaling; the
    // shift is what a narrower one (a 16-bit visual's 5 bits, say) would need.
    if bits >= 8 {
        (value >> (bits - 8)) as u8
    } else {
        (value << (8 - bits)) as u8
    }
}
