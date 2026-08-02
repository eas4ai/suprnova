//! Chain-flattened catalog loading — `FluentTranslator` builds each
//! locale's served catalog as a fold through `super::merge`'s AST-level
//! merge, lowest priority first: the embedded `en` validation catalog
//! for `en`/`en-*`, overridden by the locale's configured fallback
//! parent chain (`LocalizationConfig::parents`), overridden in turn by
//! the locale's own app files. Translator-level proof that the
//! flattening actually happens; the AST merge contract itself is
//! pinned in `framework/src/localization/merge.rs`'s own
//! `#[cfg(test)]` module, and the cycle-detection algorithm in
//! `framework/src/localization/config.rs`'s.
//!
//! Fixture locales are real BCP-47 (`Locale::parse` rejects bare
//! single-letter subtags), following the same convention documented in
//! `config.rs`'s `parse_parents_rejects_cycles` test.

#![cfg(feature = "localization")]

use std::fs;
use suprnova::{FluentTranslator, Locale, LocalizationConfig, TranslateArgs, Translator};

fn config() -> LocalizationConfig {
    LocalizationConfig {
        default_locale: Locale::parse("en").unwrap(),
        fallback_locale: Locale::parse("en").unwrap(),
        use_isolating: false,
        detection: vec![],
        session_key: "locale".into(),
        cookie_name: "locale".into(),
        parents: Default::default(),
    }
}

fn write_lang(dir: &std::path::Path, locale: &str, file: &str, ftl: &str) {
    let d = dir.join(locale);
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join(file), ftl).unwrap();
}

fn locale(s: &str) -> Locale {
    Locale::parse(s).unwrap()
}

#[test]
fn a_child_locale_inherits_and_overrides_its_parent() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(
        tmp.path(),
        "pt-BR",
        "app.ftl",
        "file = arquivo\nshared = comum\n",
    );
    write_lang(tmp.path(), "pt-PT", "app.ftl", "file = ficheiro\n");

    let cfg = config().parent(locale("pt-PT"), locale("pt-BR"));
    let t = FluentTranslator::from_dir(tmp.path(), &cfg).unwrap();
    let pt_pt = locale("pt-PT");
    let pt_br = locale("pt-BR");

    // The child's own override wins...
    assert_eq!(
        t.translate(&pt_pt, "file", &TranslateArgs::new()).unwrap(),
        "ficheiro"
    );
    // ...and a key the child never mentions still resolves through the
    // parent.
    assert_eq!(
        t.translate(&pt_pt, "shared", &TranslateArgs::new())
            .unwrap(),
        "comum"
    );

    let catalog = t.catalog(&pt_pt).unwrap();
    assert!(
        catalog.text.contains("file = ficheiro"),
        "flattened catalog missing the override: {}",
        catalog.text
    );
    assert!(
        catalog.text.contains("shared = comum"),
        "flattened catalog missing the inherited key: {}",
        catalog.text
    );

    // The child's catalog is a distinct, larger document than the
    // parent's own — not just an alias to it.
    let parent_hash = t.catalog(&pt_br).unwrap().hash;
    assert_ne!(catalog.hash, parent_hash);
}

#[test]
fn a_three_level_chain_flattens_transitively() {
    let tmp = tempfile::tempdir().unwrap();
    // de (root) -> de-AT (mid) -> de-CH (leaf), each defining one
    // distinct key plus a `shared` key the leaf overrides.
    write_lang(
        tmp.path(),
        "de",
        "app.ftl",
        "root-only = Root\nshared = R\n",
    );
    write_lang(
        tmp.path(),
        "de-AT",
        "app.ftl",
        "mid-only = Mid\nshared = M\n",
    );
    write_lang(
        tmp.path(),
        "de-CH",
        "app.ftl",
        "leaf-only = Leaf\nshared = L\n",
    );

    let cfg = config()
        .parent(locale("de-AT"), locale("de"))
        .parent(locale("de-CH"), locale("de-AT"));
    let t = FluentTranslator::from_dir(tmp.path(), &cfg).unwrap();
    let leaf = locale("de-CH");

    assert_eq!(
        t.translate(&leaf, "root-only", &TranslateArgs::new())
            .unwrap(),
        "Root"
    );
    assert_eq!(
        t.translate(&leaf, "mid-only", &TranslateArgs::new())
            .unwrap(),
        "Mid"
    );
    assert_eq!(
        t.translate(&leaf, "leaf-only", &TranslateArgs::new())
            .unwrap(),
        "Leaf"
    );
    // Leaf-most wins on the key every level defines.
    assert_eq!(
        t.translate(&leaf, "shared", &TranslateArgs::new()).unwrap(),
        "L"
    );
}

