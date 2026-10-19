//! The application's users as agent auth finds them, and the provisioning
//! policy built from an application's [`AgentUserProvisioner`].

use std::sync::Arc;

use async_trait::async_trait;
use magnetar::agent_auth::{
    AgentUser, AgentUserDirectory, ProvidedIdentity, ProvisioningDecision, ProvisioningPolicy,
};

use super::AgentUserProvisioner;
use crate::auth::Auth;
use crate::error::FrameworkError;
use crate::magnetar_integration::plugin_host::framework_epoch;

/// The default guard's users, read where the session check reads them, as
/// [`HostUserEpochs`](crate::magnetar_integration::plugin_host::HostUserEpochs)
/// reads their epoch:
///
/// - **Magnetar owns the guard's sessions**: the Magnetar engine's user
///   store, whose epoch is the one the user's Magnetar session carries.
/// - **The framework's own session guard**: the guard's user provider, its
///   [`verification_email`](crate::UserProvider::verification_email) and
///   [`is_email_verified`](crate::UserProvider::is_email_verified), and the
///   HRD-007 epoch the session handed to the plugin carries.
///
/// Either way the epoch a claim compares the session against, the one a
/// grant records and the one the authorization server checks it with are
/// the same value. A provider that cannot answer the email methods fails
/// closed: agent auth answers 503, and the error is logged.
pub(crate) struct HostAgentUsers;

#[async_trait]
impl AgentUserDirectory for HostAgentUsers {
    async fn find_by_id(&self, user_id: &str) -> magnetar::Result<Option<AgentUser>> {
        let guard_name = Auth::default_guard_name();
        if crate::auth::epoch::magnetar_fences(&guard_name) {
            return magnetar_users()?.find_by_id(user_id).await;
        }
        let provider = guard_provider(&guard_name)?;
        let Some(user) = provider
            .retrieve_by_id(user_id)
            .await
            .map_err(unavailable)?
        else {
            return Ok(None);
        };
        let email = provider
            .verification_email(user_id)
            .await
            .map_err(unavailable)?
            .unwrap_or_default();
        let email_verified = !email.is_empty()
            && provider
                .is_email_verified(user_id)
                .await
                .map_err(unavailable)?;
        Ok(Some(AgentUser {
            user_id: user.get_auth_identifier(),
            email,
            email_verified,
            auth_epoch: Some(framework_epoch(user.as_ref())),
        }))
    }

    async fn find_by_verified_email(&self, email: &str) -> magnetar::Result<Option<AgentUser>> {
        let guard_name = Auth::default_guard_name();
        if crate::auth::epoch::magnetar_fences(&guard_name) {
            return magnetar_users()?.find_by_verified_email(email).await;
        }
        let provider = guard_provider(&guard_name)?;
        // A provider sends the address in lower case; the stored one keeps
        // whatever case the user typed. Every account that holds it in any
        // case is read: an unverified `alice@corp.test` must not hide
        // Alice's verified `Alice@Corp.test`.
        let candidates = match provider.retrieve_all_by_email_ignoring_case(email).await {
            Ok(candidates) => candidates,
            Err(error) if error.status_code() == 409 => return Err(several_accounts()),
            Err(error) => return Err(unavailable(error)),
        };
        // Only an account that holds the address as its verified one
        // counts: binding a provider identity to an account that never
        // proved the mailbox would hand the agent to whoever typed the
        // address first.
        let mut verified = Vec::new();
        for candidate in candidates {
            if let Some(user) = self.find_by_id(&candidate.id).await?
                && user.email_verified
                && same_email(&user.email, email)
            {
                verified.push(user);
            }
        }
        match verified.len() {
            0 => Ok(None),
            1 => Ok(verified.pop()),
            // Picking one would choose an account for the user, so agent
            // auth refuses the ID-JAG instead, binding and creating nothing.
            _ => Err(several_accounts()),
        }
    }

    async fn find_by_verified_phone(&self, _phone: &str) -> magnetar::Result<Option<AgentUser>> {
        // The framework's user providers know no phone numbers. An
        // application that matches by phone supplies its own directory.
        Ok(None)
    }
}

