/// Calculate Levenshtein distance between two strings
/// Used for fuzzy matching suggestions in error messages
///
/// Each row of the edit-distance table depends only on the row above it,
/// so two rows are kept rather than the whole table.
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let len_a = a_chars.len();
    let len_b = b_chars.len();

    if len_a == 0 {
        return len_b;
    }
    if len_b == 0 {
        return len_a;
    }

    let mut previous: Vec<usize> = (0..=len_b).collect();
    let mut current: Vec<usize> = vec![0; len_b + 1];

    for i in 1..=len_a {
        current[0] = i;
        for j in 1..=len_b {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            current[j] = std::cmp::min(
                std::cmp::min(previous[j] + 1, current[j - 1] + 1),
                previous[j - 1] + cost,
            );
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[len_b]
}

#[cfg(test)]
mod mem_audit {
    use super::levenshtein_distance;

    /// MEM-005: the distances are the same as ever.
    #[test]
    fn mem_audit_distances_are_unchanged() {
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", ""), 3);
        assert_eq!(levenshtein_distance("é", "e"), 1);
        assert_eq!(levenshtein_distance("日本", "日"), 1);
        assert_eq!(levenshtein_distance("same", "same"), 0);
    }

    /// MEM-005: the distance keeps two rows, not the whole matrix. This
    /// crate's test binary links the standard library dynamically, so a
    /// heap profiler cannot see its allocations; the source is checked.
    #[test]
    fn mem_audit_the_distance_keeps_two_rows() {
        let source = include_str!("utils.rs");
        let body = &source[..source.find("#[cfg(test)]").expect("the tests")];
        assert!(
            !body.contains("Vec<Vec<"),
            "the distance keeps a whole matrix"
        );
    }
}