/// CRITICAL regression: the embedded `en` validation catalog must sit
/// at the *bottom* of the merge priority stack, not get re-merged over
/// the parent's already-resolved fold. Before this fix, an `en`-family
/// child (`en-AU`) re-parsed the raw embedded catalog fresh and merged
/// it *over* its parent's fold, so an app's override of an embedded id
/// in `lang/en/*.ftl` translated correctly for `en` but silently
/// reverted to the framework default for `en-AU` — the override was
/// masked by the child's own copy of the untouched embedded text.
#[test]
fn an_en_family_child_inherits_an_overridden_embedded_id_through_its_parent() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(
        tmp.path(),
        "en",
        "validation.ftl",
        "validation-required = CUSTOM\n",
    );
    // No `en-AU` directory — it inherits purely through the parent chain.

    let cfg = config().parent(locale("en-AU"), locale("en"));
    let t = FluentTranslator::from_dir(tmp.path(), &cfg).unwrap();
    let en_au = locale("en-AU");

    assert_eq!(
        t.translate(&en_au, "validation-required", &TranslateArgs::new())
            .unwrap(),
        "CUSTOM",
        "an app override of an embedded id must survive through the parent chain, \
         not be re-masked by the child's own fresh copy of the raw embedded default"
    );

    // The embedded catalog's standalone header comment must not be
    // duplicated once per `en`-family level in the chain.
    let catalog = t.catalog(&en_au).unwrap();
    let header_count = catalog
        .text
        .matches("Framework validation messages")
        .count();
    assert!(
        header_count <= 1,
        "the embedded header comment must not be duplicated per chain level: \
         found {header_count} times in {}",
        catalog.text
    );
}

#[test]
fn a_configured_child_with_no_directory_is_materialized() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(tmp.path(), "en", "app.ftl", "hello = Hello\n");
    // No `en-AU` directory at all.

    let cfg = config().parent(locale("en-AU"), locale("en"));
    let t = FluentTranslator::from_dir(tmp.path(), &cfg).unwrap();
    let en_au = locale("en-AU");

    assert!(
        t.available_locales().contains(&en_au),
        "a configured fallback child must be materialized even without its own directory"
    );
    assert_eq!(
        t.translate(&en_au, "hello", &TranslateArgs::new()).unwrap(),
        "Hello",
        "the materialized child must inherit its parent's keys"
    );
    assert!(t.catalog(&en_au).is_some());
}

#[tokio::test]
#[tracing_test::traced_test]
async fn a_missing_parent_warns_but_boots() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(tmp.path(), "pt-PT", "app.ftl", "hello = Ola\n");
    // No `pt-BR` directory, and `pt-BR` has no parent of its own either.

    let cfg = config().parent(locale("pt-PT"), locale("pt-BR"));
    let t = FluentTranslator::from_dir(tmp.path(), &cfg)
        .expect("a dangling fallback parent must not fail the load");
    let pt_pt = locale("pt-PT");

    assert_eq!(
        t.translate(&pt_pt, "hello", &TranslateArgs::new()).unwrap(),
        "Ola",
        "the child must still translate its own keys"
    );

    assert!(
        logs_contain("pt-BR"),
        "a dangling configured parent must be warned about by name"
    );
}

#[test]
fn a_parent_map_cycle_fails_the_load() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(tmp.path(), "pt-PT", "app.ftl", "hello = Ola\n");
    write_lang(tmp.path(), "pt-BR", "app.ftl", "hello = Ola\n");

    // Bypass `parse_parents` (which would reject this) by inserting the
    // cycle straight into the map — `LocalizationConfig::parents` is
    // `pub`, and `from_dir` must defend itself regardless of how a
    // cyclic map was constructed.
    let mut cfg = config();
    cfg.parents.insert(locale("pt-PT"), locale("pt-BR"));
    cfg.parents.insert(locale("pt-BR"), locale("pt-PT"));

    let err = FluentTranslator::from_dir(tmp.path(), &cfg)
        .expect_err("a cyclic parent map must fail the load");
    let message = format!("{err}");
    assert!(
        message.contains("pt-PT") && message.contains("pt-BR") && message.contains("->"),
        "error must name the cycle's walk path: {message}"
    );
}

