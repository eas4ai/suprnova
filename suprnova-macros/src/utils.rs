/// Calculate Levenshtein distance between two strings
/// Used for fuzzy matching suggestions in error messages
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

    let mut matrix: Vec<Vec<usize>> = vec![vec![0; len_b + 1]; len_a + 1];

    for (i, row) in matrix.iter_mut().enumerate().take(len_a + 1) {
        row[0] = i;
    }
    for (j, cell) in matrix[0].iter_mut().enumerate().take(len_b + 1) {
        *cell = j;
    }

    for i in 1..=len_a {
        for j in 1..=len_b {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            matrix[i][j] = std::cmp::min(
                std::cmp::min(matrix[i - 1][j] + 1, matrix[i][j - 1] + 1),
                matrix[i - 1][j - 1] + cost,
            );
        }
    }

    matrix[len_a][len_b]
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
