//! The `#[serde(...)]` attributes `#[derive(InertiaProps)]` and
//! `#[derive(Data)]` honor, and the names they give each field.
//!
//! Both derives write their own `Serialize` (and `Data` its `Deserialize`),
//! so they read serde's attributes themselves. They support the renaming and
//! skipping attributes and refuse every other one: an attribute a derive
//! silently ignored would send keys the author did not ask for.

use syn::meta::ParseNestedMeta;
use syn::{Attribute, Field, LitStr};

/// Serde's `rename_all` rules, spelled and applied as serde spells and
/// applies them to field names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RenameRule {
    Lower,
    Upper,
    Pascal,
    Camel,
    Snake,
    ScreamingSnake,
    Kebab,
    ScreamingKebab,
}

/// The spellings serde accepts, in serde's order.
const RULES: &[(&str, RenameRule)] = &[
    ("lowercase", RenameRule::Lower),
    ("UPPERCASE", RenameRule::Upper),
    ("PascalCase", RenameRule::Pascal),
    ("camelCase", RenameRule::Camel),
    ("snake_case", RenameRule::Snake),
    ("SCREAMING_SNAKE_CASE", RenameRule::ScreamingSnake),
    ("kebab-case", RenameRule::Kebab),
    ("SCREAMING-KEBAB-CASE", RenameRule::ScreamingKebab),
];

impl RenameRule {
    fn parse(lit: &LitStr) -> syn::Result<Self> {
        let value = lit.value();
        RULES
            .iter()
            .find(|(name, _)| *name == value)
            .map(|(_, rule)| *rule)
            .ok_or_else(|| {
                let known: Vec<&str> = RULES.iter().map(|(name, _)| *name).collect();
                syn::Error::new_spanned(
                    lit,
                    format!(
                        "unknown rename rule `{value}`; serde accepts {}",
                        known.join(", ")
                    ),
                )
            })
    }

    /// Serde's `RenameRule::apply_to_field`: a field name is snake_case
    /// already, so `lowercase` and `snake_case` leave it alone.
    pub(crate) fn apply(self, field: &str) -> String {
        match self {
            Self::Lower | Self::Snake => field.to_owned(),
            Self::Upper | Self::ScreamingSnake => field.to_ascii_uppercase(),
            Self::Pascal => {
                let mut pascal = String::new();
                let mut capitalize = true;
                for ch in field.chars() {
                    if ch == '_' {
                        capitalize = true;
                    } else if capitalize {
                        pascal.push(ch.to_ascii_uppercase());
                        capitalize = false;
                    } else {
                        pascal.push(ch);
                    }
                }
                pascal
            }
            Self::Camel => {
                let pascal = Self::Pascal.apply(field);
                let mut chars = pascal.chars();
                match chars.next() {
                    Some(first) => first.to_ascii_lowercase().to_string() + chars.as_str(),
                    None => pascal,
                }
            }
            Self::Kebab => field.replace('_', "-"),
            Self::ScreamingKebab => Self::ScreamingSnake.apply(field).replace('_', "-"),
        }
    }
}

/// A value serde's attributes give once or per direction:
/// `rename = "x"` or `rename(serialize = "x", deserialize = "y")`.
#[derive(Clone, Debug)]
struct PerDirection<T> {
    serialize: Option<T>,
    deserialize: Option<T>,
}

impl<T> Default for PerDirection<T> {
    fn default() -> Self {
        Self {
            serialize: None,
            deserialize: None,
        }
    }
}

impl<T: Clone> PerDirection<T> {
    fn both(value: T) -> Self {
        Self {
            serialize: Some(value.clone()),
            deserialize: Some(value),
        }
    }
}

/// Reads `name = "x"` or `name(serialize = "x", deserialize = "y")` into
/// `out`. A half the attribute does not name keeps what an earlier
/// attribute gave it, as serde merges `rename(serialize = ..)` and
/// `rename(deserialize = ..)` written in two attributes.
fn per_direction<T: Clone>(
    meta: &ParseNestedMeta<'_>,
    out: &mut PerDirection<T>,
    parse: impl Fn(&LitStr) -> syn::Result<T>,
) -> syn::Result<()> {
    if meta.input.peek(syn::Token![=]) {
        let lit: LitStr = meta.value()?.parse()?;
        *out = PerDirection::both(parse(&lit)?);
        return Ok(());
    }
    meta.parse_nested_meta(|inner| {
        let lit: LitStr = inner.value()?.parse()?;
        if inner.path.is_ident("serialize") {
            out.serialize = Some(parse(&lit)?);
            Ok(())
        } else if inner.path.is_ident("deserialize") {
            out.deserialize = Some(parse(&lit)?);
            Ok(())
        } else {
            Err(inner.error("expected `serialize` or `deserialize`"))
        }
    })
}

/// What to do with a serde attribute the derive does not read. The
/// derives that write their own serde impls refuse it; the `FormRequest`
/// derives, whose struct serde's own derive handles, ignore it and read only
/// the names.
#[derive(Clone, Copy)]
enum Unknown<'a> {
    Refuse(&'a str),
    Ignore,
}

