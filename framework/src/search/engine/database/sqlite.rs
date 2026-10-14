//! SQLite FTS5 keeps lexical retrieval native while external content stays authoritative.

use super::Sql;
use crate::{
    FrameworkError,
    search::{
        expansion::{ExpandedQuery, ExpandedTerm},
        settings::{IndexSettings, LanguageConfig},
    },
};

/// Native index DDL keeps language configuration fixed for a generation.
pub(super) fn ddl(table: &str, settings: &IndexSettings) -> Result<Vec<String>, FrameworkError> {
    let tokenizer = match &settings.index_time.language {
        LanguageConfig::Server => "unicode61",
        LanguageConfig::Sqlite { tokenizer }
            if matches!(tokenizer.as_str(), "unicode61" | "porter unicode61") =>
        {
            tokenizer
        }
        language => {
            return Err(FrameworkError::param(format!(
                "search table {table}: SQLite language {language:?} requires unicode61 or porter unicode61"
            )));
        }
    };
    let columns: Vec<_> = settings
        .index_time
        .searchable_attributes
        .iter()
        .map(|a| quote(&a.name))
        .collect();
    if columns.is_empty() {
        return Ok(Vec::new());
    }
    let fts = quote(&format!("{table}_fts"));
    let list = columns.join(", ");
    let new = columns
        .iter()
        .map(|c| format!("new.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    let old = columns
        .iter()
        .map(|c| format!("old.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(vec![
        format!(
            "CREATE VIRTUAL TABLE {fts} USING fts5({list}, content='{table}', content_rowid='rowid', tokenize='{tokenizer}')"
        ),
        format!(
            "CREATE VIRTUAL TABLE {} USING fts5vocab('{table}_fts', 'row')",
            quote(&format!("{table}_vocab"))
        ),
        format!(
            "CREATE TRIGGER {} AFTER INSERT ON {} BEGIN INSERT INTO {fts}(rowid, {list}) VALUES (new.rowid, {new}); END",
            quote(&format!("{table}_ai")),
            quote(table)
        ),
        format!(
            "CREATE TRIGGER {} AFTER DELETE ON {} BEGIN INSERT INTO {fts}({fts}, rowid, {list}) VALUES ('delete', old.rowid, {old}); END",
            quote(&format!("{table}_ad")),
            quote(table)
        ),
        format!(
            "CREATE TRIGGER {} AFTER UPDATE OF {list} ON {} BEGIN INSERT INTO {fts}({fts}, rowid, {list}) VALUES ('delete', old.rowid, {old}); INSERT INTO {fts}(rowid, {list}) VALUES (new.rowid, {new}); END",
            quote(&format!("{table}_au")),
            quote(table)
        ),
    ])
}

fn quote(identifier: &str) -> String {
    format!("\"{identifier}\"")
}
fn literal(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

fn term(term: &ExpandedTerm, disable_typos: bool) -> String {
    let mut alternatives = term
        .alternatives
        .iter()
        .filter(|word| !disable_typos || !term.typo_alternatives.contains(word))
        .map(|word| {
            let prefix = term.prefix && word == &term.original;
            format!("{}{}", literal(word), if prefix { "*" } else { "" })
        })
        .collect::<Vec<_>>();
    if alternatives.is_empty() {
        alternatives.push(format!(
            "{}{}",
            literal(&term.original),
            if term.prefix { "*" } else { "" }
        ));
    }
    format!("({})", alternatives.join(" OR "))
}

fn positive(settings: &IndexSettings, query: &ExpandedQuery) -> String {
    let mut groups = query
        .terms
        .iter()
        .map(|item| {
            if query.typo_disabled_attributes.is_empty() {
                return term(item, false);
            }
            let fields = settings
                .index_time
                .searchable_attributes
                .iter()
                .map(|attribute| {
                    format!(
                        "{} : {}",
                        quote(&attribute.name),
                        term(
                            item,
                            query.typo_disabled_attributes.contains(&attribute.name)
                        )
                    )
                })
                .collect::<Vec<_>>();
            format!("({})", fields.join(" OR "))
        })
        .collect::<Vec<_>>();
    groups.extend(
        query
            .phrases
            .iter()
            .map(|phrase| literal(&phrase.join(" "))),
    );
    groups.join(" AND ")
}

/// Bound lexical clauses preserve shared expansion and exclusion semantics.
pub(super) fn predicate(
    table: &str,
    settings: &IndexSettings,
    query: &ExpandedQuery,
    sql: &mut Sql,
) -> Result<String, FrameworkError> {
    let fts = sql.ident(&format!("{table}_fts"));
    if settings.index_time.searchable_attributes.is_empty() {
        return Ok(if query.terms.is_empty() && query.phrases.is_empty() {
            "1=1"
        } else {
            "1=0"
        }
        .into());
    }
    let mut clauses = Vec::new();
    let positive = positive(settings, query);
    if !positive.is_empty() {
        let value = sql.bind(positive);
        clauses.push(format!(
            "s.rowid IN (SELECT rowid FROM {fts} WHERE {fts} MATCH {value})"
        ));
    }
    // FTS5 has binary NOT only, so exclusions stay independent for negative-only queries.
    for negative in &query.negatives {
        let value = sql.bind(literal(negative));
        clauses.push(format!(
            "s.rowid NOT IN (SELECT rowid FROM {fts} WHERE {fts} MATCH {value})"
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
    table: &str,
    settings: &IndexSettings,
    query: &ExpandedQuery,
    sql: &mut Sql,
) -> Result<String, FrameworkError> {
    let positive = positive(settings, query);
    if positive.is_empty() || settings.index_time.searchable_attributes.is_empty() {
        return Ok("0.0".into());
    }
    let fts = sql.ident(&format!("{table}_fts"));
    let weights = settings
        .index_time
        .searchable_attributes
        .iter()
        .map(|a| sql.bind(a.weight))
        .collect::<Vec<_>>()
        .join(", ");
    let value = sql.bind(positive);
    Ok(format!(
        "COALESCE((SELECT -bm25({fts}, {weights}) FROM {fts} WHERE rowid=s.rowid AND {fts} MATCH {value}), 0.0)"
    ))
}

/// Native match markers let the caller escape source text before inserting trusted tags.
pub(super) fn highlights(
    table: &str,
    settings: &IndexSettings,
    query: &ExpandedQuery,
    crop: &std::collections::BTreeMap<String, usize>,
    sql: &mut Sql,
) -> Result<Vec<(String, String)>, FrameworkError> {
    let positive = positive(settings, query);
    if positive.is_empty() {
        return Ok(Vec::new());
    }
    let fts = sql.ident(&format!("{table}_fts"));
    settings.index_time.searchable_attributes.iter().enumerate().map(|(column, attribute)| {
        if let Some(tokens)=crop.get(&attribute.name){
            return snippet(table,settings,query,&attribute.name,*tokens,sql).map(|expression|(attribute.name.clone(),expression));
        }
        let column = sql.bind(column as i32);
        let start = sql.bind("\u{001e}".to_owned());
        let end = sql.bind("\u{001f}".to_owned());
        let value = sql.bind(positive.clone());
        Ok((attribute.name.clone(), format!("(SELECT highlight({fts}, {column}, {start}, {end}) FROM {fts} WHERE rowid=s.rowid AND {fts} MATCH {value})")))
    }).collect::<Result<Vec<_>,FrameworkError>>()
}

/// Snippets use FTS5's native token window while the caller escapes sentinel output.
pub(super) fn snippet(
    table: &str,
    settings: &IndexSettings,
    query: &ExpandedQuery,
    attribute: &str,
    tokens: usize,
    sql: &mut Sql,
) -> Result<String, FrameworkError> {
    let column = settings.index_time.searchable_attributes.iter().position(|a| a.name == attribute)
        .ok_or_else(|| FrameworkError::param(format!("search table {table} attribute {attribute}: snippets require a searchable attribute")))?;
    let positive = positive(settings, query);
    if positive.is_empty() {
        return Ok(format!("s.{}", sql.ident(attribute)));
    }
    let fts = sql.ident(&format!("{table}_fts"));
    let column = sql.bind(column as i32);
    let start = sql.bind("\u{001e}".to_owned());
    let end = sql.bind("\u{001f}".to_owned());
    let marker = sql.bind(settings.query.crop_marker.clone());
    let count = sql.bind(tokens.clamp(1, 64) as i32);
    let value = sql.bind(positive);
    Ok(format!(
        "(SELECT snippet({fts}, {column}, {start}, {end}, {marker}, {count}) FROM {fts} WHERE rowid=s.rowid AND {fts} MATCH {value})"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sqlite_alternatives_keep_prefix_on_original() {
        let term = ExpandedTerm {
            original: "rust".into(),
            alternatives: vec!["rust".into(), "trust".into(), "rust language".into()],
            prefix: true,
            typo_alternatives: vec!["trust".into()],
        };
        assert_eq!(
            super::term(&term, false),
            "(\"rust\"* OR \"trust\" OR \"rust language\")"
        );
        assert_eq!(super::term(&term, true), "(\"rust\"* OR \"rust language\")");
    }
    #[test]
    fn sqlite_rejects_unknown_tokenizer() {
        let mut settings = IndexSettings::default();
        settings.index_time.language = LanguageConfig::Sqlite {
            tokenizer: "unicode61'; DROP TABLE users".into(),
        };
        assert!(ddl("search_books_v1", &settings).is_err());
    }
}
