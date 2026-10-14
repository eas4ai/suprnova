//! MySQL and MariaDB preserve server full-text token limits and Boolean matching.

use super::Sql;
use crate::{
    FrameworkError,
    search::{
        expansion::{ExpandedQuery, ExpandedTerm, tokenize},
        settings::{IndexSettings, LanguageConfig},
    },
};

/// Native index DDL keeps language configuration fixed for a generation.
pub(super) fn ddl(table: &str, settings: &IndexSettings) -> Result<Vec<String>, FrameworkError> {
    if !matches!(settings.index_time.language, LanguageConfig::Server) {
        return Err(FrameworkError::param(format!(
            "search table {table}: MySQL and MariaDB require server language settings"
        )));
    }
    let columns = &settings.index_time.searchable_attributes;
    let mut statements = columns
        .iter()
        .enumerate()
        .map(|(i, attribute)| {
            format!(
                "CREATE FULLTEXT INDEX `ft_{i}` ON `{table}` (`{}`)",
                attribute.name
            )
        })
        .collect::<Vec<_>>();
    if columns.len() > 1 {
        statements.push(format!(
            "CREATE FULLTEXT INDEX `ft_all` ON `{table}` ({})",
            columns
                .iter()
                .map(|a| format!("`{}`", a.name))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(statements)
}

fn literal(text: &str, prefix: bool) -> String {
    let words = tokenize(text);
    if prefix && words.len() == 1 {
        format!("{}*", words[0])
    } else {
        format!("\"{}\"", words.join(" "))
    }
}

fn term(term: &ExpandedTerm, disable_typos: bool) -> String {
    let mut alternatives = term
        .alternatives
        .iter()
        .filter(|word| !disable_typos || !term.typo_alternatives.contains(word))
        .map(|word| literal(word, term.prefix && word == &term.original))
        .collect::<Vec<_>>();
    if alternatives.is_empty() {
        alternatives.push(literal(&term.original, term.prefix));
    }
    format!("+({})", alternatives.join(" "))
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
            .map(|phrase| format!("+{}", literal(&phrase.join(" "), false))),
    );
    groups.extend(
        query
            .negatives
            .iter()
            .map(|negative| format!("-{}", literal(negative, false))),
    );
    groups.join(" ")
}

fn ranking_query(query: &ExpandedQuery, disable_typos: bool) -> String {
    let mut groups = query
        .terms
        .iter()
        .map(|item| term(item, disable_typos).trim_start_matches('+').to_owned())
        .collect::<Vec<_>>();
    groups.extend(
        query
            .phrases
            .iter()
            .map(|phrase| literal(&phrase.join(" "), false)),
    );
    groups.join(" ")
}

fn matches(columns: &[String], query: String, sql: &mut Sql) -> String {
    let value = sql.bind(query);
    format!(
        "MATCH({}) AGAINST({value} IN BOOLEAN MODE)",
        columns.join(", ")
    )
}

/// Bound lexical clauses preserve shared expansion and exclusion semantics.
pub(super) fn predicate(
    _table: &str,
    settings: &IndexSettings,
    query: &ExpandedQuery,
    sql: &mut Sql,
) -> Result<String, FrameworkError> {
    let attributes = &settings.index_time.searchable_attributes;
    let columns = attributes
        .iter()
        .map(|a| format!("s.{}", sql.ident(&a.name)))
        .collect::<Vec<_>>();
    if columns.is_empty() {
        return Ok(if query.terms.is_empty() && query.phrases.is_empty() {
            "1=1"
        } else {
            "1=0"
        }
        .into());
    }
    let has_positive = !query.terms.is_empty() || !query.phrases.is_empty();
    if has_positive && query.typo_disabled_attributes.is_empty() {
        return Ok(format!(
            "({}) > 0",
            matches(&columns, render(query, false), sql)
        ));
    }
    let mut clauses = Vec::new();
    for item in &query.terms {
        let alternatives = attributes
            .iter()
            .map(|attribute| {
                let column = format!("s.{}", sql.ident(&attribute.name));
                format!(
                    "({}) > 0",
                    matches(
                        &[column],
                        term(
                            item,
                            query.typo_disabled_attributes.contains(&attribute.name)
                        ),
                        sql
                    )
                )
            })
            .collect::<Vec<_>>();
        clauses.push(format!("({})", alternatives.join(" OR ")));
    }
    for phrase in &query.phrases {
        clauses.push(format!(
            "({}) > 0",
            matches(&columns, literal(&phrase.join(" "), false), sql)
        ));
    }
    // Boolean searches containing only minus clauses match nothing on these servers.
    for negative in &query.negatives {
        clauses.push(format!(
            "NOT (({}) > 0)",
            matches(&columns, literal(negative, false), sql)
        ));
    }
    Ok(if clauses.is_empty() {
        "1=1".into()
    } else {
        clauses.join(" AND ")
    })
}

/// Native relevance keeps attribute weights in the database ranking model.
pub(super) fn score(
    _table: &str,
    settings: &IndexSettings,
    query: &ExpandedQuery,
    sql: &mut Sql,
) -> Result<String, FrameworkError> {
    if query.terms.is_empty() && query.phrases.is_empty() {
        return Ok("0e0".into());
    }
    let scores = settings
        .index_time
        .searchable_attributes
        .iter()
        .map(|attribute| {
            let weight = sql.bind(attribute.weight);
            let column = format!("s.{}", sql.ident(&attribute.name));
            let rank = matches(
                &[column],
                ranking_query(
                    query,
                    query.typo_disabled_attributes.contains(&attribute.name),
                ),
                sql,
            );
            format!("{weight} * ({rank})")
        })
        .collect::<Vec<_>>();
    Ok(if scores.is_empty() {
        "0e0".into()
    } else {
        scores.join(" + ")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mysql_boolean_groups_keep_required_synonyms_prefixes_and_exclusions() {
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
            "+(rust* \"trust\" \"rust language\") +\"database engine\" -\"slow\""
        );
        assert_eq!(
            render(&query, true),
            "+(rust* \"rust language\") +\"database engine\" -\"slow\""
        );
    }
    #[test]
    fn mysql_query_operators_cannot_escape_literals() {
        assert_eq!(
            literal("-apple +(pear) \"banana\"", false),
            "\"apple pear banana\""
        );
    }
}
