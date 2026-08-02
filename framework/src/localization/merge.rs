//! AST-level merge of two Fluent catalogs into one flattened catalog.
//!
//! Some locales differ from a sibling in vocabulary rather than in
//! substance — European Portuguese and Brazilian Portuguese share most of
//! a catalog and diverge on a handful of words (`ficheiro`/`arquivo`,
//! `utilizador`/`usuário`, `tu`/`você`). Translating both catalogs in
//! full would mean maintaining the same strings twice forever, and every
//! new string would need both. This module merges a parent catalog with
//! a child delta into one complete catalog at the AST level, so a
//! fallback chain (`pt-PT` → `pt-BR` → `en`) can be flattened to a single
//! resolved catalog per locale ahead of time rather than resolved key by
//! key at request time. See `manual/localization.md` for the user-facing
//! contract.
//!
//! # The merge contract
//!
//! - **The override unit is the pattern.** A child's value pattern
//!   replaces the parent's value pattern. Each child attribute replaces
//!   the same-named parent attribute. Parent attributes the child does
//!   not mention are retained. A child message with attributes but no
//!   value keeps the parent's value.
//! - **Patterns are replaced wholesale.** A select expression lives
//!   inside a pattern and goes with it; variants are never merged one by
//!   one. That is not just fragility avoidance — CLDR plural categories
//!   are locale dependent, so a variant-merged selector is semantically
//!   incoherent.
//! - **Terms follow the same rule.** (`Term::value` is a `Pattern`, not
//!   an `Option`, so the "attributes but no value" case cannot arise
//!   there.)
//! - **Messages and terms are separate namespaces.** `-brand` is not
//!   `brand`; overriding one must never suppress or shadow the other, so
//!   each is tracked with its own used-set.
//! - **Entry order is the parent's.** An overridden entry stays in its
//!   parent position carrying the merged value and attributes; entries
//!   only the child defines append at the end in child order.
//! - **Comments belong to the parent entry.** The override unit is the
//!   pattern, and a comment is not one — a child's comment on an
//!   overridden entry is dropped; the parent's is kept.
//! - **Serialization goes through `fluent_syntax::serializer`** with
//!   `Options::default()` (`with_junk: false`) — the single knob on the
//!   serializer, and therefore the only way two correct implementations
//!   could still disagree on bytes.
//! - **Parse errors are fatal.** `fluent_syntax::parser::parse` hands
//!   back a recovered `Resource` alongside the errors; accepting it
//!   would silently produce a catalog quietly missing whatever failed to
//!   parse. [`parse_strict`] refuses it.

use crate::error::FrameworkError;
use fluent_syntax::ast::{Attribute, Entry, Message, Resource, Term};
use std::collections::{HashMap, HashSet};

/// Parse `source` strictly: any parser error is `Err`, never the
/// recovered-with-`Junk` resource that `fluent_syntax::parser::parse`
/// also returns on failure. `origin` names the source in the error
/// message — a file path, a `"<locale>/<domain>"` label, or similar —
/// so a caller merging many catalogs can tell which one was malformed.
pub(crate) fn parse_strict(source: &str, origin: &str) -> Result<Resource<String>, FrameworkError> {
    fluent_syntax::parser::parse(source.to_string()).map_err(|(_, errors)| {
        let detail = errors
            .iter()
            .map(|e| format!("{e:?}"))
            .collect::<Vec<_>>()
            .join("; ");
        FrameworkError::param(format!("{origin} is not valid FTL: {detail}"))
    })
}

/// An empty catalog — the identity element for [`merge`]: merging it
/// with any resource on either side returns the other resource, modulo
/// serializer normalization (whitespace, blank-line collapsing).
pub(crate) fn empty() -> Resource<String> {
    Resource { body: Vec::new() }
}

