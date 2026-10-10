//! Application-owned identity mapping plans for every supported source.

use std::collections::BTreeMap;

use crate::password::normalize_email;
use crate::{Error, Result};

use super::preflight::SourceUser;

/// A minimal application-owned user view required for migration matching.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppUser {
    /// The application-owned stable i64 primary key.
    pub id: i64,
    /// The application's email address.
    pub email: String,
    /// The existing global-authentication epoch.
    pub auth_epoch: i64,
    /// The existing session-version value.
    pub session_version: i64,
}

/// A planned or completed external identity binding.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExternalIdentity {
    /// The source identity provider.
    pub provider: String,
    /// The provider-owned user identifier.
    pub external_user_id: String,
    /// The application-owned i64 user ID.
    pub app_user_id: i64,
}

/// A passkey envelope imported through the application binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedPasskey {
    /// The application-owned passkey owner.
    pub app_user_id: i64,
    /// The source credential identifier.
    pub credential_id: String,
    /// The byte-preserved source `data_json` envelope.
    pub data_json: String,
}

/// One source-to-application user mapping decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityMapEntry {
    /// The source user matches exactly one existing application user.
    Existing {
        /// Stable source user ID.
        source_user_id: String,
        /// The normalized email used for matching.
        normalized_email: String,
        /// The preserved application-owned i64 ID.
        app_user_id: i64,
    },
    /// The source user has no application match and needs an app-owned row.
    Create {
        /// Stable source user ID.
        source_user_id: String,
        /// The source email supplied to the application binding.
        email: String,
        /// The normalized email used for matching.
        normalized_email: String,
        /// Required application ID for app-owned numeric source schemas.
        app_user_id: Option<i64>,
    },
}

impl IdentityMapEntry {
    /// Returns the source user ID represented by this entry.
    pub fn source_user_id(&self) -> &str {
        match self {
            Self::Existing { source_user_id, .. } | Self::Create { source_user_id, .. } => {
                source_user_id
            }
        }
    }
}

/// Stable source-to-application identity decisions in a dry run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdentityMapPlan {
    /// Entries sorted by source user ID.
    pub entries: Vec<IdentityMapEntry>,
}

impl IdentityMapPlan {
    /// Returns the existing app IDs preserved by this plan, in source-ID order.
    pub fn existing_app_user_ids(&self) -> Vec<i64> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                IdentityMapEntry::Existing { app_user_id, .. } => Some(*app_user_id),
                IdentityMapEntry::Create { .. } => None,
            })
            .collect()
    }

    /// Returns the number of application rows the plan must create.
    pub fn pending_creates(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| matches!(entry, IdentityMapEntry::Create { .. }))
            .count()
    }
}

pub(crate) fn plan_identity_map(
    mut source_users: Vec<SourceUser>,
    app_users: Vec<AppUser>,
) -> Result<IdentityMapPlan> {
    let mut app_users_by_email: BTreeMap<String, Vec<AppUser>> = BTreeMap::new();
    let mut app_users_by_id = BTreeMap::new();
    for app_user in app_users {
        if app_users_by_id
            .insert(app_user.id, app_user.clone())
            .is_some()
        {
            return Err(Error::Conflict {
                resource: "application user identity".to_owned(),
                message: format!("application user id {} appears more than once", app_user.id),
            });
        }
        app_users_by_email
            .entry(normalize_email(&app_user.email))
            .or_default()
            .push(app_user);
    }
    for users in app_users_by_email.values_mut() {
        users.sort_by_key(|user| user.id);
    }
    source_users.sort_by(|left, right| left.id.cmp(&right.id));

    let mut entries = Vec::with_capacity(source_users.len());
    for source_user in source_users {
        let normalized_email = normalize_email(&source_user.email);
        let matches = app_users_by_email
            .get(&normalized_email)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if let Some(required_id) = source_user.preferred_app_user_id {
            match app_users_by_id.get(&required_id) {
                Some(app_user)
                    if normalize_email(&app_user.email) == normalized_email
                        && matches.len() == 1 =>
                {
                    entries.push(IdentityMapEntry::Existing {
                        source_user_id: source_user.id,
                        normalized_email,
                        app_user_id: required_id,
                    });
                }
                Some(_) => {
                    return Err(Error::Conflict {
                        resource: "application user identity".to_owned(),
                        message: format!(
                            "source user {} requires application id {required_id}, which belongs to a different identity",
                            source_user.id
                        ),
                    });
                }
                None if matches.is_empty() => entries.push(IdentityMapEntry::Create {
                    source_user_id: source_user.id,
                    email: source_user.email,
                    normalized_email,
                    app_user_id: Some(required_id),
                }),
                None => {
                    let ids = matches
                        .iter()
                        .map(|user| user.id.to_string())
                        .collect::<Vec<_>>()
                        .join(",");
                    return Err(Error::Conflict {
                        resource: "application user identity".to_owned(),
                        message: format!(
                            "{normalized_email} belongs to app user IDs {ids}, not required id {required_id}"
                        ),
                    });
                }
            }
            continue;
        }

        match matches {
            [] => entries.push(IdentityMapEntry::Create {
                source_user_id: source_user.id,
                email: source_user.email,
                normalized_email,
                app_user_id: None,
            }),
            [app_user] => entries.push(IdentityMapEntry::Existing {
                source_user_id: source_user.id,
                normalized_email,
                app_user_id: app_user.id,
            }),
            _ => {
                let ids = matches
                    .iter()
                    .map(|user| user.id.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                return Err(Error::Conflict {
                    resource: "application normalized email".to_owned(),
                    message: format!("{normalized_email} matches app user IDs {ids}"),
                });
            }
        }
    }
    Ok(IdentityMapPlan { entries })
}
