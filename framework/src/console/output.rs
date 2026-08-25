//! Terminal output helpers for console commands.
//!
//! Laravel renders command progress through Blade "components"
//! (`$this->components->twoColumnDetail(...)`), which resolve terminal
//! width and ANSI colour at render time. Suprnova's console has no view
//! layer, carries no terminal-width crate, and writes to a stdout that
//! is routinely piped into a log - so the equivalent here is a pure
//! function that returns the line. Pure means the layout is unit
//! testable to the character, and the caller owns the decision to
//! print.

/// Rendered width of a [`two_column_detail`] line, in characters.
///
/// Laravel sizes its dot leader to the terminal, capped at 150 columns.
/// Reading the real width would mean a new dependency, and a layout
/// that shifts with the terminal is a layout no test can pin, so this
/// is fixed at the conventional 80.
pub const DETAIL_WIDTH: usize = 80;

/// Render `left`, a dot leader, and `right` as one line of
/// [`DETAIL_WIDTH`] characters.
///
/// ```text
///   BaseSeeder ....................................................... 812 ms DONE
/// ```
///
/// The line is a two-space margin, `left`, a space, the dot leader,
/// then a space and `right` when `right` is non-empty. When the two
/// halves leave no room the leader collapses to nothing and the line
/// runs past [`DETAIL_WIDTH`]: a seeder name you cannot read is worse
/// than a line that wraps, so this never truncates.
///
/// Width counts `char`s, not bytes, so a multi-byte name still lines
/// up. Grapheme clusters and East Asian double-width characters are
/// not accounted for - that needs a Unicode width table this crate
/// does not carry.
pub fn two_column_detail(left: &str, right: &str) -> String {
    let mut line = format!("  {left} ");
    let used = line.chars().count() + right.chars().count() + usize::from(!right.is_empty());
    line.push_str(&".".repeat(DETAIL_WIDTH.saturating_sub(used)));
    if !right.is_empty() {
        line.push(' ');
        line.push_str(right);
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_line_is_exactly_the_detail_width() {
        let line = two_column_detail("BaseSeeder", "RUNNING");
        assert_eq!(line.chars().count(), DETAIL_WIDTH);
        assert!(line.starts_with("  BaseSeeder "), "got: {line}");
        assert!(line.ends_with(" RUNNING"), "got: {line}");
        assert!(line.contains("...."), "the dot leader is missing: {line}");
    }

    #[test]
    fn an_empty_right_column_fills_the_line_with_dots() {
        let line = two_column_detail("BaseSeeder", "");
        assert_eq!(line.chars().count(), DETAIL_WIDTH);
        assert!(line.ends_with('.'), "got: {line}");
    }

    #[test]
    fn an_overlong_pair_keeps_both_halves_instead_of_truncating() {
        let long = "A".repeat(DETAIL_WIDTH);
        let line = two_column_detail(&long, "DONE");
        assert!(line.contains(&long), "the left column must survive");
        assert!(line.ends_with(" DONE"), "the right column must survive");
        assert!(line.chars().count() > DETAIL_WIDTH);
    }

    #[test]
    fn width_counts_characters_not_bytes() {
        // "Ünïcöde" is 7 chars but 10 bytes; a byte-based width would
        // render this line three columns short.
        let line = two_column_detail("Ünïcöde", "DONE");
        assert_eq!(line.chars().count(), DETAIL_WIDTH);
    }
}