/// Merge `child` over `parent` per the contract documented at module
/// level. Infallible — both resources have already been parsed via
/// [`parse_strict`], so there is nothing left that can fail.
pub(crate) fn merge(parent: &Resource<String>, child: &Resource<String>) -> Resource<String> {
    // Index the child's messages and terms. Terms and messages live in
    // separate namespaces in Fluent (`-brand` is not `brand`), so keying
    // by id alone would let one shadow the other.
    let mut child_messages: HashMap<&str, &Message<String>> = HashMap::new();
    let mut child_terms: HashMap<&str, &Term<String>> = HashMap::new();
    for entry in &child.body {
        match entry {
            Entry::Message(m) => {
                child_messages.insert(m.id.name.as_str(), m);
            }
            Entry::Term(t) => {
                child_terms.insert(t.id.name.as_str(), t);
            }
            _ => {}
        }
    }

    // Two separate used-sets for the same reason: a term overriding a
    // parent term must never mark a same-named message as "already
    // emitted" (and vice versa), or the child-only append pass below
    // would silently drop it.
    let mut used_messages: HashSet<&str> = HashSet::new();
    let mut used_terms: HashSet<&str> = HashSet::new();
    let mut body: Vec<Entry<String>> = Vec::with_capacity(parent.body.len());

    for entry in &parent.body {
        match entry {
            Entry::Message(pm) => match child_messages.get(pm.id.name.as_str()) {
                Some(cm) => {
                    used_messages.insert(pm.id.name.as_str());
                    body.push(Entry::Message(merge_message(pm, cm)));
                }
                // Not overridden: the parent entry passes through
                // untouched, comment and all.
                None => body.push(entry.clone()),
            },
            Entry::Term(pt) => match child_terms.get(pt.id.name.as_str()) {
                Some(ct) => {
                    used_terms.insert(pt.id.name.as_str());
                    body.push(Entry::Term(merge_term(pt, ct)));
                }
                None => body.push(entry.clone()),
            },
            other => body.push(other.clone()),
        }
    }

    // Anything the child introduced that the parent never had, in child
    // order. Comments attached to a child-only message or term travel
    // with it, since they live on the `Message`/`Term` node itself.
    for entry in &child.body {
        let already = match entry {
            Entry::Message(m) => used_messages.contains(m.id.name.as_str()),
            Entry::Term(t) => used_terms.contains(t.id.name.as_str()),
            _ => false,
        };
        if !already {
            body.push(entry.clone());
        }
    }

    Resource { body }
}

/// Value pattern replaced if the child has one; attributes replaced by
/// name with the parent's order preserved and child-only attributes
/// appended; comment stays the parent's.
fn merge_message(parent: &Message<String>, child: &Message<String>) -> Message<String> {
    Message {
        id: parent.id.clone(),
        // A child entry with attributes but no value keeps the parent's
        // value — the override unit is the pattern, and no pattern was
        // given.
        value: child.value.clone().or_else(|| parent.value.clone()),
        attributes: merge_attributes(&parent.attributes, &child.attributes),
        comment: parent.comment.clone(),
    }
}

/// As [`merge_message`], but the child's value always wins: `Term::value`
/// is a `Pattern`, not an `Option`, so there is no "attributes but no
/// value" case that would need the parent's value kept.
fn merge_term(parent: &Term<String>, child: &Term<String>) -> Term<String> {
    Term {
        id: parent.id.clone(),
        value: child.value.clone(),
        attributes: merge_attributes(&parent.attributes, &child.attributes),
        comment: parent.comment.clone(),
    }
}

/// The parent-order-with-append fold shared by [`merge_message`] and
/// [`merge_term`]: a child attribute of the same name replaces the
/// parent's in place; a child-only attribute appends after the parent's
/// own, in child order.
fn merge_attributes(
    parent: &[Attribute<String>],
    child: &[Attribute<String>],
) -> Vec<Attribute<String>> {
    let mut merged: Vec<Attribute<String>> = parent
        .iter()
        .map(
            |attribute| match child.iter().find(|c| c.id.name == attribute.id.name) {
                // Same name: the child's pattern replaces the parent's,
                // in the parent's position.
                Some(override_attribute) => override_attribute.clone(),
                None => attribute.clone(),
            },
        )
        .collect();

    for attribute in child {
        if !parent.iter().any(|p| p.id.name == attribute.id.name) {
            merged.push(attribute.clone());
        }
    }

    merged
}

