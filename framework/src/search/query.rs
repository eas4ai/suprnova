//! Typed query plans keep driver implementations independent of request syntax.
use super::document::{AttributeType, AttributeValue, GeoPoint, SearchDocument};
use super::expansion::ExpandedQuery;
use super::registry::IndexDescriptor;
use crate::FrameworkError;
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, collections::BTreeMap};

/// Bound predicates prevent request values from becoming engine syntax.
///
/// ```
/// use suprnova::{Filter, AttributeValue};
/// let private = Filter::Eq("public".into(), AttributeValue::Bool(false));
/// let excluded = Filter::Not(Box::new(private));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Filter {
    /// Equality also matches an element of a list attribute.
    Eq(String, AttributeValue),
    /// Negated equality retains missing values unless null is compared.
    Ne(String, AttributeValue),
    /// Membership expresses alternatives without string interpolation.
    In(String, Vec<AttributeValue>),
    /// Exclusion expresses a bounded set of unwanted values.
    NotIn(String, Vec<AttributeValue>),
    /// Ordered comparison excludes missing values.
    Gt(String, AttributeValue),
    /// Inclusive ordered comparison excludes missing values.
    Gte(String, AttributeValue),
    /// Ordered comparison excludes missing values.
    Lt(String, AttributeValue),
    /// Inclusive ordered comparison excludes missing values.
    Lte(String, AttributeValue),
    /// An inclusive interval preserves the declared attribute type.
    Between(String, AttributeValue, AttributeValue),
    /// Missing and explicit null values share null semantics.
    IsNull(String),
    /// A presence predicate excludes explicit nulls.
    IsNotNull(String),
    /// Distance is expressed in meters on the Earth's surface.
    GeoRadius(String, GeoPoint, f64),
    /// All children must match, including an empty conjunction.
    And(Vec<Filter>),
    /// At least one child must match.
    Or(Vec<Filter>),
    /// Negation composes arbitrary bound predicates.
    Not(Box<Filter>),
}
impl Filter {
    /// Parse and bind against declarations before any engine receives the plan.
    pub fn parse(input: &str, descriptor: &IndexDescriptor) -> Result<Self, FrameworkError> {
        let mut parser = Parser {
            tokens: lex(input)?,
            at: 0,
            descriptor,
            depth: 0,
        };
        let filter = parser.or()?;
        if parser.at != parser.tokens.len() {
            return Err(FrameworkError::param("filter: unexpected trailing token"));
        }
        filter.validate(descriptor)?;
        Ok(filter)
    }
    /// Validate AST callers with the same boundary used by the grammar.
    pub fn validate(&self, descriptor: &IndexDescriptor) -> Result<(), FrameworkError> {
        self.validate_depth(descriptor, 0)
    }
    fn validate_depth(
        &self,
        descriptor: &IndexDescriptor,
        depth: usize,
    ) -> Result<(), FrameworkError> {
        if depth > 64 {
            return Err(FrameworkError::param("filter: nesting exceeds 64"));
        }
        match self {
            Self::And(children) | Self::Or(children) => {
                for child in children {
                    child.validate_depth(descriptor, depth + 1)?;
                }
            }
            Self::Not(child) => child.validate_depth(descriptor, depth + 1)?,
            Self::GeoRadius(attr, point, radius) => {
                let decl = filter_attribute(descriptor, attr)?;
                if decl.ty != AttributeType::Geo || !radius.is_finite() || *radius < 0.0 {
                    return Err(FrameworkError::param(format!(
                        "filter attribute {attr}: requires a geo point and finite nonnegative radius"
                    )));
                }
                point.validate()?;
            }
            Self::IsNull(attr) | Self::IsNotNull(attr) => {
                filter_attribute(descriptor, attr)?;
            }
            Self::In(attr, values) | Self::NotIn(attr, values) => {
                let decl = filter_attribute(descriptor, attr)?;
                for value in values {
                    validate_value(value, decl.ty, attr)?;
                }
            }
            Self::Between(attr, lo, hi) => {
                let decl = filter_attribute(descriptor, attr)?;
                validate_value(lo, decl.ty, attr)?;
                validate_value(hi, decl.ty, attr)?;
                if lo
                    .compare(hi)
                    .is_none_or(|order| order == Ordering::Greater)
                {
                    return Err(FrameworkError::param(format!(
                        "filter attribute {attr}: invalid range"
                    )));
                }
            }
            Self::Eq(attr, value)
            | Self::Ne(attr, value)
            | Self::Gt(attr, value)
            | Self::Gte(attr, value)
            | Self::Lt(attr, value)
            | Self::Lte(attr, value) => {
                let decl = filter_attribute(descriptor, attr)?;
                validate_value(value, decl.ty, attr)?;
                if matches!(
                    self,
                    Self::Gt(..) | Self::Gte(..) | Self::Lt(..) | Self::Lte(..)
                ) && matches!(
                    value,
                    AttributeValue::Null | AttributeValue::List(_) | AttributeValue::Geo(_)
                ) {
                    return Err(FrameworkError::param(format!(
                        "filter attribute {attr}: value is not ordered"
                    )));
                }
            }
        }
        Ok(())
    }
    /// Evaluate documents identically for collection queries and live-row scope checks.
    pub fn matches(&self, document: &SearchDocument) -> bool {
        let value = |attr: &str| {
            document
                .attributes
                .get(attr)
                .unwrap_or(&AttributeValue::Null)
        };
        match self {
            Self::Eq(a, v) => equal(value(a), v),
            Self::Ne(a, v) => !equal(value(a), v),
            Self::In(a, vs) => vs.iter().any(|v| equal(value(a), v)),
            Self::NotIn(a, vs) => !vs.iter().any(|v| equal(value(a), v)),
            Self::Gt(a, v) => value(a).compare(v) == Some(Ordering::Greater),
            Self::Gte(a, v) => matches!(
                value(a).compare(v),
                Some(Ordering::Greater | Ordering::Equal)
            ),
            Self::Lt(a, v) => value(a).compare(v) == Some(Ordering::Less),
            Self::Lte(a, v) => {
                matches!(value(a).compare(v), Some(Ordering::Less | Ordering::Equal))
            }
            Self::Between(a, lo, hi) => {
                matches!(
                    value(a).compare(lo),
                    Some(Ordering::Greater | Ordering::Equal)
                ) && matches!(value(a).compare(hi), Some(Ordering::Less | Ordering::Equal))
            }
            Self::IsNull(a) => matches!(value(a), AttributeValue::Null),
            Self::IsNotNull(a) => !matches!(value(a), AttributeValue::Null),
            Self::GeoRadius(a, p, r) => {
                matches!(value(a),AttributeValue::Geo(v) if v.distance_to(p)<=*r)
            }
            Self::And(fs) => fs.iter().all(|f| f.matches(document)),
            Self::Or(fs) => fs.iter().any(|f| f.matches(document)),
            Self::Not(f) => !f.matches(document),
        }
    }
}
fn equal(left: &AttributeValue, right: &AttributeValue) -> bool {
    match left {
        AttributeValue::List(values) if !matches!(right, AttributeValue::List(_)) => {
            values.iter().any(|v| equal(v, right))
        }
        _ => left == right,
    }
}
fn filter_attribute<'a>(
    descriptor: &'a IndexDescriptor,
    attr: &str,
) -> Result<&'a super::registry::AttributeDecl, FrameworkError> {
    let decl = descriptor.attribute(attr)?;
    if !decl.filterable {
        return Err(FrameworkError::param(format!(
            "filter attribute {attr}: is not filterable"
        )));
    }
    Ok(decl)
}
fn validate_value(
    value: &AttributeValue,
    ty: AttributeType,
    attr: &str,
) -> Result<(), FrameworkError> {
    let valid = matches!(
        (ty, value),
        (AttributeType::Text, AttributeValue::Text(_))
            | (AttributeType::Integer, AttributeValue::Integer(_))
            | (AttributeType::Float, AttributeValue::Float(_))
            | (AttributeType::Bool, AttributeValue::Bool(_))
            | (AttributeType::DateTime, AttributeValue::DateTime(_))
            | (AttributeType::Geo, AttributeValue::Geo(_))
            | (_, AttributeValue::Null)
            | (AttributeType::TextList, AttributeValue::Text(_))
    ) || matches!((ty,value),(AttributeType::TextList,AttributeValue::List(vs)) if vs.iter().all(|v| matches!(v,AttributeValue::Text(_))));
    if !valid || matches!(value,AttributeValue::Float(v) if !v.is_finite()) {
        return Err(FrameworkError::param(format!(
            "filter attribute {attr}: value has the wrong declared type"
        )));
    }
    Ok(())
}