/// Consumes a serde attribute this module does not read: `name`,
/// `name = value` or `name(...)`.
fn skip_meta(meta: &ParseNestedMeta<'_>) -> syn::Result<()> {
    if meta.input.peek(syn::Token![=]) {
        let _: syn::Expr = meta.value()?.parse()?;
    } else if meta.input.peek(syn::token::Paren) {
        meta.parse_nested_meta(|inner| skip_meta(&inner))?;
    }
    Ok(())
}

/// The container's `#[serde(rename_all = ...)]`, per direction.
#[derive(Clone, Default, Debug)]
pub(crate) struct ContainerSerde {
    rename_all: PerDirection<RenameRule>,
}

/// Reads the struct's `#[serde(...)]` attributes for `derive`. Only
/// `rename_all` is supported.
pub(crate) fn parse_container(attrs: &[Attribute], derive: &str) -> syn::Result<ContainerSerde> {
    container_with(attrs, Unknown::Refuse(derive))
}

/// [`parse_container`] for a struct serde's own derive handles: every
/// attribute but `rename_all` is left to serde.
pub(crate) fn parse_container_lenient(attrs: &[Attribute]) -> syn::Result<ContainerSerde> {
    container_with(attrs, Unknown::Ignore)
}

fn container_with(attrs: &[Attribute], unknown: Unknown<'_>) -> syn::Result<ContainerSerde> {
    let mut out = ContainerSerde::default();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") {
                return per_direction(&meta, &mut out.rename_all, RenameRule::parse);
            }
            match unknown {
                Unknown::Ignore => skip_meta(&meta),
                Unknown::Refuse(derive) => Err(meta.error(format!(
                    "#[derive({derive})] does not support this serde attribute on a struct; it supports `rename_all`"
                ))),
            }
        })?;
    }
    Ok(out)
}

/// A field's `#[serde(...)]` attributes.
#[derive(Clone, Default, Debug)]
struct FieldSerde {
    rename: PerDirection<String>,
    skip_serializing: bool,
    skip_deserializing: bool,
}

fn parse_field_serde(field: &Field, unknown: Unknown<'_>) -> syn::Result<FieldSerde> {
    let mut out = FieldSerde::default();
    for attr in field
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("serde"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                per_direction(&meta, &mut out.rename, |lit| Ok(lit.value()))?;
            } else if meta.path.is_ident("skip") {
                out.skip_serializing = true;
                out.skip_deserializing = true;
            } else if meta.path.is_ident("skip_serializing") {
                out.skip_serializing = true;
            } else if meta.path.is_ident("skip_deserializing") {
                out.skip_deserializing = true;
            } else {
                return match unknown {
                    Unknown::Ignore => skip_meta(&meta),
                    Unknown::Refuse(derive) => Err(meta.error(format!(
                        "#[derive({derive})] does not support this serde attribute on a field; it supports `rename`, `skip`, `skip_serializing` and `skip_deserializing`"
                    ))),
                };
            }
            Ok(())
        })?;
    }
    Ok(out)
}

/// The names a field goes by outside Rust, and whether it goes at all.
#[derive(Clone, Debug, Default)]
pub(crate) struct FieldNames {
    /// The key it is written under: serialized output, Inertia props,
    /// includes, JSON:API members.
    pub(crate) serialize: String,
    /// The key it is read from: request input, and so the key of its
    /// validation errors.
    pub(crate) deserialize: String,
    pub(crate) skip_serializing: bool,
    pub(crate) skip_deserializing: bool,
}

/// The names serde's own derive would give `field`: the field name without
/// a raw identifier's `r#`, then the field's `rename`, else the container's
/// `rename_all`, per direction.
pub(crate) fn field_names(
    field: &Field,
    container: &ContainerSerde,
    derive: &str,
) -> syn::Result<FieldNames> {
    names_with(field, container, Unknown::Refuse(derive))
}

/// [`field_names`] for a struct serde's own derive handles.
pub(crate) fn field_names_lenient(
    field: &Field,
    container: &ContainerSerde,
) -> syn::Result<FieldNames> {
    names_with(field, container, Unknown::Ignore)
}

