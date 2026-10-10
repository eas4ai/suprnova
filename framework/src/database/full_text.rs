//! Full-text search: the options `where_full_text` takes and the SQL each
//! engine writes for it, shared by [`DbTableBuilder`], the model builder
//! [`Builder<M>`](crate::eloquent::Builder) and the schema builder's
//! full-text index.
//!
//! MySQL and MariaDB search a `FULLTEXT` index with
//! `MATCH (c1, c2) AGAINST (? IN NATURAL LANGUAGE MODE)`. Postgres searches
//! `(to_tsvector('english', c1) || to_tsvector('english', c2)) @@
//! plainto_tsquery('english', ?)`, and its `GIN` index is built over the same
//! `to_tsvector` expression, so the planner can answer the search from it.
//! Both shapes are Laravel's, so an index a Laravel migration created serves
//! a Suprnova query and the other way round. SQLite has neither, and every
//! call refuses with an error that names SQLite.
//!
//! [`DbTableBuilder`]: crate::database::DbTableBuilder

use sea_orm::DbBackend;

use crate::FrameworkError;

/// The Postgres text search configuration a search or an index uses when
/// none is named, as in Laravel.
pub(crate) const DEFAULT_LANGUAGE: &str = "english";

/// How a full-text search reads the text it is given.
///
/// Each mode renders on the engine that has it. On the other engine the
/// text is read as natural language, as Laravel's grammars read it.
///
/// | Mode | MySQL and MariaDB | Postgres |
/// |---|---|---|
/// | `NaturalLanguage` | `IN NATURAL LANGUAGE MODE` | `plainto_tsquery` |
/// | `Boolean` | `IN BOOLEAN MODE` | `plainto_tsquery` |
/// | `Websearch` | `IN NATURAL LANGUAGE MODE` | `websearch_to_tsquery` |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FullTextMode {
    /// The text is plain words, and a row matches by relevance to them.
    /// Operators in the text have no meaning. The default.
    #[default]
    NaturalLanguage,
    /// MySQL's boolean syntax: `+word` must occur, `-word` must not,
    /// `"a phrase"` matches the words in order and `word*` matches a prefix.
    Boolean,
    /// Postgres's web search syntax: `"a phrase"`, `-word` to exclude and
    /// `or` between alternatives.
    Websearch,
}

/// The options of a full-text search: the mode, the Postgres language, and
/// MySQL's query expansion. Laravel passes these as the options array of
/// `whereFullText`; here you build them and pass them to a `_with` method:
///
/// ```rust,no_run
/// # use suprnova::{DB, FullTextMode, FullTextOptions};
/// # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
/// let rows = DB::table("articles")
///     .where_full_text_with(
///         ["title", "body"],
///         "\"query builder\" -php",
///         FullTextOptions::new()
///             .mode(FullTextMode::Websearch)
///             .language("english"),
///     )
///     .get()
///     .await?;
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FullTextOptions {
    mode: FullTextMode,
    language: Option<String>,
    expanded: bool,
}

impl FullTextOptions {
    /// Natural-language mode, the `english` configuration on Postgres, and
    /// no query expansion: what `where_full_text` uses.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the text in `mode`. See [`FullTextMode`] for what each engine
    /// renders.
    pub fn mode(mut self, mode: FullTextMode) -> Self {
        self.mode = mode;
        self
    }

    /// Search with the Postgres text search configuration `language`, such
    /// as `simple`, `french` or a configuration you created. It must be the
    /// language the column's `GIN` index was built with, or Postgres cannot
    /// use the index. MySQL and MariaDB have no language and ignore it.
    ///
    /// The name is written into the SQL, so one that is not a plain name
    /// (letters, digits and `_`, with an optional `schema.` prefix) makes the
    /// query fail before it runs.
    pub fn language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Add MySQL's `WITH QUERY EXPANSION`: a second search that also finds
    /// rows sharing words with the best first matches. It applies to
    /// natural-language mode on MySQL and MariaDB; boolean mode and Postgres
    /// have no query expansion and ignore it, as Laravel does.
    pub fn expanded(mut self) -> Self {
        self.expanded = true;
        self
    }

    /// The Postgres configuration a search with these options uses.
    pub(crate) fn language_or_default(&self) -> &str {
        self.language.as_deref().unwrap_or(DEFAULT_LANGUAGE)
    }

    /// Check the language, the one part of the options written into the SQL
    /// text.
    pub(crate) fn validate(&self) -> Result<(), FrameworkError> {
        match &self.language {
            Some(language) => {
                check_language(language).map_err(FrameworkError::param)?;
                Ok(())
            }
            None => Ok(()),
        }
    }
}

/// Check that `language` is a plain name, as it is written between single
/// quotes into the SQL text. The message names the language.
pub(crate) fn check_language(language: &str) -> Result<(), String> {
    crate::database::validate_identifier(language)
        .map(|_| ())
        .map_err(|_| {
            format!(
                "the full-text language {language:?} is not a name; use a Postgres text search \
                 configuration such as english or simple (letters, digits and _, with an optional \
                 schema. prefix)"
            )
        })
}

/// The Postgres document a search reads and a `GIN` index stores: each
/// column's `to_tsvector` under `language`, joined with `||`, as Laravel's
/// `PostgresGrammar` writes both. `quote` quotes a column.
pub(crate) fn postgres_document<'a>(
    columns: impl IntoIterator<Item = &'a str>,
    language: &str,
    quote: impl Fn(&str) -> String,
) -> String {
    columns
        .into_iter()
        .map(|column| format!("to_tsvector('{language}', {})", quote(column)))
        .collect::<Vec<_>>()
        .join(" || ")
}

/// The search over the already quoted `columns` for `backend`, with `ph`
/// the placeholder that holds the text.
pub(crate) fn render_search(
    backend: DbBackend,
    columns: &[String],
    options: &FullTextOptions,
    ph: &str,
) -> Result<String, FrameworkError> {
    match backend {
        DbBackend::MySql => {
            let modifier = match (options.mode, options.expanded) {
                (FullTextMode::Boolean, _) => " IN BOOLEAN MODE",
                (_, true) => " IN NATURAL LANGUAGE MODE WITH QUERY EXPANSION",
                (_, false) => " IN NATURAL LANGUAGE MODE",
            };
            Ok(format!(
                "MATCH ({}) AGAINST ({ph}{modifier})",
                columns.join(", ")
            ))
        }
        DbBackend::Postgres => {
            let language = options.language_or_default();
            let document = postgres_document(columns.iter().map(String::as_str), language, |c| {
                c.to_owned()
            });
            let function = match options.mode {
                FullTextMode::Websearch => "websearch_to_tsquery",
                FullTextMode::NaturalLanguage | FullTextMode::Boolean => "plainto_tsquery",
            };
            Ok(format!("({document}) @@ {function}('{language}', {ph})"))
        }
        DbBackend::Sqlite => Err(search_unsupported_on_sqlite()),
        _ => Err(super::unsupported_database_backend(backend)),
    }
}

/// A full-text search reached SQLite, which has no `MATCH ... AGAINST` and
/// no `to_tsvector`. `param`, like the refusal of `where_binary`: the builder
/// refuses before any I/O rather than run a search that cannot match.
pub(crate) fn search_unsupported_on_sqlite() -> FrameworkError {
    FrameworkError::param(
        "where_full_text and or_where_full_text are not supported on SQLite: full-text search \
         needs MySQL, MariaDB or Postgres. On SQLite use where_like, or an FTS5 virtual table \
         through where_raw.",
    )
}
