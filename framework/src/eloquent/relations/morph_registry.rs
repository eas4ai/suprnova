//! Compile-time morph type registry.
//!
//! Every struct annotated `#[suprnova::model(morph_type = "...")]` emits
//! one [`MorphTypeEntry`] via `inventory::submit!`. The registry provides
//! string-to-`TypeId` and `TypeId`-to-string lookups consumed by the
//! per-family morph enum (T6) and the morph m2m loader (T7), and walked
//! by Phase 8 (Admin) to render the polymorphic relation graph.
//!
//! A `MorphTo` relation also carries its own owner list, one
//! [`MorphToTarget`] per declared target, because the registry holds only
//! the models that opt in and holds every family at once.
//!
//! Structurally identical to the [`ModelEntry`](crate::eloquent::ModelEntry)
//! registry - opt-in: only structs that actually declare a
//! `morph_type = "..."` attribute appear here. Plain `#[suprnova::model]`
//! structs without that attribute are deliberately absent (the
//! `morph_type_not_registered_for_non_morph_models` test pins this).

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::OnceLock;

/// One entry per `#[suprnova::model(morph_type = "...")]`-annotated
/// struct, emitted at compile time.
///
/// All non-fn fields are `&'static` so the entry is a const initialiser
/// (a requirement of `inventory::submit!`). The `type_id` field is a
/// `fn() -> TypeId` rather than a stored `TypeId` because `TypeId` is
/// not constructible in a const context on stable Rust - wrapping
/// `TypeId::of::<T>` (itself a `const fn`) keeps the entry `Copy` and
/// the lookup is one indirection.
#[derive(Debug, Clone, Copy)]
pub struct MorphTypeEntry {
    /// String stored in the polymorphic table's `*_type` column (e.g.
    /// `"post"`, `"video"`). Matches the value of `morph_type = "..."`
    /// on the model's `#[suprnova::model]` attribute.
    pub morph_type: &'static str,
    /// Further strings the `*_type` column may hold for this model, from
    /// `morph_aliases = [...]`: every read accepts them, no write stores
    /// them. A Laravel database that adopted `Relation::morphMap` late
    /// holds both `App\Models\Post` and `post` for one model.
    pub aliases: &'static [&'static str],
    /// The Rust type name (`"Post"`).
    pub type_name: &'static str,
    /// The SQL table name (`"posts"`).
    pub table: &'static str,
    /// The model's primary-key column (`"id"`, or what
    /// `#[model(primary_key = "...")]` declares). A `MorphTo` reads the
    /// key of a row's owner from here, since the owner model is chosen
    /// by the type the row names.
    pub primary_key: &'static str,
    /// `TypeId::of::<T>` thunk - wrapped as `fn() -> TypeId` because
    /// `TypeId` itself isn't a stable const, so it can't be stored
    /// directly in an `inventory::submit!` constant.
    pub type_id: fn() -> TypeId,
    /// Build this owner's scoped predicates so morph existence matches its normal reads.
    pub query_constraints: fn() -> crate::eloquent::Builder<()>,
}

inventory::collect!(MorphTypeEntry);

/// One owner type a `MorphTo` relation declares in `targets = [...]`.
///
/// The model macro emits the list into the relation's
/// [`RelationEntry::morph_targets`](super::RelationEntry::morph_targets), in
/// declaration order. The registry cannot stand in for it: it holds every
/// model that declares `morph_type`, from every family, and no model that
/// does not. A `MorphTo` existence query (`has_morph(relation, "*")`, and
/// `with_exists` on a `MorphTo`) reads this list instead, so it reads only
/// the tables of the relation's own owners, whose keys share one type, and
/// finds an owner without `morph_type` by the type string its parent writes.
///
/// All non-fn fields are `&'static` so the list is a constant initialiser
/// inside `inventory::submit!`, as for [`MorphTypeEntry`].
#[derive(Debug, Clone, Copy)]
pub struct MorphToTarget {
    /// The Rust type name (`"Post"`), which `has_morph` also accepts.
    pub type_name: &'static str,
    /// The type string the `*_type` column holds for an owner that
    /// declares no `morph_type`: its snake-cased type name, the default a
    /// parent writes and the fetch helper matches. A registered owner is
    /// matched by its registry entry instead.
    pub fallback_morph_type: &'static str,
    /// The owner's SQL table.
    pub table: &'static str,
    /// The owner's primary-key column, which the row's `*_id` column holds.
    pub primary_key: &'static str,
    /// `TypeId::of::<T>` thunk, to find the owner's registry entry.
    pub type_id: fn() -> TypeId,
    /// Build the owner's scoped predicates so morph existence matches its
    /// normal reads.
    pub query_constraints: fn() -> crate::eloquent::Builder<()>,
}