/// Serialize a merged resource back to FTL bytes. Always goes through
/// `Options::default()` (`with_junk: false`) — the single serializer
/// knob, and therefore the only way two correct implementations could
/// still disagree on bytes for the same AST.
pub(crate) fn serialize(resource: &Resource<String>) -> String {
    fluent_syntax::serializer::serialize_with_options(
        resource,
        fluent_syntax::serializer::Options::default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merged(parent: &str, child: &str) -> String {
        let parent = parse_strict(parent, "parent.ftl").expect("parent should parse");
        let child = parse_strict(child, "child.ftl").expect("child should parse");
        serialize(&merge(&parent, &child))
    }

    #[test]
    fn a_child_value_replaces_the_parent_value() {
        let out = merged("greeting = Olá\n", "greeting = Viva\n");
        assert_eq!(out, "greeting = Viva\n");
    }

    /// The failure that motivated the whole contract: message-level
    /// shadowing (what `fluent_bundle::add_resource_overriding` does)
    /// would drop the placeholder and the page would render an input
    /// with no hint.
    #[test]
    fn attributes_the_child_does_not_mention_survive() {
        let parent = "resource-url = URL\n    .placeholder = https://exemplo.com\n    .aria-label = URL do recurso\n";
        let out = merged(parent, "resource-url = Endereço\n");
        assert!(
            out.contains("resource-url = Endereço"),
            "value replaced: {out}"
        );
        assert!(
            out.contains(".placeholder = https://exemplo.com"),
            "inherited placeholder missing: {out}"
        );
        assert!(
            out.contains(".aria-label = URL do recurso"),
            "inherited aria-label missing: {out}"
        );
    }

    #[test]
    fn a_named_attribute_is_replaced_in_the_parents_position() {
        let parent = "field = Ficheiro\n    .hint = Primeiro\n    .error = Segundo\n";
        let child = "field =\n    .hint = Alterado\n";
        let out = merged(parent, child);
        // Value untouched (the child gave no value), hint replaced where
        // it stood, error retained after it.
        assert_eq!(
            out,
            "field = Ficheiro\n    .hint = Alterado\n    .error = Segundo\n"
        );
    }

    #[test]
    fn a_child_only_attribute_appends() {
        let parent = "field = Nome\n    .hint = Um\n";
        let child = "field =\n    .aria-label = Dois\n";
        let out = merged(parent, child);
        assert_eq!(
            out,
            "field = Nome\n    .hint = Um\n    .aria-label = Dois\n"
        );
    }

    /// Whole-pattern replacement. Merging variant-by-variant would be
    /// incoherent: `pt` and `pt-PT` do not share CLDR plural boundaries,
    /// so a half-inherited selector describes no locale's grammar.
    #[test]
    fn a_selector_is_replaced_whole_not_variant_by_variant() {
        let parent = "saves =\n    { $n ->\n        [one] { $n } gravação\n       *[other] { $n } gravações\n    }\n";
        let child = "saves =\n    { $n ->\n       *[other] { $n } guardados\n    }\n";
        let out = merged(parent, child);
        assert!(out.contains("guardados"), "child variant missing: {out}");
        assert!(
            !out.contains("gravação"),
            "parent variant leaked into a replaced pattern: {out}"
        );
    }

    #[test]
    fn parent_entry_order_holds_and_child_only_entries_append() {
        let parent = "a = A\nb = B\nc = C\n";
        let child = "c = C2\nz = Z\na = A2\n";
        let out = merged(parent, child);
        assert_eq!(out, "a = A2\nb = B\nc = C2\nz = Z\n");
    }

    #[test]
    fn a_parent_comment_stays_with_its_entry() {
        let parent = "# Explains the string\ngreeting = Olá\n";
        let out = merged(parent, "greeting = Viva\n");
        assert_eq!(out, "# Explains the string\ngreeting = Viva\n");
    }

    #[test]
    fn terms_merge_by_the_same_rule_and_do_not_collide_with_messages() {
        let parent = "-brand = Marca\nbrand = Mensagem\n";
        let out = merged(parent, "-brand = Marca PT\n");
        assert!(out.contains("-brand = Marca PT"), "{out}");
        // The message of the same name is a different namespace and must
        // not have been touched by the term override.
        assert!(out.contains("brand = Mensagem"), "{out}");
    }

    #[test]
    fn a_malformed_delta_is_an_error_not_a_short_catalog() {
        let err = parse_strict("this is not = = valid ftl {\n", "child.ftl")
            .expect_err("malformed FTL must fail loudly");
        let message = format!("{err}");
        assert!(
            message.contains("not valid FTL"),
            "unexpected error: {message}"
        );
        assert!(
            message.contains("child.ftl"),
            "error does not name its origin: {message}"
        );
    }

    /// A child message overriding a parent term of the same name must not
    /// suppress that message from the child-only append pass — messages
    /// and terms are separate namespaces, and each needs its own
    /// used-set to keep that true.
    #[test]
    fn a_new_child_message_survives_overriding_a_term_of_the_same_name() {
        let parent = "-brand = Marca\n";
        let child = "-brand = Marca PT\nbrand = Nova mensagem\n";
        let out = merged(parent, child);
        assert!(out.contains("-brand = Marca PT"), "{out}");
        assert!(out.contains("brand = Nova mensagem"), "{out}");
    }

    #[test]
    fn a_child_comment_is_dropped_on_an_overridden_entry() {
        let out = merged("greeting = Olá\n", "# nova\ngreeting = Viva\n");
        assert_eq!(out, "greeting = Viva\n");
    }

    /// A comment attached to a child-only entry travels with it into the
    /// child-only append region, in child order — the append pass clones
    /// child entries whole, and a comment directly above a message is
    /// part of that message's own `comment` field at parse time.
    #[test]
    fn child_standalone_comments_append_after_child_only_entries_region() {
        let out = merged("a = A\n", "# Delta header\nz = Z\n");
        assert_eq!(out, "a = A\n# Delta header\nz = Z\n");
    }

    #[test]
    fn merging_with_an_empty_parent_or_child_is_identity_modulo_normalization() {
        assert_eq!(merged("", "a = A\n"), "a = A\n");
        assert_eq!(merged("a = A\n", ""), "a = A\n");
    }

    #[test]
    fn a_malformed_parent_is_also_fatal() {
        let err = parse_strict("this is not = = valid ftl {\n", "parent.ftl")
            .expect_err("malformed FTL must fail loudly");
        let message = format!("{err}");
        assert!(
            message.contains("not valid FTL"),
            "unexpected error: {message}"
        );
        assert!(
            message.contains("parent.ftl"),
            "error does not name its origin: {message}"
        );
    }
}
