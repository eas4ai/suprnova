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
    // The prefix alternative leads its group; see `ordered_terms`.
    alternatives.sort_by_key(|alternative| !alternative.ends_with('*'));
    format!("({})", alternatives.join(" "))
}

/// InnoDB's boolean parser, in MySQL 8.4 and MariaDB 12.3 alike, ignores a
/// `word*` prefix term that follows a quoted term anywhere earlier in the same
/// query: `"apple" orchard*` neither matches nor scores `orchard`, while
/// `orchard* "apple"` does both. Boolean mode gives the order no meaning, so
/// the prefix group is rendered first and every quoted alternative, phrase and
/// exclusion follows it.
fn ordered_terms(query: &ExpandedQuery) -> impl Iterator<Item = &ExpandedTerm> {
    let (prefixed, rest): (Vec<_>, Vec<_>) = query.terms.iter().partition(|item| item.prefix);
    prefixed.into_iter().chain(rest)
}

fn render(query: &ExpandedQuery, disable_typos: bool) -> String {
    let mut groups = ordered_terms(query)
        .map(|item| {
            format!(
                "{}{}",
                if item.required { "+" } else { "" },
                term(item, disable_typos)
            )
        })
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
    let mut groups = ordered_terms(query)
        .map(|item| term(item, disable_typos))
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

fn native_predicate(query: &ExpandedQuery, columns: &[String], sql: &mut Sql) -> String {
    let native = format!("({}) > 0", matches(columns, render(query, false), sql));
    let optional = ordered_terms(query)
        .filter(|item| !item.required)
        .map(|item| term(item, false))
        .collect::<Vec<_>>();
    if !optional.is_empty()
        && (!query.phrases.is_empty() || query.terms.iter().any(|item| item.required))
    {
        let words = matches(columns, optional.join(" "), sql);
        return format!("({native}) AND (({words}) > 0)");
    }
    native
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
        return Ok(native_predicate(query, &columns, sql));
    }
    let mut clauses = super::word_predicates(query, |item| {
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
        Ok(format!("({})", alternatives.join(" OR ")))
    })?;
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
    fn word_groups_are_optional_but_phrases_and_negatives_stay_required() {
        let query = crate::search::expand(
            &crate::search::QueryText::parse("apple pie -green \"fresh baked\""),
            &crate::search::QuerySettings::default(),
            &crate::search::Vocabulary::default(),
            64,
        )
        .unwrap();
        assert_eq!(
            render(&query, false),
            r#"(pie*) ("apple") +"fresh baked" -"green""#
        );
        assert_eq!(
            ranking_query(&query, false),
            r#"(pie*) ("apple") "fresh baked""#
        );
    }
    #[test]
    fn the_prefix_group_and_its_wildcard_precede_every_quoted_term() {
        let query = ExpandedQuery {
            terms: vec![
                ExpandedTerm {
                    original: "new york".into(),
                    alternatives: vec!["new york".into(), "nyc".into()],
                    prefix: false,
                    required: true,
                    typo_alternatives: vec![],
                },
                ExpandedTerm {
                    original: "orch".into(),
                    alternatives: vec!["orchid".into(), "orch".into()],
                    prefix: true,
                    required: false,
                    typo_alternatives: vec!["orchid".into()],
                },
            ],
            phrases: vec![vec!["fresh".into(), "baked".into()]],
            negatives: vec!["green".into()],
            ..ExpandedQuery::default()
        };
        assert_eq!(
            render(&query, false),
            r#"(orch* "orchid") +("new york" "nyc") +"fresh baked" -"green""#
        );
        assert_eq!(
            ranking_query(&query, true),
            r#"(orch*) ("new york" "nyc") "fresh baked""#
        );
    }
    #[test]
    fn mysql_boolean_groups_keep_optional_synonyms_prefixes_and_required_phrases() {
        let query = super::super::expansion_fixture();
        assert_eq!(
            render(&query, false),
            "(rust* \"trust\" \"rust language\") +\"database engine\" -\"slow\""
        );
        assert_eq!(
            render(&query, true),
            "(rust* \"rust language\") +\"database engine\" -\"slow\""
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
