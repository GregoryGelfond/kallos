//! The layout model — the single home of the house-style constants.
//! All fields private: a struct with only private fields cannot be
//! struct-literal-constructed or `..`-updated by another crate, so internal
//! fields may grow without breaking callers. NOT `Copy` (semver
//! headroom for future non-Copy fields). The *rules* (which opener abuts, which
//! separator trails) are emitter algorithm, not data — only scalar constants
//! live here.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Style {
    line_width: usize,
    indent: usize,
    neck_width: usize,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            line_width: 100,
            indent: 4,
            neck_width: 3,
        }
    }
}

impl Style {
    /// Set the one user-facing knob. Clamps to the floor of 1 (a 0 becomes 1) so
    /// the value carries `line_width >= 1` for its whole lifetime (Hoare: the
    /// type carries its invariant from birth). Strict rejection of a user-typed
    /// non-positive value lives at the CLI boundary, not here.
    #[must_use]
    pub fn with_line_width(mut self, cols: usize) -> Self {
        self.line_width = cols.max(1);
        self
    }

    #[must_use]
    pub fn line_width(&self) -> usize {
        self.line_width
    }
}

// `indent` / `neck_width` are read by the emitter (`nest_deltas`).
impl Style {
    pub(crate) fn indent(&self) -> usize {
        self.indent
    }

    pub(crate) fn neck_width(&self) -> usize {
        self.neck_width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_house_style_width_100() {
        assert_eq!(Style::default().line_width(), 100);
    }

    #[test]
    fn with_line_width_clamps_zero_to_one() {
        assert_eq!(Style::default().with_line_width(0).line_width(), 1);
        assert_eq!(Style::default().with_line_width(80).line_width(), 80);
    }

    #[test]
    fn house_constants_are_fixed() {
        let s = Style::default();
        assert_eq!(s.indent(), 4);
        assert_eq!(s.neck_width(), 3); // ":- " = 3
    }
}
