//! Qdrant predicates keep AND between words and OR within each expansion group.
use super::*;
use qdrant_client::qdrant::{
    Condition, DatetimeRange, Filter as QFilter, GeoPoint as QGeo, GeoRadius, Range,
};

fn negate(condition: Condition) -> Condition {
    QFilter::must_not([condition]).into()
}
fn null(key: &str) -> Condition {
    QFilter::should([
        Condition::is_null(key),
        QFilter::must([
            Condition::is_empty(key),
            negate(Condition::matches("__present", key.to_owned())),
        ])
        .into(),
    ])
    .into()
}
fn equality(key: &str, value: &AttributeValue) -> Result<Condition, FrameworkError> {
    Ok(match value {
        AttributeValue::Null => null(key),
        AttributeValue::Text(v) => Condition::matches(key, v.clone()),
        AttributeValue::Integer(v) => Condition::matches(key, *v),
        AttributeValue::Bool(v) => Condition::matches(key, *v),
        AttributeValue::Float(v) => Condition::range(
            key,
            Range {
                gte: Some(*v),
                lte: Some(*v),
                ..Default::default()
            },
        ),
        AttributeValue::DateTime(_) | AttributeValue::Geo(_) => {
            Condition::matches(format!("__value_{key}"), value.facet_key())
        }
        AttributeValue::List(_) => Condition::matches(
            format!("__value_{key}"),
            crate::search::document::canonical_json(value.to_json()),
        ),
    })
}
fn membership(key: &str, values: &[AttributeValue]) -> Result<Condition, FrameworkError> {
    if values.is_empty() {
        return Ok(negate(QFilter::default().into()));
    }
    if values.iter().all(|v| matches!(v, AttributeValue::Text(_))) {
        return Ok(Condition::matches(
            key,
            values
                .iter()
                .filter_map(|v| v.as_text().map(str::to_owned))
                .collect::<Vec<_>>(),
        ));
    }
    if values
        .iter()
        .all(|v| matches!(v, AttributeValue::Integer(_)))
    {
        return Ok(Condition::matches(
            key,
            values
                .iter()
                .filter_map(|v| {
                    if let AttributeValue::Integer(n) = v {
                        Some(*n)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>(),
        ));
    }
    Ok(QFilter::should(
        values
            .iter()
            .map(|v| equality(key, v))
            .collect::<Result<Vec<_>, _>>()?,
    )
    .into())
}
fn ordered(key: &str, value: &AttributeValue, op: &str) -> Result<Condition, FrameworkError> {
    if let AttributeValue::DateTime(v) = value {
        let t = Some(timestamp(v));
        return Ok(Condition::datetime_range(
            key,
            DatetimeRange {
                gt: if op == "gt" { t } else { None },
                gte: if op == "gte" { t } else { None },
                lt: if op == "lt" { t } else { None },
                lte: if op == "lte" { t } else { None },
            },
        ));
    }
    if let AttributeValue::Bool(v) = value {
        let accepted = [false, true]
            .into_iter()
            .filter(|n| match op {
                "gt" => n > v,
                "gte" => n >= v,
                "lt" => n < v,
                _ => n <= v,
            })
            .map(|n| Condition::matches(key, n))
            .collect::<Vec<_>>();
        return Ok(if accepted.is_empty() {
            negate(QFilter::default().into())
        } else {
            QFilter::should(accepted).into()
        });
    }
    let n = value.numeric().ok_or_else(|| {
        FrameworkError::param(format!(
            "Qdrant filter attribute {key}: ordered comparisons require numeric or datetime values"
        ))
    })?;
    Ok(Condition::range(
        key,
        Range {
            gt: (op == "gt").then_some(n),
            gte: (op == "gte").then_some(n),
            lt: (op == "lt").then_some(n),
            lte: (op == "lte").then_some(n),
        },
    ))
}
pub(super) fn predicate(filter: &Filter) -> Result<Condition, FrameworkError> {
    Ok(match filter {
        Filter::Eq(k, v) => equality(k, v)?,
        Filter::Ne(k, v) => negate(equality(k, v)?),
        Filter::In(k, v) => membership(k, v)?,
        Filter::NotIn(k, v) => negate(membership(k, v)?),
        Filter::Gt(k, v) => ordered(k, v, "gt")?,
        Filter::Gte(k, v) => ordered(k, v, "gte")?,
        Filter::Lt(k, v) => ordered(k, v, "lt")?,
        Filter::Lte(k, v) => ordered(k, v, "lte")?,
        Filter::Between(k, lo, hi) => {
            QFilter::must([ordered(k, lo, "gte")?, ordered(k, hi, "lte")?]).into()
        }
        Filter::IsNull(k) => null(k),
        Filter::IsNotNull(k) => negate(null(k)),
        Filter::GeoRadius(k, p, r) => Condition::geo_radius(
            k,
            GeoRadius {
                center: Some(QGeo {
                    lat: p.lat,
                    lon: p.lng,
                }),
                radius: (*r as f32).min(f32::MAX),
            },
        ),
        Filter::And(fs) => {
            QFilter::must(fs.iter().map(predicate).collect::<Result<Vec<_>, _>>()?).into()
        }
        Filter::Or(fs) => {
            if fs.is_empty() {
                negate(QFilter::default().into())
            } else {
                QFilter::should(fs.iter().map(predicate).collect::<Result<Vec<_>, _>>()?).into()
            }
        }
        Filter::Not(f) => negate(predicate(f)?),
    })
}
fn term_clause(
    field: &str,
    words: &str,
    term: &crate::search::ExpandedTerm,
    disabled: bool,
) -> Condition {
    let mut singles = Vec::new();
    let mut clauses = Vec::new();
    for alternative in term
        .alternatives
        .iter()
        .chain(std::iter::once(&term.original))
        .collect::<BTreeSet<_>>()
    {
        if disabled && term.typo_alternatives.contains(alternative) {
            continue;
        }
        if alternative.contains(' ') {
            clauses.push(Condition::matches_phrase(field, alternative.clone()));
        } else {
            singles.push(alternative.clone());
        }
    }
    if !singles.is_empty() {
        clauses.push(Condition::matches_text_any(field, singles.join(" ")));
    }
    if term.prefix {
        clauses.push(Condition::matches_prefix(words, term.original.clone()));
    }
    QFilter::should(clauses).into()
}
pub(super) fn lexical(
    settings: &IndexSettings,
    q: &EngineQuery,
) -> Result<QFilter, FrameworkError> {
    let mut must = Vec::new();
    if let Some(f) = &q.filter {
        must.push(predicate(f)?);
    }
    let disabled = &q.expanded.typo_disabled_attributes;
    for term in &q.expanded.terms {
        if disabled.is_empty()
            && settings
                .query
                .typo_tolerance
                .disable_on_attributes
                .is_empty()
            && !term
                .alternatives
                .iter()
                .chain(std::iter::once(&term.original))
                .any(|a| a.contains(' '))
        {
            must.push(term_clause("__text", "__words", term, false));
        } else {
            must.push(
                QFilter::should(
                    settings
                        .index_time
                        .searchable_attributes
                        .iter()
                        .map(|attr| {
                            term_clause(
                                &format!("__search_{}", attr.name),
                                &format!("__words_{}", attr.name),
                                term,
                                disabled.contains(&attr.name)
                                    || settings
                                        .query
                                        .typo_tolerance
                                        .disable_on_attributes
                                        .contains(&attr.name),
                            )
                        }),
                )
                .into(),
            );
        }
    }
    for phrase in &q.expanded.phrases {
        // Separate fields prevent an artificial phrase across attribute boundaries.
        must.push(
            QFilter::should(
                settings
                    .index_time
                    .searchable_attributes
                    .iter()
                    .map(|attr| {
                        Condition::matches_phrase(
                            format!("__search_{}", attr.name),
                            phrase.join(" "),
                        )
                    }),
            )
            .into(),
        );
    }
    let must_not = q
        .expanded
        .negatives
        .iter()
        .map(|word| Condition::matches_text("__text", word.clone()))
        .collect();
    Ok(QFilter {
        must,
        must_not,
        ..Default::default()
    })
}

fn timestamp(value: &chrono::DateTime<chrono::Utc>) -> qb::Timestamp {
    qb::Timestamp {
        seconds: value.timestamp(),
        nanos: value.timestamp_subsec_nanos() as i32,
    }
}
