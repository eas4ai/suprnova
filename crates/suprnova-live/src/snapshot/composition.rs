//! Typed snapshot-schema-v1 extension for independent island composition lineage.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::canonical::{CanonicalValue, to_canonical_bytes};
use crate::component::composition::ChildKey;
use crate::identity::{ContentDigest, InstanceId, Revision};
use crate::limits::InputLimits;

use super::{SnapshotError, SnapshotErrorKind, SnapshotLimits};

#[cfg(test)]
thread_local! {
    static CANONICALIZATION_PATHS: std::cell::RefCell<Vec<&'static str>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

#[cfg(test)]
pub(crate) fn record_composition_canonicalization(path: &'static str) {
    CANONICALIZATION_PATHS.with(|paths| paths.borrow_mut().push(path));
}

#[cfg(test)]
pub(crate) fn take_composition_canonicalization_paths() -> Vec<&'static str> {
    CANONICALIZATION_PATHS.with(|paths| std::mem::take(&mut *paths.borrow_mut()))
}

/// Snapshot-schema-v1 extension name for independently owned island lineage.
pub const COMPOSITION_LINEAGE_EXTENSION_V1: &str = "x_suprnova_live_composition_v1";
/// Maximum immediate children recorded by one instanced snapshot.
pub const MAX_COMPOSITION_LINEAGE_CHILDREN_V1: usize = 256;
/// Maximum independently owned island nesting depth recorded by lineage v1.
pub const MAX_COMPOSITION_LINEAGE_DEPTH_V1: u16 = 64;
/// Maximum canonical bytes occupied by the composition extension alone.
pub const MAX_COMPOSITION_LINEAGE_BYTES_V1: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompositionBindingV1 {
    parent_instance: InstanceId,
    parent_revision: Revision,
    child_key: ChildKey,
    child_contract: ContentDigest,
    child_instance: InstanceId,
    depth: u16,
}

impl CompositionBindingV1 {
    fn new(
        parent_instance: InstanceId,
        parent_revision: Revision,
        child_key: ChildKey,
        child_contract: ContentDigest,
        child_instance: InstanceId,
        depth: u16,
    ) -> Result<Self, SnapshotError> {
        if parent_instance == child_instance
            || depth == 0
            || depth > MAX_COMPOSITION_LINEAGE_DEPTH_V1
        {
            return Err(SnapshotError::new(SnapshotErrorKind::InvalidExtension));
        }
        Ok(Self {
            parent_instance,
            parent_revision,
            child_key,
            child_contract,
            child_instance,
            depth,
        })
    }

    fn to_canonical(&self) -> CanonicalValue {
        CanonicalValue::Object(BTreeMap::from([
            (
                "child_component_contract".to_owned(),
                CanonicalValue::String(self.child_contract.to_base64url()),
            ),
            (
                "child_instance".to_owned(),
                CanonicalValue::String(self.child_instance.to_base64url()),
            ),
            (
                "child_key".to_owned(),
                CanonicalValue::String(self.child_key.as_str().to_owned()),
            ),
            (
                "depth".to_owned(),
                CanonicalValue::String(self.depth.to_string()),
            ),
            (
                "parent_instance".to_owned(),
                CanonicalValue::String(self.parent_instance.to_base64url()),
            ),
            (
                "parent_revision".to_owned(),
                CanonicalValue::String(self.parent_revision.get().to_string()),
            ),
        ]))
    }

    fn from_canonical(value: &CanonicalValue) -> Result<Self, SnapshotError> {
        const FIELDS: [&str; 6] = [
            "child_component_contract",
            "child_instance",
            "child_key",
            "depth",
            "parent_instance",
            "parent_revision",
        ];
        let CanonicalValue::Object(fields) = value else {
            return Err(SnapshotError::new(SnapshotErrorKind::InvalidExtension));
        };
        if fields.len() != FIELDS.len() || FIELDS.iter().any(|field| !fields.contains_key(*field)) {
            return Err(SnapshotError::new(SnapshotErrorKind::InvalidExtension));
        }
        let depth_text = string_field(fields, "depth")?;
        let depth = depth_text
            .parse::<u16>()
            .ok()
            .filter(|depth| depth.to_string() == depth_text)
            .ok_or_else(|| SnapshotError::new(SnapshotErrorKind::InvalidExtension))?;
        Self::new(
            InstanceId::parse(string_field(fields, "parent_instance")?)
                .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidExtension))?,
            Revision::parse(string_field(fields, "parent_revision")?)
                .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidExtension))?,
            ChildKey::parse(string_field(fields, "child_key")?)
                .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidExtension))?,
            ContentDigest::parse(string_field(fields, "child_component_contract")?)
                .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidExtension))?,
            InstanceId::parse(string_field(fields, "child_instance")?)
                .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidExtension))?,
            depth,
        )
    }
}

