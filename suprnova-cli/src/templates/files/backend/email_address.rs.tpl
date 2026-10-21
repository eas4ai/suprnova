/// `email` as this application stores it and looks it up: without the space
/// around it, and with ASCII letters in lowercase. Every other character
/// stays as typed, so the address keeps the length the rules checked.
///
/// The database stays the one authority on case: every lookup compares
/// `LOWER(email) = LOWER(?)`. On SQLite, and on Postgres with the C
/// collation, `LOWER` folds ASCII letters only. Postgres with a Unicode
/// collation and the `_ci` collations of MySQL and MariaDB ignore case more
/// widely. Folding ASCII letters here, and nothing more, never makes a stored
/// address unreachable on any of them.
pub fn normalize(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}