fn names_with(
    field: &Field,
    container: &ContainerSerde,
    unknown: Unknown<'_>,
) -> syn::Result<FieldNames> {
    let ident = field
        .ident
        .as_ref()
        .ok_or_else(|| syn::Error::new_spanned(field, "expected a named field"))?;
    let raw = ident.to_string();
    let base = raw.strip_prefix("r#").unwrap_or(&raw);
    let serde = parse_field_serde(field, unknown)?;
    let name = |rename: &Option<String>, rule: Option<RenameRule>| match (rename, rule) {
        (Some(rename), _) => rename.clone(),
        (None, Some(rule)) => rule.apply(base),
        (None, None) => base.to_owned(),
    };
    Ok(FieldNames {
        serialize: name(&serde.rename.serialize, container.rename_all.serialize),
        deserialize: name(&serde.rename.deserialize, container.rename_all.deserialize),
        skip_serializing: serde.skip_serializing,
        skip_deserializing: serde.skip_deserializing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(source: &str) -> FieldNames {
        let item: syn::ItemStruct = syn::parse_str(source).expect("a struct");
        let container = parse_container(&item.attrs, "Data").expect("container");
        let field = item.fields.iter().next().expect("a field");
        field_names(field, &container, "Data").expect("names")
    }

    #[test]
    fn every_rule_renames_as_serde_does() {
        for (rule, expected) in [
            ("lowercase", "display_name"),
            ("UPPERCASE", "DISPLAY_NAME"),
            ("PascalCase", "DisplayName"),
            ("camelCase", "displayName"),
            ("snake_case", "display_name"),
            ("SCREAMING_SNAKE_CASE", "DISPLAY_NAME"),
            ("kebab-case", "display-name"),
            ("SCREAMING-KEBAB-CASE", "DISPLAY-NAME"),
        ] {
            let got = names(&format!(
                "#[serde(rename_all = \"{rule}\")] struct S {{ display_name: String }}"
            ));
            assert_eq!(
                (got.serialize.as_str(), got.deserialize.as_str()),
                (expected, expected),
                "{rule}"
            );
        }
    }

    #[test]
    fn a_field_rename_wins_and_directions_split() {
        let got = names(
            "#[serde(rename_all = \"camelCase\")] struct S { #[serde(rename(serialize = \"out\", deserialize = \"in\"))] display_name: String }",
        );
        assert_eq!(
            (got.serialize.as_str(), got.deserialize.as_str()),
            ("out", "in")
        );
        let got = names(
            "#[serde(rename_all(deserialize = \"kebab-case\"))] struct S { display_name: String }",
        );
        assert_eq!(
            (got.serialize.as_str(), got.deserialize.as_str()),
            ("display_name", "display-name")
        );
    }

    #[test]
    fn split_halves_in_two_attributes_merge() {
        let got = names(
            "struct S { #[serde(rename(serialize = \"out\"))] #[serde(rename(deserialize = \"in\"))] a: String }",
        );
        assert_eq!(
            (got.serialize.as_str(), got.deserialize.as_str()),
            ("out", "in")
        );
    }

    #[test]
    fn a_raw_identifier_loses_its_prefix() {
        let got = names("struct S { r#type: String }");
        assert_eq!(got.serialize, "type");
        let got = names("#[serde(rename_all = \"UPPERCASE\")] struct S { r#type: String }");
        assert_eq!(got.serialize, "TYPE");
    }

    #[test]
    fn skips_set_their_directions() {
        let got = names("struct S { #[serde(skip)] a: String }");
        assert!(got.skip_serializing && got.skip_deserializing);
        let got = names("struct S { #[serde(skip_serializing)] a: String }");
        assert!(got.skip_serializing && !got.skip_deserializing);
        let got = names("struct S { #[serde(skip_deserializing)] a: String }");
        assert!(!got.skip_serializing && got.skip_deserializing);
    }

    #[test]
    fn the_lenient_form_reads_the_names_and_leaves_the_rest() {
        let item: syn::ItemStruct = syn::parse_str(
            "#[serde(deny_unknown_fields, rename_all = \"camelCase\", bound(serialize = \"\"))] struct S { #[serde(default, with = \"m\", rename = \"x\")] display_name: String, #[serde(alias = \"b\")] other_name: String }",
        )
        .expect("struct");
        let container = parse_container_lenient(&item.attrs).expect("container");
        let mut fields = item.fields.iter();
        let first = field_names_lenient(fields.next().expect("field"), &container).expect("names");
        assert_eq!(first.deserialize, "x");
        let second = field_names_lenient(fields.next().expect("field"), &container).expect("names");
        assert_eq!(second.deserialize, "otherName");
    }

    #[test]
    fn an_unsupported_attribute_is_refused() {
        let item: syn::ItemStruct =
            syn::parse_str("#[serde(deny_unknown_fields)] struct S { a: String }").expect("struct");
        let err = parse_container(&item.attrs, "Data").expect_err("refused");
        assert!(err.to_string().contains("rename_all"), "{err}");
        let item: syn::ItemStruct =
            syn::parse_str("struct S { #[serde(default)] a: String }").expect("struct");
        let container = parse_container(&item.attrs, "Data").expect("container");
        let err = field_names(
            item.fields.iter().next().expect("field"),
            &container,
            "Data",
        )
        .expect_err("refused");
        assert!(err.to_string().contains("skip_deserializing"), "{err}");
        let item: syn::ItemStruct =
            syn::parse_str("#[serde(rename_all = \"camel\")] struct S { a: String }")
                .expect("struct");
        let err = parse_container(&item.attrs, "Data").expect_err("unknown rule");
        assert!(err.to_string().contains("camelCase"), "{err}");
    }
}