fn string_field<'value>(
    fields: &'value BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Result<&'value str, SnapshotError> {
    let Some(CanonicalValue::String(value)) = fields.get(name) else {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidExtension));
    };
    Ok(value)
}

/// Immediate parent ownership binding carried by an independently owned child.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionOwnerLineageV1(CompositionBindingV1);

impl CompositionOwnerLineageV1 {
    /// Creates one exact parent-to-child ownership binding.
    pub fn new(
        parent_instance: InstanceId,
        parent_revision: Revision,
        child_key: ChildKey,
        child_contract: ContentDigest,
        child_instance: InstanceId,
        depth: u16,
    ) -> Result<Self, SnapshotError> {
        CompositionBindingV1::new(
            parent_instance,
            parent_revision,
            child_key,
            child_contract,
            child_instance,
            depth,
        )
        .map(Self)
    }

    /// Returns the exact issuing parent instance.
    #[must_use]
    pub const fn parent_instance(&self) -> &InstanceId {
        &self.0.parent_instance
    }

    /// Returns the accepted parent revision that established ownership.
    #[must_use]
    pub const fn parent_revision(&self) -> Revision {
        self.0.parent_revision
    }

    /// Returns the stable key within the parent ownership scope.
    #[must_use]
    pub const fn child_key(&self) -> &ChildKey {
        &self.0.child_key
    }

    /// Returns the exact child component contract.
    #[must_use]
    pub const fn child_contract(&self) -> &ContentDigest {
        &self.0.child_contract
    }

    /// Returns the exact independently owned child instance.
    #[must_use]
    pub const fn child_instance(&self) -> &InstanceId {
        &self.0.child_instance
    }

    /// Returns the child's bounded composition depth.
    #[must_use]
    pub const fn depth(&self) -> u16 {
        self.0.depth
    }
}

/// One exact independently owned child binding carried by its parent snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionChildLineageV1(CompositionBindingV1);

impl CompositionChildLineageV1 {
    /// Creates one exact parent-to-child binding.
    pub fn new(
        parent_instance: InstanceId,
        parent_revision: Revision,
        child_key: ChildKey,
        child_contract: ContentDigest,
        child_instance: InstanceId,
        depth: u16,
    ) -> Result<Self, SnapshotError> {
        CompositionBindingV1::new(
            parent_instance,
            parent_revision,
            child_key,
            child_contract,
            child_instance,
            depth,
        )
        .map(Self)
    }

    /// Returns the exact parent instance.
    #[must_use]
    pub const fn parent_instance(&self) -> &InstanceId {
        &self.0.parent_instance
    }

    /// Returns the accepted parent revision that owns this entry.
    #[must_use]
    pub const fn parent_revision(&self) -> Revision {
        self.0.parent_revision
    }

    /// Returns the stable key within the parent ownership scope.
    #[must_use]
    pub const fn child_key(&self) -> &ChildKey {
        &self.0.child_key
    }

    /// Returns the exact child component contract.
    #[must_use]
    pub const fn child_contract(&self) -> &ContentDigest {
        &self.0.child_contract
    }

    /// Returns the exact independently owned child instance.
    #[must_use]
    pub const fn child_instance(&self) -> &InstanceId {
        &self.0.child_instance
    }

    /// Returns the child's bounded composition depth.
    #[must_use]
    pub const fn depth(&self) -> u16 {
        self.0.depth
    }
}

/// Validated owner and immediate-child lineage stored in one signed extension.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionLineageV1 {
    owner: Option<CompositionOwnerLineageV1>,
    children: Vec<CompositionChildLineageV1>,
}

