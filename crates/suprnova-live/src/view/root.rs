//! One engine-owned browser root shape shared by every island publication path.

use std::fmt::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::Bytes;

use crate::identity::{ComponentName, InstanceId, IslandSlot, Revision};

use super::{IslandRender, ViewError, ViewErrorKind};

const RUNTIME_CONTRACT_V1: u16 = 1;
const MAX_FLAGS: usize = 64;
const MAX_FLAG_NAME_BYTES: usize = 32;
const MAX_FLAG_VALUE_BYTES: usize = 1_024;
const MAX_DOCUMENT_KEY_BYTES: usize = 128;
pub(crate) const MAX_SUCCESSOR_METADATA_BYTES: usize = 1_048_576;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IslandSnapshotForm {
    Seed,
    Instance,
}

impl IslandSnapshotForm {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Seed => "seed",
            Self::Instance => "instance",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IslandRootFlag {
    name: String,
    value: String,
}

impl IslandRootFlag {
    pub(crate) fn from_validated(name: &str, value: &str) -> Self {
        Self {
            name: name.to_owned(),
            value: value.to_owned(),
        }
    }
}

pub(crate) struct IslandRootInput {
    pub(crate) component: ComponentName,
    pub(crate) slot: IslandSlot,
    pub(crate) document_key: String,
    pub(crate) protocol_minimum: u16,
    pub(crate) runtime_contract: u16,
    pub(crate) snapshot: Bytes,
    pub(crate) snapshot_form: IslandSnapshotForm,
    pub(crate) instance_id: Option<InstanceId>,
    pub(crate) revision: Revision,
    pub(crate) lazy_complete: bool,
    pub(crate) flags: Vec<IslandRootFlag>,
    /// The declared stream the island subscribes to, emitted as the
    /// island-owned `live:stream` directive so the browser runtime opens the
    /// asynchronous transport for framework-rendered islands.
    pub(crate) stream: Option<String>,
}

/// The stream the island root subscribes on the component's behalf.
///
/// The root carries one island-owned `live:stream` directive, so only a
/// component that declares exactly one stream gets it. A component with
/// several streams gets none and subscribes each through the runtime's
/// registered calls, rather than having one chosen silently for it.
#[must_use]
pub(crate) fn declared_stream(metadata: &crate::metadata::ComponentMetadata) -> Option<String> {
    match metadata.subscriptions() {
        [only] => Some(only.stream().as_str().to_owned()),
        _ => None,
    }
}

pub(crate) fn assemble_island_root(
    render: IslandRender,
    input: IslandRootInput,
    max_metadata_bytes: usize,
) -> Result<IslandRender, ViewError> {
    validate(&input)?;
    let encoded_snapshot = URL_SAFE_NO_PAD.encode(&input.snapshot);
    let mut attributes = String::new();
    write_attribute(
        &mut attributes,
        "data-suprnova-live-root",
        input.slot.as_str(),
    );
    write_attribute(&mut attributes, "data-suprnova-live-island", "");
    write_attribute(
        &mut attributes,
        "data-suprnova-live-component",
        input.component.as_str(),
    );
    write_attribute(
        &mut attributes,
        "data-suprnova-live-slot",
        input.slot.as_str(),
    );
    write_attribute(
        &mut attributes,
        "data-suprnova-live-document-key",
        &input.document_key,
    );
    write_attribute(
        &mut attributes,
        "data-suprnova-live-protocol-min",
        &input.protocol_minimum.to_string(),
    );
    write_attribute(
        &mut attributes,
        "data-suprnova-live-contract",
        &input.runtime_contract.to_string(),
    );
    write_attribute(
        &mut attributes,
        "data-suprnova-live-snapshot-kind",
        input.snapshot_form.as_str(),
    );
    write_attribute(
        &mut attributes,
        "data-suprnova-live-snapshot",
        &encoded_snapshot,
    );
    write_attribute(
        &mut attributes,
        "data-suprnova-live-revision",
        &input.revision.get().to_string(),
    );
    write_attribute(
        &mut attributes,
        "data-suprnova-live-lazy-complete",
        if input.lazy_complete { "true" } else { "false" },
    );
    if let Some(instance_id) = &input.instance_id {
        write_attribute(
            &mut attributes,
            "data-suprnova-live-instance",
            &instance_id.to_base64url(),
        );
    }
    for flag in &input.flags {
        let _ = write!(attributes, " data-suprnova-live-flag-{}=\"", flag.name);
        escape_attribute(&mut attributes, &flag.value);
        attributes.push('"');
    }
    if let Some(stream) = &input.stream {
        write_attribute(&mut attributes, "live:stream", stream);
    }
    if attributes.len() > max_metadata_bytes {
        return Err(ViewError::new(ViewErrorKind::InvalidMountMetadata));
    }
    let inner = std::str::from_utf8(&render.body)
        .map_err(|_| ViewError::new(ViewErrorKind::TemplateRenderFailed))?;
    let mut body = String::with_capacity(
        attributes
            .len()
            .saturating_add(inner.len())
            .saturating_add(11),
    );
    body.push_str("<div");
    body.push_str(&attributes);
    body.push('>');
    body.push_str(inner);
    body.push_str("</div>");
    Ok(IslandRender {
        body: Bytes::from(body),
        assets: render.assets,
        children: render.children,
    })
}

fn validate(input: &IslandRootInput) -> Result<(), ViewError> {
    let key_valid = !input.document_key.is_empty()
        && input.document_key.len() <= MAX_DOCUMENT_KEY_BYTES
        && input
            .document_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'));
    let form_valid = match input.snapshot_form {
        IslandSnapshotForm::Seed => {
            input.instance_id.is_none() && input.revision == Revision::new(0)
        }
        IslandSnapshotForm::Instance => input.instance_id.is_some(),
    };
    let flags_valid = input.flags.len() <= MAX_FLAGS
        && input.flags.iter().all(|flag| {
            !flag.name.is_empty()
                && flag.name.len() <= MAX_FLAG_NAME_BYTES
                && flag.value.len() <= MAX_FLAG_VALUE_BYTES
                && flag.name.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'-' | b'_')
                })
                && flag.value.chars().all(|character| !character.is_control())
        });
    if !key_valid
        || !form_valid
        || !flags_valid
        || input.snapshot.is_empty()
        || input.runtime_contract != RUNTIME_CONTRACT_V1
        || !matches!(input.protocol_minimum, 1 | 2)
    {
        return Err(ViewError::new(ViewErrorKind::InvalidMountMetadata));
    }
    Ok(())
}

fn write_attribute(output: &mut String, name: &str, value: &str) {
    let _ = write!(output, " {name}=\"");
    escape_attribute(output, value);
    output.push('"');
}

fn escape_attribute(output: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ => output.push(character),
        }
    }
}