/// The installed Magnetar engine's users.
fn magnetar_users() -> magnetar::Result<Arc<dyn AgentUserDirectory>> {
    crate::magnetar_integration::host_sign_in_authority()
        .and_then(|authority| authority.agent_users())
        .ok_or_else(|| magnetar::Error::DependencyUnavailable {
            dependency: "Magnetar user store".to_owned(),
            message: "the installed Magnetar engine reads no users for agent auth".to_owned(),
        })
}

/// The answer that makes agent auth refuse an address verified on more
/// than one account.
fn several_accounts() -> magnetar::Error {
    magnetar::Error::Conflict {
        resource: "user".to_owned(),
        message: "the verified address matches more than one account".to_owned(),
    }
}

fn guard_provider(guard_name: &str) -> magnetar::Result<Arc<dyn crate::UserProvider>> {
    crate::auth::epoch::guard_provider(guard_name).ok_or_else(|| {
        magnetar::Error::DependencyUnavailable {
            dependency: "user provider".to_owned(),
            message: format!("the default guard '{guard_name}' has no user provider"),
        }
    })
}

fn unavailable(error: FrameworkError) -> magnetar::Error {
    tracing::error!(%error, "agent auth could not read a user from the user provider");
    magnetar::Error::DependencyUnavailable {
        dependency: "user provider".to_owned(),
        message: error.to_string(),
    }
}

/// Whether two addresses are the same once case and surrounding space are
/// ignored.
fn same_email(left: &str, right: &str) -> bool {
    left.trim().to_lowercase() == right.trim().to_lowercase()
}

/// The provisioning policy an application's [`AgentUserProvisioner`]
/// gives: the user it creates is bound only when the directory finds it
/// with the provider's verified email, verified.
pub(crate) struct ProvisionWith {
    provisioner: Arc<dyn AgentUserProvisioner>,
    users: Arc<dyn AgentUserDirectory>,
}

impl ProvisionWith {
    pub(crate) fn new(
        provisioner: Arc<dyn AgentUserProvisioner>,
        users: Arc<dyn AgentUserDirectory>,
    ) -> Self {
        Self { provisioner, users }
    }
}

#[async_trait]
impl ProvisioningPolicy for ProvisionWith {
    async fn provision(
        &self,
        identity: &ProvidedIdentity,
    ) -> magnetar::Result<ProvisioningDecision> {
        let user_id = match self.provisioner.provision(identity).await {
            Ok(Some(user_id)) => user_id,
            Ok(None) => return Ok(ProvisioningDecision::Refused),
            Err(error) => {
                tracing::error!(%error, "the application's agent user provisioner failed");
                return Err(magnetar::Error::DependencyUnavailable {
                    dependency: "agent user provisioner".to_owned(),
                    message: error.to_string(),
                });
            }
        };
        let Some(user) = self.users.find_by_id(&user_id).await? else {
            tracing::error!(
                user_id,
                "the agent user provisioner named a user the directory cannot find"
            );
            return Err(magnetar::Error::DependencyUnavailable {
                dependency: "agent user provisioner".to_owned(),
                message: "the provisioned user cannot be found".to_owned(),
            });
        };
        // The identity assertion signed for a bound user says its email is
        // verified, and the identity is the provider's for that address.
        let holds_the_address = identity
            .verified_email
            .as_deref()
            .is_none_or(|email| same_email(email, &user.email));
        if !user.email_verified || !holds_the_address {
            tracing::warn!(
                user_id,
                "the agent user provisioner's user lacks the provider's verified email; the \
                 ID-JAG is refused"
            );
            return Ok(ProvisioningDecision::Refused);
        }
        Ok(ProvisioningDecision::Provisioned(user))
    }
}

#[cfg(test)]
mod tests {
    use super::same_email;

    #[test]
    fn agent_008_emails_compare_without_case() {
        assert!(same_email("Alice@Corp.test", "alice@corp.test"));
        assert!(same_email(" alice@corp.test ", "ALICE@CORP.TEST"));
        assert!(!same_email("alice@corp.test", "alicia@corp.test"));
    }
}