impl MorphToTarget {
    /// The registered model `entry` describes, as an owner: a type list may
    /// name a registered model the relation does not declare.
    pub(crate) fn from_entry(entry: &MorphTypeEntry) -> Self {
        Self {
            type_name: entry.type_name,
            fallback_morph_type: entry.morph_type,
            table: entry.table,
            primary_key: entry.primary_key,
            type_id: entry.type_id,
            query_constraints: entry.query_constraints,
        }
    }

    /// The type string a parent writes for this owner: its `morph_type`
    /// when the model registers one, else the snake-cased type name.
    pub(crate) fn morph_type(&self) -> &'static str {
        find_morph_type_by_id((self.type_id)())
            .map_or(self.fallback_morph_type, |entry| entry.morph_type)
    }

    /// Every string the `*_type` column may hold for this owner: the
    /// registered `morph_type` and its aliases, or the snake-cased type
    /// name alone when the model registers none.
    pub(crate) fn morph_type_names(&self) -> Vec<String> {
        match find_morph_type_by_id((self.type_id)()) {
            Some(entry) => std::iter::once(entry.morph_type)
                .chain(entry.aliases.iter().copied())
                .map(str::to_owned)
                .collect(),
            None => vec![self.fallback_morph_type.to_owned()],
        }
    }

    /// Whether `name` names this owner: one of its type strings, or its
    /// Rust type name.
    pub(crate) fn is_named(&self, name: &str) -> bool {
        self.type_name == name
            || names_morph_target((self.type_id)(), self.fallback_morph_type, name)
    }
}

/// Iterator over every registered morph type in the binary. Order is
/// link-time; do not depend on it.
pub fn morph_types() -> impl Iterator<Item = &'static MorphTypeEntry> {
    inventory::iter::<MorphTypeEntry>()
}

/// Morph-string index built once on first lookup. Inventory is static
/// over the lifetime of the binary so the index never needs to grow;
/// `OnceLock` gives us a free single-init slot without dragging in
/// `once_cell` for this one site.
fn morph_by_name() -> &'static HashMap<&'static str, &'static MorphTypeEntry> {
    static IDX: OnceLock<HashMap<&'static str, &'static MorphTypeEntry>> = OnceLock::new();
    IDX.get_or_init(|| {
        let mut index = HashMap::new();
        // Aliases first, so a `morph_type` another model also lists as an
        // alias still resolves to the model that writes it.
        for entry in morph_types() {
            for alias in entry.aliases {
                index.insert(*alias, entry);
            }
        }
        for entry in morph_types() {
            index.insert(entry.morph_type, entry);
        }
        index
    })
}

/// Morph-TypeId index (reverse lookup). Same shape as `morph_by_name`;
/// we materialise the `TypeId` values during init so the lookup itself
/// doesn't call the per-entry thunk.
fn morph_by_type_id() -> &'static HashMap<TypeId, &'static MorphTypeEntry> {
    static IDX: OnceLock<HashMap<TypeId, &'static MorphTypeEntry>> = OnceLock::new();
    IDX.get_or_init(|| morph_types().map(|e| ((e.type_id)(), e)).collect())
}

/// Find a morph type by its stored `*_type` string, its `morph_type` or
/// one of its `morph_aliases`. `None` if no model registers that string - distinguishes "registered but not in this
/// MorphTo's target list" from "completely unknown" at runtime. O(1)
/// after the first lookup builds the index; linear scans previously
/// scaled with the number of `#[suprnova::model(morph_type)]` decls,
/// which matters once a polymorphic table fans out across many target
/// types and the loader runs per-row.
pub fn find_morph_type(name: &str) -> Option<&'static MorphTypeEntry> {
    morph_by_name().get(name).copied()
}

/// Reverse lookup: find the registered morph type for a Rust `TypeId`.
/// Useful for debug / admin tooling that wants to render the morph_type
/// string for a known concrete type. O(1) after first init.
pub fn find_morph_type_by_id(id: TypeId) -> Option<&'static MorphTypeEntry> {
    morph_by_type_id().get(&id).copied()
}

/// Every string a `*_type` column may hold for the model `morph_type`
/// names, the `morph_type` first and then its aliases. A string no model
/// registers is returned alone.
pub fn morph_type_names(morph_type: &str) -> Vec<String> {
    match find_morph_type(morph_type) {
        Some(entry) => std::iter::once(entry.morph_type)
            .chain(entry.aliases.iter().copied())
            .map(str::to_owned)
            .collect(),
        None => vec![morph_type.to_owned()],
    }
}

/// Whether the stored `*_type` string `stored` names the model `type_id`:
/// its `morph_type` or one of its aliases when it is registered, otherwise
/// `fallback` (the snake-cased type name, the default a parent writes).
/// The `MorphTo` fetch helper the model macro emits asks this per target.
pub fn names_morph_target(type_id: TypeId, fallback: &str, stored: &str) -> bool {
    match find_morph_type_by_id(type_id) {
        Some(entry) => entry.morph_type == stored || entry.aliases.contains(&stored),
        None => fallback == stored,
    }
}