impl CompositionLineageV1 {
    /// Creates a nonempty, bounded, unambiguous immediate-lineage record.
    pub fn new(
        owner: Option<CompositionOwnerLineageV1>,
        children: Vec<CompositionChildLineageV1>,
    ) -> Result<Self, SnapshotError> {
        if (owner.is_none() && children.is_empty())
            || children.len() > MAX_COMPOSITION_LINEAGE_CHILDREN_V1
        {
            return Err(SnapshotError::new(SnapshotErrorKind::InvalidExtension));
        }
        let expected_child_depth = if children.is_empty() {
            None
        } else {
            Some(
                owner
                    .as_ref()
                    .map_or(Some(1), |owner| owner.depth().checked_add(1))
                    .filter(|depth| *depth <= MAX_COMPOSITION_LINEAGE_DEPTH_V1)
                    .ok_or_else(|| SnapshotError::new(SnapshotErrorKind::InvalidExtension))?,
            )
        };
        let mut keys = BTreeSet::new();
        let mut instances = HashSet::new();
        let mut parent = None;
        for child in &children {
            let binding = &child.0;
            let current_parent = (&binding.parent_instance, binding.parent_revision);
            if Some(binding.depth) != expected_child_depth
                || !keys.insert(binding.child_key.clone())
                || !instances.insert(binding.child_instance.clone())
                || parent.is_some_and(|parent| parent != current_parent)
            {
                return Err(SnapshotError::new(SnapshotErrorKind::InvalidExtension));
            }
            parent = Some(current_parent);
        }
        Ok(Self { owner, children })
    }

    /// Returns the optional parent ownership of this snapshot's island.
    #[must_use]
    pub const fn owner(&self) -> Option<&CompositionOwnerLineageV1> {
        self.owner.as_ref()
    }

    /// Returns the stable-key-unique immediate child entries.
    #[must_use]
    pub fn children(&self) -> &[CompositionChildLineageV1] {
        &self.children
    }

    pub(crate) fn to_canonical(&self) -> CanonicalValue {
        CanonicalValue::Object(BTreeMap::from([
            (
                "children".to_owned(),
                CanonicalValue::Array(
                    self.children
                        .iter()
                        .map(|child| child.0.to_canonical())
                        .collect(),
                ),
            ),
            (
                "owner".to_owned(),
                self.owner
                    .as_ref()
                    .map_or(CanonicalValue::Null, |owner| owner.0.to_canonical()),
            ),
        ]))
    }

    fn from_canonical(value: &CanonicalValue) -> Result<Self, SnapshotError> {
        let CanonicalValue::Object(fields) = value else {
            return Err(SnapshotError::new(SnapshotErrorKind::InvalidExtension));
        };
        if fields.len() != 2 || !fields.contains_key("owner") || !fields.contains_key("children") {
            return Err(SnapshotError::new(SnapshotErrorKind::InvalidExtension));
        }
        let owner = match fields.get("owner") {
            Some(CanonicalValue::Null) => None,
            Some(value) => Some(CompositionOwnerLineageV1(
                CompositionBindingV1::from_canonical(value)?,
            )),
            None => return Err(SnapshotError::new(SnapshotErrorKind::InvalidExtension)),
        };
        let Some(CanonicalValue::Array(children)) = fields.get("children") else {
            return Err(SnapshotError::new(SnapshotErrorKind::InvalidExtension));
        };
        if children.len() > MAX_COMPOSITION_LINEAGE_CHILDREN_V1 {
            return Err(SnapshotError::new(SnapshotErrorKind::InvalidExtension));
        }
        let children = children
            .iter()
            .map(CompositionBindingV1::from_canonical)
            .map(|binding| binding.map(CompositionChildLineageV1))
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(owner, children)
    }
}

pub(crate) fn composition_lineage_from_extensions(
    extensions: &BTreeMap<String, CanonicalValue>,
    limits: &SnapshotLimits,
    instance_id: &InstanceId,
    revision: Revision,
    component_contract: &ContentDigest,
) -> Result<Option<CompositionLineageV1>, SnapshotError> {
    let Some(value) = extensions.get(COMPOSITION_LINEAGE_EXTENSION_V1) else {
        return Ok(None);
    };
    canonicalize_composition_lineage_extension(value, limits)?;
    let lineage = CompositionLineageV1::from_canonical(value)?;
    if lineage.owner().is_some_and(|owner| {
        owner.child_instance() != instance_id || owner.child_contract() != component_contract
    }) || lineage
        .children()
        .iter()
        .any(|child| child.parent_instance() != instance_id || child.parent_revision() != revision)
    {
        return Err(SnapshotError::new(SnapshotErrorKind::BindingMismatch));
    }
    Ok(Some(lineage))
}

fn canonicalize_composition_lineage_extension(
    value: &CanonicalValue,
    limits: &SnapshotLimits,
) -> Result<Vec<u8>, SnapshotError> {
    let input = *limits.input();
    let composition_input = InputLimits::new(
        input.max_bytes().min(MAX_COMPOSITION_LINEAGE_BYTES_V1),
        input.max_depth(),
        input.max_entries(),
        input.max_string_bytes(),
    )
    .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidExtension))?;
    #[cfg(test)]
    record_composition_canonicalization("recognized");
    to_canonical_bytes(value, &composition_input)
        .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidExtension))
}
