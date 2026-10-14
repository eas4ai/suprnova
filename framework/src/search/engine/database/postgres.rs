//! Postgres stores weighted tsvectors and retains language-specific native ranking.

use super::Sql;
use crate::{
    FrameworkError,
    search::{
        expansion::{ExpandedQuery, ExpandedTerm, tokenize},
        settings::{IndexSettings, LanguageConfig},
    },
};

fn configuration(settings: &IndexSettings) -> Result<&str, FrameworkError> {
    let name = match &settings.index_time.language {
        LanguageConfig::Server => "simple",
        LanguageConfig::Postgres { configuration } => configuration,
        language => {
            return Err(FrameworkError::param(format!(
                "search Postgres language {language:?}: requires a text search configuration"
            )));
        }
    };
    super::identifier(name).map_err(|_|FrameworkError::param(format!("search Postgres configuration {name:?}: requires [A-Za-z_][A-Za-z0-9_]* and at most 64 bytes")))?;
    Ok(name)
}

/// Native index DDL keeps language configuration fixed for a generation.
pub(super) fn ddl(table: &str, settings: &IndexSettings) -> Result<Vec<String>, FrameworkError> {
    configuration(settings)?;
    Ok(vec![
        format!(
            "ALTER TABLE \"{table}\" ADD COLUMN search_vector tsvector NOT NULL DEFAULT ''::tsvector"
        ),
        format!("CREATE INDEX \"{table}_gin\" ON \"{table}\" USING GIN (search_vector)"),
    ])
}