/// Structured text separates literal phrases from independent word clauses.
///
/// ```
/// use suprnova::QueryText;
/// let query = QueryText::parse("apple -rotten");
/// assert_eq!(query.negatives, vec!["rotten"]);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QueryText {
    /// The original spelling is echoed without exposing engine syntax.
    pub original: String,
    /// Words become OR-expanded clauses joined by AND.
    pub words: Vec<String>,
    /// Phrase order and adjacency remain meaningful to each engine.
    pub phrases: Vec<Vec<String>>,
    /// Exclusions are not expanded into positive candidates.
    pub negatives: Vec<String>,
}
impl QueryText {
    /// Unmatched quotes and lone minus signs remain text rather than losing input.
    pub fn parse(input: &str) -> Self {
        let mut result = Self {
            original: input.into(),
            ..Self::default()
        };
        let mut rest = input;
        while !rest.is_empty() {
            rest = rest.trim_start();
            if rest.is_empty() {
                break;
            }
            if let Some(after) = rest.strip_prefix('"')
                && let Some(end) = after.find('"')
            {
                let phrase = after[..end]
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                if !phrase.is_empty() {
                    result.phrases.push(phrase);
                }
                rest = &after[end + 1..];
                continue;
            }
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let word = &rest[..end];
            if let Some(negative) = word.strip_prefix('-').filter(|s| !s.is_empty()) {
                result.negatives.push(negative.into());
            } else {
                result.words.push(word.into());
            }
            rest = &rest[end..];
        }
        result
    }
}
/// Explicit direction avoids driver-specific spelling in public queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortDirection {
    /// Lowest values appear first.
    Asc,
    /// Highest values appear first.
    Desc,
}
/// Sorting uses typed values and always places missing values last.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Sort {
    /// Order a declared sortable attribute.
    Attribute {
        /// Attribute names are validated before dispatch.
        attribute: String,
        /// The requested comparison order.
        direction: SortDirection,
    },
    /// Order by great-circle distance in meters.
    Distance {
        /// The geo attribute to measure.
        attribute: String,
        /// The origin of the distance measurement.
        point: GeoPoint,
        /// The requested distance order.
        direction: SortDirection,
    },
}
/// Grouping retains bounded additional members behind each representative.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupBy {
    /// The declaration whose value defines a group.
    pub attribute: String,
    /// Maximum hits retained in one group.
    pub per_group: usize,
}
/// A named reranker can replace the order of a bounded leading slice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rerank {
    /// Registered cross-encoder identity.
    pub model: String,
    /// Maximum candidates scored by that encoder.
    pub depth: usize,
}
/// A complete builder plan can be inspected before hydration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchQuery {
    /// Parsed text for expansion.
    pub text: QueryText,
    /// Caller predicates before viewer scope is added.
    pub filter: Option<Filter>,
    /// Requested facet distributions.
    pub facets: Vec<String>,
    /// Requested ordered facet ranges.
    pub facet_stats: Vec<String>,
    /// Primary ordering before relevance ties.
    pub sort: Vec<Sort>,
    /// Optional representative grouping.
    pub group: Option<GroupBy>,
    /// Attributes needing formatted matches.
    pub highlight: Vec<String>,
    /// Crop lengths in visible characters per attribute.
    pub crop: BTreeMap<String, usize>,
    /// Semantic weight, with zero retaining lexical order.
    pub hybrid_ratio: Option<f32>,
    /// Optional named final reordering step.
    pub rerank: Option<Rerank>,
    /// Page size after ranking and grouping.
    pub limit: usize,
    /// Number of ranked hits skipped.
    pub offset: usize,
    /// Optional bounded pagination universe.
    pub hits_cap: Option<usize>,
    /// Explicit soft-deletion opt-in.
    pub with_trashed: bool,
}
impl SearchQuery {
    /// Defaults keep a lexical query bounded until callers choose otherwise.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: QueryText::parse(&text.into()),
            filter: None,
            facets: vec![],
            facet_stats: vec![],
            sort: vec![],
            group: None,
            highlight: vec![],
            crop: BTreeMap::new(),
            hybrid_ratio: None,
            rerank: None,
            limit: 20,
            offset: 0,
            hits_cap: None,
            with_trashed: false,
        }
    }
}
/// Engines receive expanded clauses and a scope-inclusive filter, never raw grammar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineQuery {
    /// Positive alternatives, phrases and exclusions.
    pub expanded: ExpandedQuery,
    /// Includes both caller predicates and viewer scope.
    pub filter: Option<Filter>,
    /// Sorting precedes relevance in the engine candidate order.
    pub sort: Vec<Sort>,
    /// Maximum candidates retained for fusion.
    pub depth: usize,
    /// Allows engines with safe native formatting to supply it.
    pub highlights: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    Quoted(String),
    Symbol(char),
    Compare(String),
}
fn lex(input: &str) -> Result<Vec<Token>, FrameworkError> {
    if input.len() > 65536 {
        return Err(FrameworkError::param("filter: exceeds 65536 bytes"));
    }
    let mut chars = input.chars().peekable();
    let mut tokens = vec![];
    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        let token =
            match ch {
                '(' | ')' | '[' | ']' | ',' => Token::Symbol(ch),
                '=' | '!' | '<' | '>' => {
                    let mut op = ch.to_string();
                    if chars.peek() == Some(&'=') {
                        op.push('=');
                        chars.next();
                    }
                    if op == "!" {
                        return Err(FrameworkError::param("filter: expected !="));
                    }
                    Token::Compare(op)
                }
                '\'' | '"' => {
                    let mut value = String::new();
                    let mut closed = false;
                    while let Some(c) = chars.next() {
                        if c == ch {
                            closed = true;
                            break;
                        }
                        if c == '\\' {
                            value.push(chars.next().ok_or_else(|| {
                                FrameworkError::param("filter: unfinished escape")
                            })?);
                        } else {
                            value.push(c);
                        }
                    }
                    if !closed {
                        return Err(FrameworkError::param("filter: unclosed string"));
                    }
                    Token::Quoted(value)
                }
                _ => {
                    let mut word = ch.to_string();
                    while let Some(&c) = chars.peek() {
                        if c.is_whitespace() || "()[],=!<>\"'".contains(c) {
                            break;
                        }
                        word.push(c);
                        chars.next();
                    }
                    Token::Word(word)
                }
            };
        tokens.push(token);
        if tokens.len() > 4096 {
            return Err(FrameworkError::param("filter: exceeds 4096 tokens"));
        }
    }
    Ok(tokens)
}
struct Parser<'a> {
    tokens: Vec<Token>,
    at: usize,
    descriptor: &'a IndexDescriptor,
    depth: usize,
}
impl Parser<'_> {
    fn take(&mut self) -> Result<Token, FrameworkError> {
        let token = self
            .tokens
            .get(self.at)
            .cloned()
            .ok_or_else(|| FrameworkError::param("filter: unexpected end"))?;
        self.at += 1;
        Ok(token)
    }
    fn keyword(&mut self, w: &str) -> bool {
        if matches!(self.tokens.get(self.at),Some(Token::Word(v)) if v.eq_ignore_ascii_case(w)) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn symbol(&mut self, c: char) -> bool {
        if self.tokens.get(self.at) == Some(&Token::Symbol(c)) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn require_symbol(&mut self, c: char) -> Result<(), FrameworkError> {
        if self.symbol(c) {
            Ok(())
        } else {
            Err(FrameworkError::param(format!("filter: expected {c}")))
        }
    }
    fn or(&mut self) -> Result<Filter, FrameworkError> {
        let mut fs = vec![self.and()?];
        while self.keyword("OR") {
            fs.push(self.and()?);
        }
        if fs.len() == 1 {
            Ok(fs.remove(0))
        } else {
            Ok(Filter::Or(fs))
        }
    }
    fn and(&mut self) -> Result<Filter, FrameworkError> {
        let mut fs = vec![self.unary()?];
        while self.keyword("AND") {
            fs.push(self.unary()?);
        }
        if fs.len() == 1 {
            Ok(fs.remove(0))
        } else {
            Ok(Filter::And(fs))
        }
    }
    fn unary(&mut self) -> Result<Filter, FrameworkError> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(FrameworkError::param("filter: nesting exceeds 64"));
        }
        let result = if self.keyword("NOT") {
            self.unary().map(|f| Filter::Not(Box::new(f)))
        } else if self.symbol('(') {
            let f = self.or()?;
            self.require_symbol(')')?;
            Ok(f)
        } else {
            self.predicate()
        };
        self.depth -= 1;
        result
    }
    fn value(&mut self, attr: &str, ty: AttributeType) -> Result<AttributeValue, FrameworkError> {
        let token = self.take()?;
        let json = match token {
            Token::Quoted(v) => serde_json::Value::String(v),
            Token::Word(v) => {
                if v.eq_ignore_ascii_case("NULL") {
                    serde_json::Value::Null
                } else if v.eq_ignore_ascii_case("true") {
                    serde_json::Value::Bool(true)
                } else if v.eq_ignore_ascii_case("false") {
                    serde_json::Value::Bool(false)
                } else if let Ok(number) = v.parse::<serde_json::Number>() {
                    serde_json::Value::Number(number)
                } else if ty == AttributeType::DateTime {
                    serde_json::Value::String(v)
                } else {
                    return Err(FrameworkError::param(format!(
                        "filter attribute {attr}: expected a quoted string or typed value"
                    )));
                }
            }
            _ => {
                return Err(FrameworkError::param(format!(
                    "filter attribute {attr}: expected value"
                )));
            }
        };
        AttributeValue::from_json(
            &json,
            if ty == AttributeType::TextList {
                AttributeType::Text
            } else {
                ty
            },
            attr,
        )
    }
    fn number(&mut self) -> Result<f64, FrameworkError> {
        match self.take()? {
            Token::Word(v) => v
                .parse::<f64>()
                .map_err(|_| FrameworkError::param("filter _geoRadius: expected number")),
            _ => Err(FrameworkError::param("filter _geoRadius: expected number")),
        }
    }
    fn geo_radius(&mut self) -> Result<Filter, FrameworkError> {
        let attributes = self
            .descriptor
            .attributes
            .iter()
            .filter(|attribute| attribute.filterable && attribute.ty == AttributeType::Geo)
            .collect::<Vec<_>>();
        if attributes.len() != 1 {
            return Err(FrameworkError::param(
                "filter _geoRadius: requires exactly one filterable geo attribute",
            ));
        }
        let name = attributes[0].name.clone();
        self.require_symbol('(')?;
        let lat = self.number()?;
        self.require_symbol(',')?;
        let lng = self.number()?;
        self.require_symbol(',')?;
        let radius = self.number()?;
        self.require_symbol(')')?;
        Ok(Filter::GeoRadius(name, GeoPoint { lat, lng }, radius))
    }
    fn null_predicate(&mut self, attribute: String) -> Result<Filter, FrameworkError> {
        let not = self.keyword("NOT");
        if !self.keyword("NULL") {
            return Err(FrameworkError::param(format!(
                "filter attribute {attribute}: expected NULL"
            )));
        }
        Ok(if not {
            Filter::IsNotNull(attribute)
        } else {
            Filter::IsNull(attribute)
        })
    }
    fn membership(
        &mut self,
        attribute: String,
        ty: AttributeType,
        not: bool,
    ) -> Result<Filter, FrameworkError> {
        self.require_symbol('[')?;
        let mut values = Vec::new();
        if !self.symbol(']') {
            loop {
                values.push(self.value(&attribute, ty)?);
                if self.symbol(']') {
                    break;
                }
                self.require_symbol(',')?;
            }
        }
        Ok(if not {
            Filter::NotIn(attribute, values)
        } else {
            Filter::In(attribute, values)
        })
    }
    fn comparison(
        &mut self,
        attribute: String,
        ty: AttributeType,
        operator: &str,
    ) -> Result<Filter, FrameworkError> {
        let value = self.value(&attribute, ty)?;
        match operator {
            "=" | "==" => Ok(Filter::Eq(attribute, value)),
            "!=" => Ok(Filter::Ne(attribute, value)),
            ">" => Ok(Filter::Gt(attribute, value)),
            ">=" => Ok(Filter::Gte(attribute, value)),
            "<" => Ok(Filter::Lt(attribute, value)),
            "<=" => Ok(Filter::Lte(attribute, value)),
            _ => Err(FrameworkError::param(format!(
                "filter attribute {attribute}: invalid comparison"
            ))),
        }
    }
    fn predicate(&mut self) -> Result<Filter, FrameworkError> {
        let attribute = match self.take()? {
            Token::Word(attribute) => attribute,
            _ => return Err(FrameworkError::param("filter: expected attribute")),
        };
        if attribute.eq_ignore_ascii_case("_geoRadius") {
            return self.geo_radius();
        }
        let ty = filter_attribute(self.descriptor, &attribute)?.ty;
        if self.keyword("IS") {
            return self.null_predicate(attribute);
        }
        let not = self.keyword("NOT");
        if self.keyword("IN") {
            return self.membership(attribute, ty, not);
        }
        if not {
            return Err(FrameworkError::param(format!(
                "filter attribute {attribute}: expected IN after NOT"
            )));
        }
        if let Some(Token::Compare(operator)) = self.tokens.get(self.at).cloned() {
            self.at += 1;
            return self.comparison(attribute, ty, &operator);
        }
        let low = self.value(&attribute, ty)?;
        if !self.keyword("TO") {
            return Err(FrameworkError::param(format!(
                "filter attribute {attribute}: expected TO"
            )));
        }
        let high = self.value(&attribute, ty)?;
        Ok(Filter::Between(attribute, low, high))
    }
}
