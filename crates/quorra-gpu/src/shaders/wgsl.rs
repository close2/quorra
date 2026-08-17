//! Cutting a module-scope function out of a WGSL source.
//!
//! Two gates in this directory read shader *text* for a property no compiler states:
//! [`copies`](super::copies) requires that the promised copies of a helper are
//! byte-identical, and [`shape_inputs`](super::shape_inputs) requires that no shape
//! entry point can reach the soft mask. Both need the same two operations — find a
//! function's text, and find where its body ends — and a second brace matcher is
//! exactly the drift `copies` exists to refuse, so there is one.
//!
//! Nothing here parses WGSL, and nothing here needs to: both gates ask about *whole
//! functions*, and a function is delimited by its `fn` keyword and a balanced brace.

/// The text of the module-scope function `name` in `source`, from its `fn` keyword
/// through the closing brace of its body.
///
/// Returns `None` when the shader does not define it. Panics if it is defined twice,
/// which would make any question about "the" function ambiguous.
pub(super) fn function_text<'a>(shader: &str, source: &'a str, name: &str) -> Option<&'a str> {
    let opening = format!("fn {name}(");
    let starts: Vec<usize> = source
        .match_indices(&opening)
        .map(|(at, _)| at)
        // Module scope only: a call `soft_mask_value(...)` inside another function is
        // not a definition, and neither is a substring of a longer name.
        .filter(|at| *at == 0 || source.get(..*at).is_some_and(|head| head.ends_with('\n')))
        .collect();
    assert!(
        starts.len() <= 1,
        "{shader} defines `{name}` {} times",
        starts.len()
    );
    let start = *starts.first()?;
    let end = body_end(source, start).unwrap_or_else(|| {
        panic!("{shader}: `{name}` has no balanced body — unclosed brace?");
    });
    source.get(start..end)
}

/// Every module-scope function `source` declares, in the order it declares them.
///
/// The names are what a reachability walk moves between; an entry point is one of them
/// like any other, which is why this makes no distinction for `@fragment`.
pub(super) fn function_names(source: &str) -> Vec<&str> {
    source
        .match_indices("\nfn ")
        .filter_map(|(at, _)| {
            source
                .get(at.saturating_add(4)..)
                .and_then(|rest| rest.split('(').next())
        })
        .map(str::trim)
        .collect()
}

/// The byte just past the closing brace of the first `{`-delimited block at or after
/// `start`, counting nested braces and ignoring those inside comments.
///
/// The functions cut out here contain no string literals with braces in them, and WGSL
/// has no string literal at all in the sense C does; the comment skipping is here so
/// that a helper which grows a comment is still cut whole rather than quietly short.
pub(super) fn body_end(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut i = start;
    while i < bytes.len() {
        match (bytes[i], bytes.get(i.saturating_add(1))) {
            (b'/', Some(b'/')) => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i = i.saturating_add(1);
                }
            }
            (b'/', Some(b'*')) => {
                i = i.saturating_add(2);
                while i.saturating_add(1) < bytes.len()
                    && !(bytes[i] == b'*' && bytes[i.saturating_add(1)] == b'/')
                {
                    i = i.saturating_add(1);
                }
                i = i.saturating_add(2);
            }
            (b'{', _) => {
                depth = depth.saturating_add(1);
                i = i.saturating_add(1);
            }
            (b'}', _) => {
                depth = depth.checked_sub(1)?;
                i = i.saturating_add(1);
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => i = i.saturating_add(1),
        }
    }
    None
}