fn lexeme(text: &str) -> String {
    format!("'{}'", text.replace('\\', "\\\\").replace('\'', "''"))
}
fn phrase(text: &str, prefix: bool) -> String {
    let words = tokenize(text);
    words
        .iter()
        .enumerate()
        .map(|(i, word)| {
            format!(
                "{}{}",
                lexeme(word),
                if prefix && i + 1 == words.len() {
                    ":*"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join(" <-> ")
}
fn term(term: &ExpandedTerm, disable_typos: bool) -> String {
    let mut alternatives = term
        .alternatives
        .iter()
        .filter(|word| !disable_typos || !term.typo_alternatives.contains(word))
        .map(|word| format!("({})", phrase(word, term.prefix && word == &term.original)))
        .collect::<Vec<_>>();
    if alternatives.is_empty() {
        alternatives.push(format!("({})", phrase(&term.original, term.prefix)));
    }
    format!("({})", alternatives.join(" | "))
}
fn render(query: &ExpandedQuery, disable_typos: bool) -> String {
    let mut groups = query
        .terms
        .iter()
        .map(|item| term(item, disable_typos))
        .collect::<Vec<_>>();
    groups.extend(
        query
            .phrases
            .iter()
            .map(|words| format!("({})", phrase(&words.join(" "), false))),
    );
    groups.extend(
        query
            .negatives
            .iter()
            .map(|word| format!("!({})", phrase(word, false))),
    );
    groups.join(" & ")
}
fn ranking_query(query: &ExpandedQuery, disable_typos: bool) -> String {
    let mut groups = query
        .terms
        .iter()
        .map(|item| term(item, disable_typos))
        .collect::<Vec<_>>();
    groups.extend(
        query
            .phrases
            .iter()
            .map(|words| format!("({})", phrase(&words.join(" "), false))),
    );
    groups.join(" | ")
}

fn tsquery(
    settings: &IndexSettings,
    query: String,
    sql: &mut Sql,
) -> Result<String, FrameworkError> {
    let config = sql.bind(configuration(settings)?.to_owned());
    let query = sql.bind(query);
    Ok(format!("to_tsquery({config}::text::regconfig, {query})"))
}
fn column_vector(
    settings: &IndexSettings,
    column: &str,
    sql: &mut Sql,
) -> Result<String, FrameworkError> {
    let config = sql.bind(configuration(settings)?.to_owned());
    Ok(format!(
        "to_tsvector({config}::text::regconfig, COALESCE({column}, ''))"
    ))
}
fn weights(settings: &IndexSettings) -> Vec<f32> {
    let mut weights = settings
        .index_time
        .searchable_attributes
        .iter()
        .map(|a| a.weight)
        .collect::<Vec<_>>();
    weights.sort_by(|a, b| b.total_cmp(a));
    weights.dedup();
    weights
}

/// Stored vectors let the GIN index serve candidate selection without re-tokenizing rows.
pub(super) fn vector(settings: &IndexSettings, sql: &mut Sql) -> Result<String, FrameworkError> {
    configuration(settings)?;
    let weights = weights(settings);
    let vectors = settings
        .index_time
        .searchable_attributes
        .iter()
        .map(|attribute| {
            let column = sql.ident(&attribute.name);
            let document = column_vector(settings, &column, sql)?;
            let class = weights
                .iter()
                .position(|weight| *weight == attribute.weight)
                .unwrap_or(3)
                .min(3);
            let class = sql.bind(["A", "B", "C", "D"][class].to_owned());
            Ok(format!("setweight({document}, {class}::text::\"char\")"))
        })
        .collect::<Result<Vec<_>, FrameworkError>>()?;
    Ok(if vectors.is_empty() {
        "''::tsvector".into()
    } else {
        vectors.join(" || ")
    })
}

/// Bound lexical clauses preserve shared expansion and exclusion semantics.
pub(super) fn predicate(
    _table: &str,
    settings: &IndexSettings,
    query: &ExpandedQuery,
    sql: &mut Sql,
) -> Result<String, FrameworkError> {
    if query.terms.is_empty() && query.phrases.is_empty() && query.negatives.is_empty() {
        return Ok("1=1".into());
    }
    if settings.index_time.searchable_attributes.is_empty() {
        return Ok(if query.terms.is_empty() && query.phrases.is_empty() {
            "1=1"
        } else {
            "1=0"
        }
        .into());
    }
    let native = tsquery(settings, render(query, false), sql)?;
    let mut clauses = vec![format!("s.search_vector @@ {native}")];
    // Concatenated tsvectors can join the end of one field to the next field.
    // Field checks preserve phrase adjacency and field-specific typo exclusions.
    let field_checks = !query.typo_disabled_attributes.is_empty()
        || !query.phrases.is_empty()
        || query
            .terms
            .iter()
            .any(|item| item.alternatives.iter().any(|word| word.contains(' ')));
    if field_checks {
        for item in &query.terms {
            let fields = settings
                .index_time
                .searchable_attributes
                .iter()
                .map(|attribute| {
                    let column = format!("s.{}", sql.ident(&attribute.name));
                    let vector = column_vector(settings, &column, sql)?;
                    let query = tsquery(
                        settings,
                        term(
                            item,
                            query.typo_disabled_attributes.contains(&attribute.name),
                        ),
                        sql,
                    )?;
                    Ok(format!("{vector} @@ {query}"))
                })
                .collect::<Result<Vec<_>, FrameworkError>>()?;
            clauses.push(format!("({})", fields.join(" OR ")));
        }
        for words in &query.phrases {
            let fields = settings
                .index_time
                .searchable_attributes
                .iter()
                .map(|attribute| {
                    let column = format!("s.{}", sql.ident(&attribute.name));
                    let vector = column_vector(settings, &column, sql)?;
                    let query = tsquery(settings, phrase(&words.join(" "), false), sql)?;
                    Ok(format!("{vector} @@ {query}"))
                })
                .collect::<Result<Vec<_>, FrameworkError>>()?;
            clauses.push(format!("({})", fields.join(" OR ")));
        }
    }
    Ok(clauses.join(" AND "))
}

/// Native relevance keeps attribute weights in the database ranking model.
pub(super) fn score(
    _table: &str,
    settings: &IndexSettings,
    query: &ExpandedQuery,
    sql: &mut Sql,
) -> Result<String, FrameworkError> {
    if query.terms.is_empty() && query.phrases.is_empty()
        || settings.index_time.searchable_attributes.is_empty()
    {
        return Ok("0.0".into());
    }
    let weights = weights(settings);
    if weights.len() <= 4 && query.typo_disabled_attributes.is_empty() {
        // Native rank restricts class weights to 0..1, so retain relative weights.
        let maximum = weights.first().copied().unwrap_or(1.0);
        let weights = (0..4)
            .rev()
            .map(|i| sql.bind(weights.get(i).copied().unwrap_or(0.0) / maximum))
            .map(|value| format!("{value}::real"))
            .collect::<Vec<_>>()
            .join(", ");
        let query = tsquery(settings, render(query, false), sql)?;
        return Ok(format!(
            "ts_rank_cd(ARRAY[{weights}], s.search_vector, {query})"
        ));
    }
    // PostgreSQL has four weight classes. Per-column native rank preserves additional weights.
    let ranks = settings
        .index_time
        .searchable_attributes
        .iter()
        .map(|attribute| {
            let weight = sql.bind(attribute.weight);
            let column = format!("s.{}", sql.ident(&attribute.name));
            let vector = column_vector(settings, &column, sql)?;
            let query = tsquery(
                settings,
                ranking_query(
                    query,
                    query.typo_disabled_attributes.contains(&attribute.name),
                ),
                sql,
            )?;
            Ok(format!("{weight} * ts_rank_cd({vector}, {query})"))
        })
        .collect::<Result<Vec<_>, FrameworkError>>()?;
    Ok(ranks.join(" + "))
}

/// Native match markers let the caller escape source text before inserting trusted tags.
pub(super) fn highlights(
    _table: &str,
    settings: &IndexSettings,
    query: &ExpandedQuery,
    sql: &mut Sql,
) -> Result<Vec<(String, String)>, FrameworkError> {
    if query.terms.is_empty() && query.phrases.is_empty() {
        return Ok(Vec::new());
    }
    settings
        .index_time
        .searchable_attributes
        .iter()
        .map(|attribute| {
            let config = sql.bind(configuration(settings)?.to_owned());
            let column = format!("s.{}", sql.ident(&attribute.name));
            let native = tsquery(
                settings,
                render(
                    query,
                    query.typo_disabled_attributes.contains(&attribute.name),
                ),
                sql,
            )?;
            let options =
                sql.bind("StartSel=\u{001e}, StopSel=\u{001f}, HighlightAll=true".to_owned());
            Ok((
                attribute.name.clone(),
                format!(
                    "ts_headline({config}::text::regconfig, COALESCE({column}, ''), {native}, {options})"
                ),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn postgres_renders_phrase_synonym_prefix_and_negative() {
        let query = ExpandedQuery {
            terms: vec![ExpandedTerm {
                original: "rust".into(),
                alternatives: vec!["rust".into(), "trust".into(), "rust language".into()],
                prefix: true,
                typo_alternatives: vec!["trust".into()],
            }],
            phrases: vec![vec!["database".into(), "engine".into()]],
            negatives: vec!["slow".into()],
            ..ExpandedQuery::default()
        };
        assert_eq!(
            render(&query, false),
            "(('rust':*) | ('trust') | ('rust' <-> 'language')) & ('database' <-> 'engine') & !('slow')"
        );
        assert_eq!(
            term(&query.terms[0], true),
            "(('rust':*) | ('rust' <-> 'language'))"
        );
    }
    #[test]
    fn postgres_configuration_requires_plain_identifier() {
        let mut settings = IndexSettings::default();
        settings.index_time.language = LanguageConfig::Postgres {
            configuration: "english); DROP TABLE users".into(),
        };
        assert!(ddl("search_books_v1", &settings).is_err());
        settings.index_time.language = LanguageConfig::Postgres {
            configuration: "simple".into(),
        };
        assert!(ddl("search_books_v1", &settings).is_ok());
    }
}