#[test]
fn editing_a_parent_regenerates_the_child_on_reload() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(tmp.path(), "pt-BR", "app.ftl", "shared = comum\n");
    write_lang(tmp.path(), "pt-PT", "app.ftl", "file = ficheiro\n");

    let cfg = config().parent(locale("pt-PT"), locale("pt-BR"));
    let t = FluentTranslator::from_dir(tmp.path(), &cfg).unwrap();
    let pt_pt = locale("pt-PT");

    assert_eq!(
        t.translate(&pt_pt, "shared", &TranslateArgs::new())
            .unwrap(),
        "comum"
    );
    let hash_before = t.catalog(&pt_pt).unwrap().hash;

    write_lang(tmp.path(), "pt-BR", "app.ftl", "shared = mudou\n");
    t.reload().unwrap();

    assert_eq!(
        t.translate(&pt_pt, "shared", &TranslateArgs::new())
            .unwrap(),
        "mudou",
        "editing the parent must regenerate the flattened child catalog on reload"
    );
    let hash_after = t.catalog(&pt_pt).unwrap().hash;
    assert_ne!(
        hash_before, hash_after,
        "the child's catalog hash must change when its parent's content changes"
    );
}

/// The failure the AST merge module exists to fix: message-level
/// shadowing (what plain `add_resource_overriding` across two resources
/// does) would drop `.hint` entirely when `b.ftl` redefines `field`
/// without it. Now that intra-locale merging also folds through
/// `super::merge`, the attribute must survive.
#[test]
fn intra_locale_merge_preserves_unmentioned_attributes() {
    let tmp = tempfile::tempdir().unwrap();
    // "Antigo" and "Renomeado" share no substring, unlike the original
    // "Nome"/"Renomeado" fixture (whose `!contains("Nome")` assertion
    // below only held because "Renomeado" happens to contain lowercase
    // "nome", not "Nome" — a coincidence that could go vacuous on a
    // future fixture edit). A distinct sentinel makes the negative
    // assertion robust by construction, not by accident.
    write_lang(
        tmp.path(),
        "es",
        "a.ftl",
        "field = Antigo\n    .hint = Um\n",
    );
    write_lang(tmp.path(), "es", "b.ftl", "field = Renomeado\n");

    let t = FluentTranslator::from_dir(tmp.path(), &config()).unwrap();
    let es = locale("es");

    let catalog = t.catalog(&es).unwrap();
    assert!(
        catalog.text.contains("field = Renomeado"),
        "the later file's value must win: {}",
        catalog.text
    );
    assert!(
        catalog.text.contains(".hint = Um"),
        "an attribute the overriding file never mentions must survive: {}",
        catalog.text
    );
    // The two `field` entries must have been merged into one, not just
    // concatenated — the superseded value must not survive verbatim
    // alongside the override (a raw-concatenation catalog would contain
    // both `field = Antigo` and `field = Renomeado`).
    assert!(
        !catalog.text.contains("Antigo"),
        "the parent file's superseded value must not survive the merge: {}",
        catalog.text
    );
}

/// Documented behavior change from v1.0.0 (not a regression): under
/// plain `add_resource_overriding`, a later resource redefining
/// `field` with attributes but no value shadowed the whole message, so
/// `translate("field")` used to err "has no value". Intra-locale
/// merging now folds through `super::merge` too, whose contract is
/// that a value-less child keeps the parent's value (see `merge.rs`'s
/// `merge_message` doc and its `a_named_attribute_is_replaced_in_the_
/// parents_position` test) — so the same fixture now resolves. Pinned
/// here (empty `parents`, so this is purely the intra-locale fold, not
/// the parent-chain one) so the changelog task can pick up the
/// behavior change.
#[test]
fn a_later_file_adding_only_attributes_inherits_the_earlier_files_value() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(tmp.path(), "es", "a.ftl", "field = X\n");
    write_lang(tmp.path(), "es", "b.ftl", "field =\n    .hint = Y\n");

    let t = FluentTranslator::from_dir(tmp.path(), &config()).unwrap();
    let es = locale("es");

    assert_eq!(
        t.translate(&es, "field", &TranslateArgs::new()).unwrap(),
        "X",
        "a later file with attributes but no value must inherit the earlier value, \
         not blank out the whole message"
    );
    let catalog = t.catalog(&es).unwrap();
    assert!(
        catalog.text.contains(".hint = Y"),
        "the later file's attribute must be present: {}",
        catalog.text
    );
}

#[test]
fn a_malformed_ftl_file_still_fails_loudly_naming_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(
        tmp.path(),
        "en",
        "bad.ftl",
        "this is not = = valid ftl {{{\n",
    );
    let err = FluentTranslator::from_dir(tmp.path(), &config()).unwrap_err();
    assert!(
        err.to_string().contains("bad.ftl"),
        "error must name the file: {err}"
    );
}
