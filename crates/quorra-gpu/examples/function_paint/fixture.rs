//! Where the two programs come from: the caller's own corpus, read, never copied.
//!
//! `QUORRA_FUNCTION_PAINT.md` §1 names both witnesses — "2580 bytes of PostScript
//! calculator with `ifelse` branches driving a seven-segment display" and "1605
//! computing π by the BBP series". They are tracked bytes in
//! `/home/cl/projects/pdf-viewer/doc/corpora-own/`, and this reads the type 4 stream
//! straight out of them so the spike measures *their* programs and not a plausible
//! imitation of them. Nothing is written to that tree, and nothing is vendored into
//! this one.
//!
//! This is not a PDF parser and must never become one: the two files are
//! uncompressed, single-stream and known, and the scan below asserts the length it
//! was promised rather than discovering it.

use std::path::{Path, PathBuf};

/// One witness: its file, and what §1 says its program is.
pub(crate) struct Witness {
    /// Short name for the tables.
    pub(crate) name: &'static str,
    /// The file under the corpus root.
    pub(crate) file: &'static str,
    /// The `/Length` the stream must have, from §1 of the caller's document.
    pub(crate) length: usize,
}

/// The two the caller measured.
pub(crate) const WITNESSES: [Witness; 2] = [
    Witness {
        name: "seven-segment",
        file: "pi_seven_segment.pdf",
        length: 2580,
    },
    Witness {
        name: "bbp-pi",
        file: "type4_pi.pdf",
        length: 1605,
    },
];

/// Where the corpus is, overridable so the spike is not welded to one machine.
pub(crate) fn root() -> PathBuf {
    std::env::var_os("QUORRA_FUNCTION_PAINT_CORPUS").map_or_else(
        || PathBuf::from("/home/cl/projects/pdf-viewer/doc/corpora-own"),
        PathBuf::from,
    )
}

/// Why a witness could not be read.
#[derive(Debug)]
pub(crate) enum Missing {
    /// The file is not where [`root`] says.
    NoFile(PathBuf),
    /// No `/FunctionType 4` object in it.
    NoFunction,
    /// The stream is not the length §1 promised, so it is not the program measured.
    WrongLength {
        /// What the file has.
        found: usize,
        /// What was expected.
        expected: usize,
    },
}

impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoFile(path) => write!(f, "no such file: {}", path.display()),
            Self::NoFunction => write!(f, "no /FunctionType 4 object"),
            Self::WrongLength { found, expected } => {
                write!(f, "stream is {found} bytes, §1 names {expected}")
            }
        }
    }
}

/// Read the type 4 program out of one witness.
///
/// # Errors
///
/// [`Missing`], naming which of the three things was not there.
pub(crate) fn read(directory: &Path, witness: &Witness) -> Result<String, Missing> {
    let path = directory.join(witness.file);
    let bytes = std::fs::read(&path).map_err(|_| Missing::NoFile(path))?;
    let start = find(&bytes, b"/FunctionType 4").ok_or(Missing::NoFunction)?;
    let length_at = find(&bytes[start..], b"/Length ").ok_or(Missing::NoFunction)? + start + 8;
    let digits: String = bytes[length_at..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .map(|b| char::from(*b))
        .collect();
    let length: usize = digits.parse().map_err(|_| Missing::NoFunction)?;
    if length != witness.length {
        return Err(Missing::WrongLength {
            found: length,
            expected: witness.length,
        });
    }
    let stream = find(&bytes[length_at..], b"stream").ok_or(Missing::NoFunction)? + length_at + 6;
    let body = bytes[stream..]
        .iter()
        .position(|b| *b != b'\r' && *b != b'\n')
        .map_or(stream, |skip| stream + skip);
    let end = body.saturating_add(length).min(bytes.len());
    // Latin-1: a §7.10.5 program is ASCII, and this cannot fail on bytes that are not.
    Ok(bytes[body..end].iter().map(|b| char::from(*b)).collect())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
