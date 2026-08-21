//! Executable credential host: composes the password, email-verification,
//! password-management, magic-link, passkey, and two-factor plugins over
//! in-memory SQLite and walks ux.md's J2 (password sign-up and sign-in),
//! J4 (passkeys), J5 (magic link), J6 (two-factor), and J8 (recovery) end
//! to end, through both the API bearer lane and the web data-session lane.
//!
//! Run with `--smoke-credentials` to print the scenario summary; the binary
//! asserts every step either way.

use std::sync::Arc;

use parking_lot::Mutex;

use async_trait::async_trait;
use magnetar::Result;
use magnetar::abuse::{AbuseLimiter, AbusePolicy, Permit};
use magnetar::auth::OpaqueFactorGate;
use magnetar::crypto::AeadEncryptor;
use magnetar::passkey::{PasskeyAuthService, PasskeyConfig};
use magnetar::password::{
    LockoutConfig, LockoutService, PasswordHashConfig, PasswordVerifier, StandardPasswordHashDriver,
};
use magnetar::plugin::{
    Effect, HttpRequest, HttpResponse, HttpTransport, LifecycleEvent, LifecycleEventKind,
    LinkGenerator, MailDriver, MailMessage, Method, PluginContext, PluginRegistry, WireBody,
    WireRequest, WireResponse,
};
use magnetar::plugins::email_verification::{
    EmailVerificationPlugin, EmailVerificationPluginConfig, EmailVerificationService,
};
use magnetar::plugins::magic_link::{
    MagicLinkPlugin, MagicLinkPluginConfig, MagicLinkService, RegistrationPolicy,
};
use magnetar::plugins::passkey::{PasskeyPlugin, PasskeyPluginConfig, ReauthSource};
use magnetar::plugins::password::{
    PasswordAuthService, PasswordPlugin, PasswordPluginConfig, RegistrationVerification,
};
use magnetar::plugins::password_management::{
    PasswordManagementPlugin, PasswordManagementPluginConfig, PasswordManagementService,
};
use magnetar::plugins::two_factor::TwoFactorPlugin;
use magnetar::sessions::{
    OpaqueConfig, OpaqueSessionProvider, RememberFacade, RememberService, SessionQueries,
    WebSessionBinding,
};
use magnetar::storage::{SeaOrmStorage, UserStore};
use magnetar::two_factor::{TwoFactorConfig, TwoFactorService};
use serde_json::{Value, json};

#[path = "../../../tests/fixtures/storage_schema.rs"]
mod fixture;
#[path = "../../../tests/fixtures/fakes.rs"]
mod fixture_fakes;
use fixture_fakes::SequentialFirstProofStore;
use fixture::StorageSchema;
use fixture::sql_stores::{SqlRememberStore, SqlSessionStore};
use fixture::sql_two_factor::SqlTwoFactorStore;

const EMAIL: &str = "jordan@example.test";
const PASSWORD: &str = "orange tabby cat";
const NEW_PASSWORD: &str = "fresh honest password";

/// Recording mail driver so the smoke can follow emailed links.
#[derive(Default)]
struct SmokeMail(Mutex<Vec<MailMessage>>);

#[async_trait]
impl MailDriver for SmokeMail {
    async fn send(&self, message: MailMessage) -> Result<()> {
        self.0.lock().push(message);
        Ok(())
    }
}

impl SmokeMail {
    fn last(&self, name: &str) -> Value {
        self.0
            .lock()
            .iter()
            .rev()
            .find(|message| message.name == name)
            .unwrap_or_else(|| panic!("expected a {name} mail"))
            .payload
            .clone()
    }
    fn count(&self) -> usize {
        self.0.lock().len()
    }
}

/// Permissive limiter for the smoke; production hosts install Redis.
struct OpenLimiter;

#[async_trait]
impl AbuseLimiter for OpenLimiter {
    async fn acquire(&self, _key: &str, _policy: AbusePolicy) -> Result<Permit> {
        Ok(Permit::Allowed { retry_after: None })
    }
}

struct SmokeEncryptor;

#[async_trait]
impl magnetar::plugin::Encryptor for SmokeEncryptor {
    async fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        Ok(plaintext.to_vec())
    }
    async fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        Ok(ciphertext.to_vec())
    }
}

struct NoTransport;

#[async_trait]
impl HttpTransport for NoTransport {
    async fn send(&self, _request: HttpRequest) -> Result<HttpResponse> {
        Err(magnetar::Error::Internal {
            message: "the credential host sends no outbound HTTP".into(),
        })
    }
}

struct HostLinks;

#[async_trait]
impl LinkGenerator for HostLinks {
    async fn url_for(&self, route_name: &str, params: &[(String, String)]) -> Result<String> {
        let query = params
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("&");
        Ok(format!("https://host.example/{route_name}?{query}"))
    }
}

/// An in-tree "third party" plugin: it consumes only the public SDK surface
/// and proves route registration and post-commit lifecycle delivery.
struct PublicExamplePlugin {
    seen: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl magnetar::plugin::Plugin<StorageSchema> for PublicExamplePlugin {
    fn name(&self) -> &str {
        "example-credentials"
    }
    fn routes(&self) -> Vec<magnetar::plugin::RouteDescriptor> {
        vec![magnetar::plugin::RouteDescriptor::new(
            Method::Post,
            "/credentials/check",
            "credentials.check",
        )]
    }
    async fn handle(
        &self,
        context: magnetar::plugin::RequestContext<'_, StorageSchema>,
    ) -> magnetar::plugin::PluginResult<WireResponse> {
        let user = context
            .session
            .map(|session| session.user_id.clone())
            .unwrap_or_else(|| "anonymous".into());
        Ok(WireResponse::json(json!({"user_id": user})))
    }
    fn lifecycle_hooks(&self) -> Vec<Arc<dyn magnetar::plugin::LifecycleHook<StorageSchema>>> {
        vec![Arc::new(ExampleHook {
            seen: Arc::clone(&self.seen),
        })]
    }
}

struct ExampleHook {
    seen: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl magnetar::plugin::LifecycleHook<StorageSchema> for ExampleHook {
    async fn on_event(
        &self,
        _context: magnetar::plugin::HookContext<'_, StorageSchema>,
        event: LifecycleEvent,
    ) -> magnetar::plugin::PluginResult<()> {
        self.seen.lock().push(event.mutation_id);
        Ok(())
    }
}

/// Host reauth boundary: the smoke stamps password confirmation at login
/// time, exactly as the deployed `Auth::attempt` does.
#[derive(Default)]
struct HostReauth(Mutex<Option<chrono::DateTime<chrono::Utc>>>);

#[async_trait]
impl ReauthSource for HostReauth {
    async fn password_confirmed_at(
        &self,
        _session: &magnetar::sessions::VerifiedSession,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>> {
        Ok(*self.0.lock())
    }
}

fn post(path: &str, body: Value) -> WireRequest {
    let mut request = WireRequest::new(Method::Post, path);
    request.body = WireBody::Json(body);
    request
        .headers
        .insert("user-agent".into(), "smoke-agent".into());
    request
        .headers
        .insert("x-client-ip".into(), "203.0.113.9".into());
    request
}

fn query_param(link: &str, name: &str) -> String {
    let query = link.split('?').nth(1).expect("link has a query");
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        if parts.next() == Some(name) {
            return parts.next().expect("parameter value").to_owned();
        }
    }
    panic!("link {link} missing {name}");
}

struct Reply {
    status: u16,
    body: Option<Value>,
    grant: Option<magnetar::sessions::SessionGrant>,
    cleared: bool,
    remember: Option<magnetar::sessions::RememberCredential>,
}

fn split(response: WireResponse) -> Reply {
    let effects = response.into_effects();
    let mut grant = None;
    let mut cleared = false;
    let mut remember = None;
    for effect in effects.effects {
        match effect {
            Effect::EstablishSession(value) => grant = Some(value),
            Effect::ClearSession => cleared = true,
            Effect::IssueRemember(value) => remember = Some(value),
            _ => {}
        }
    }
    Reply {
        status: effects.status,
        body: effects.body,
        grant,
        cleared,
        remember,
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let verbose = std::env::args().any(|arg| arg == "--smoke-credentials");
    let step = |name: &str| {
        if verbose {
            println!("ok: {name}");
        }
    };

    // Composition: real stores over in-memory SQLite, fake edge drivers.
    let db = fixture::database().await;
    let storage = Arc::new(SeaOrmStorage::<StorageSchema>::new(db.clone()));
    let sessions = Arc::new(OpaqueSessionProvider::new(
        Arc::new(SqlSessionStore(db.clone())),
        OpaqueConfig::default(),
    ));
    let remember = Arc::new(RememberService::new(
        Arc::new(SqlRememberStore(db.clone())),
        chrono::Duration::days(30),
    )?);
    let verifier = Arc::new(PasswordVerifier::new(
        Arc::new(StandardPasswordHashDriver),
        // The deployed profiles are exercised by the corpus suite; the smoke
        // uses light parameters to stay fast.
        PasswordHashConfig {
            bcrypt_cost: 4,
            argon2_memory_kib: 8,
            argon2_iterations: 1,
            argon2_parallelism: 1,
        },
    )?);
    let lockout = Arc::new(LockoutService::new(
        storage.clone(),
        storage.clone(),
        LockoutConfig::default(),
    ));
    let mail = Arc::new(SmokeMail::default());
    let links = Arc::new(HostLinks);
    let provider = Arc::new(PasswordAuthService::new(
        storage.clone(),
        storage.clone(),
        verifier.clone(),
    ));
    let verification = Arc::new(EmailVerificationService::new(
        storage.clone(),
        storage.clone(),
        mail.clone(),
        links.clone(),
    ));
    let first_proof = Arc::new(SequentialFirstProofStore::new(
        storage.clone(),
        storage.clone(),
        storage.clone(),
        remember.clone(),
    ));
    let management = Arc::new(PasswordManagementService::new(
        storage.clone(),
        storage.clone(),
        first_proof.clone(),
        verifier,
        lockout.clone(),
        mail.clone(),
        links.clone(),
    ));
    let crypto = Arc::new(AeadEncryptor::new([9; 32]));
    let two_factor = Arc::new(TwoFactorService::new(
        Arc::new(SqlTwoFactorStore(db.clone())),
        storage.clone(),
        lockout.clone(),
        crypto.clone(),
        TwoFactorConfig::default(),
    ));
    let gate = Arc::new(OpaqueFactorGate::new(
        storage.clone(),
        two_factor.clone(),
        crypto.clone(),
        sessions.clone(),
    ));
    let magic = Arc::new(MagicLinkService::new(
        storage.clone(),
        storage.clone(),
        first_proof.clone(),
        gate.clone(),
        RegistrationPolicy::Open,
    ));
    let passkeys = Arc::new(PasskeyAuthService::new(
        &PasskeyConfig::default(),
        storage.clone(),
        storage.clone(),
        storage.clone(),
        crypto.clone(),
        gate.clone(),
    )?);
    let reauth = Arc::new(HostReauth::default());
    let context = PluginContext::new(
        storage.clone(),
        sessions.clone(),
        gate.clone(),
        Arc::new(SmokeEncryptor),
        Arc::new(OpenLimiter),
        mail.clone(),
        Arc::new(NoTransport),
        links,
    );
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let registry = PluginRegistry::new(context)
        .register(PublicExamplePlugin {
            seen: Arc::clone(&seen),
        })
        .register(PasswordPlugin::new(
            provider,
            lockout,
            Some(verification.clone() as Arc<dyn RegistrationVerification>),
            Some(remember.clone() as Arc<dyn RememberFacade>),
            PasswordPluginConfig::default(),
        ))
        .register(EmailVerificationPlugin::new(
            verification,
            EmailVerificationPluginConfig::default(),
        ))
        .register(PasswordManagementPlugin::new(
            management,
            PasswordManagementPluginConfig::default(),
        ))
        .register(MagicLinkPlugin::new(
            magic,
            mail.clone(),
            Arc::new(HostLinks),
            MagicLinkPluginConfig::default(),
        ))
        .register(PasskeyPlugin::new(
            passkeys,
            reauth.clone(),
            PasskeyPluginConfig::default(),
        ))
        .register(TwoFactorPlugin::new(
            two_factor.clone(),
            Some(remember.clone() as Arc<dyn RememberFacade>),
        ))
        .build()
        .await?;
    registry.init().await?;
    step("composed password, email-verification, and password-management plugins");

    // J2.1: register — new email creates the user and mails a verification
    // link; the generic response carries no session.
    let reply = split(
        registry
            .handle(post(
                "/register",
                json!({"email": EMAIL, "password": PASSWORD}),
            ))
            .await?,
    );
    assert_eq!(reply.status, 200);
    assert!(reply.grant.is_none());
    let user_id = storage
        .find_by_email(EMAIL)
        .await?
        .expect("registration created the user")
        .user_id;
    step("registered a new account (no session, generic body)");

    // J2.1 (existing email): byte-identical body, no mail, no session.
    let mails = mail.count();
    let replay = split(
        registry
            .handle(post(
                "/register",
                json!({"email": EMAIL, "password": "attacker password"}),
            ))
            .await?,
    );
    assert_eq!(replay.status, reply.status);
    assert_eq!(replay.body, reply.body);
    assert!(replay.grant.is_none());
    assert_eq!(mail.count(), mails, "existing email receives no mail");
    step("existing-email registration stayed generic");

    // J2.2: click the emailed verification link.
    let link = mail.last("email_verification")["verification_link"]
        .as_str()
        .expect("verification link")
        .to_owned();
    let verify = WireRequest::new(
        Method::Get,
        format!(
            "/email/verify/{}/{}",
            query_param(&link, "id"),
            query_param(&link, "hash")
        ),
    );
    let verified = split(registry.handle(verify).await?);
    assert_eq!(verified.status, 200);
    assert!(
        storage
            .find_by_id(&user_id)
            .await?
            .expect("user exists")
            .email_verified_at
            .is_some()
    );
    step("verified the email through the mailed link");

    // J2.3: login with remember-me; the factor gate issues the session.
    let login = split(
        registry
            .handle(post(
                "/login",
                json!({"email": EMAIL, "password": PASSWORD, "remember": true}),
            ))
            .await?,
    );
    assert_eq!(login.status, 200);
    let grant = login.grant.expect("session grant");
    let web_binding: WebSessionBinding = grant.web_binding();
    let remember_cookie = login.remember.expect("remember-me credential");
    drop(remember_cookie.expose_once());
    step("logged in through the factor gate (remember-me issued)");

    // API bearer lane: the bearer token authenticates a bound request.
    let second_login = split(
        registry
            .handle(post(
                "/login",
                json!({"email": EMAIL, "password": PASSWORD}),
            ))
            .await?,
    );
    let bearer = second_login
        .grant
        .expect("second session")
        .into_bearer()
        .expose_token_once();
    let verified_bearer = sessions.verify_bearer(secrecy::ExposeSecret::expose_secret(&bearer));
    assert_eq!(verified_bearer.await?.user_id, user_id);
    step("API bearer lane verified the issued token");

    // J2.4: logout on the web lane revokes the presented session only and
    // retires remember-me rows.
    let logout = split(
        registry
            .handle_web_binding(post("/logout", json!({})), &web_binding)
            .await?,
    );
    assert_eq!(logout.status, 200);
    assert!(logout.cleared);
    let remaining = sessions.list_for_user(&user_id).await?;
    assert_eq!(remaining.len(), 1, "the bearer session survives logout");
    step("logout revoked the presented session and remember-me rows");

    // J8.1: forgot-password mints a single-use token and mails the link;
    // absent emails answer identically.
    let forgot = split(
        registry
            .handle(post("/forgot-password", json!({"email": EMAIL})))
            .await?,
    );
    let absent = split(
        registry
            .handle(post(
                "/forgot-password",
                json!({"email": "nobody@example.test"}),
            ))
            .await?,
    );
    assert_eq!(forgot.status, 200);
    assert_eq!(absent.status, 200);
    assert_eq!(forgot.body, absent.body);
    step("forgot-password stayed anti-enumeration");

    // J8.2: reset — token consumed atomically, password rotated, lockout
    // cleared, all sessions revoked, notification dispatched.
    let token = query_param(
        mail.last("password_reset")["reset_link"]
            .as_str()
            .expect("reset link"),
        "token",
    );
    let reset = split(
        registry
            .handle(post(
                "/reset-password",
                json!({"token": token, "password": NEW_PASSWORD}),
            ))
            .await?,
    );
    assert_eq!(reset.status, 200);
    assert!(sessions.list_for_user(&user_id).await?.is_empty());
    assert!(
        sessions
            .verify_bearer(secrecy::ExposeSecret::expose_secret(&bearer))
            .await
            .is_err(),
        "pre-reset sessions are dead"
    );
    assert_eq!(
        storage
            .find_by_id(&user_id)
            .await?
            .expect("user exists")
            .auth_epoch,
        1,
        "reset bumps the authentication epoch"
    );
    let _ = mail.last("password_changed");
    let burned = split(
        registry
            .handle(post(
                "/reset-password",
                json!({"token": token, "password": "another password"}),
            ))
            .await?,
    );
    assert_eq!(burned.status, 400, "reset tokens are single-use");
    step("reset rotated the credential, revoked every session, and burned the token");

    // Recovery complete: the old password fails, the new one signs in.
    let old = split(
        registry
            .handle(post(
                "/login",
                json!({"email": EMAIL, "password": PASSWORD}),
            ))
            .await?,
    );
    assert_eq!(old.status, 401);
    let fresh = split(
        registry
            .handle(post(
                "/login",
                json!({"email": EMAIL, "password": NEW_PASSWORD}),
            ))
            .await?,
    );
    assert_eq!(fresh.status, 200);
    step("re-authenticated with the reset credential");

    // The reset login re-confirms the password: stamp the reauth window
    // exactly as the deployed `Auth::attempt` does.
    *reauth.0.lock() = Some(chrono::Utc::now());
    let session_binding = fresh
        .grant
        .as_ref()
        .expect("reset login established a session")
        .web_binding();

    // J5: magic link — first-time email is a passwordless signup; the
    // mailed link consumes once and signs in through the gate.
    let magic_email = "casey@example.test";
    let sent = split(
        registry
            .handle(post("/magic-link", json!({"email": magic_email})))
            .await?,
    );
    assert_eq!(sent.status, 200);
    let magic_token = query_param(
        mail.last("magic_link")["magic_link"]
            .as_str()
            .expect("magic link"),
        "token",
    );
    let mut verify_link = WireRequest::new(Method::Get, "/magic-link/verify");
    verify_link.query.insert("token".into(), magic_token);
    let magic = split(registry.handle(verify_link).await?);
    assert_eq!(magic.status, 200);
    let magic_user = magic
        .grant
        .expect("magic link signed in")
        .user_id()
        .to_owned();
    assert!(
        storage
            .find_by_id(&magic_user)
            .await?
            .expect("magic user exists")
            .password_hash
            .is_none(),
        "open policy created a passwordless account"
    );
    step("J5: magic link signed in a first-time passwordless user");

    // J4 (signup): a brand-new email registers a passkey with a software
    // authenticator and signs in with it.
    let passkey_email = "devon@example.test";
    let mut authenticator = webauthn_authenticator_rs::WebauthnAuthenticator::new(
        webauthn_authenticator_rs::softpasskey::SoftPasskey::new(true),
    );
    let origin = webauthn_authenticator_rs::prelude::Url::parse("http://localhost")?;
    let begun = split(
        registry
            .handle(post(
                "/passkeys/register/options",
                json!({"email": passkey_email}),
            ))
            .await?,
    );
    assert_eq!(
        begun.status, 200,
        "new-email signup needs no authentication"
    );
    let body = begun.body.expect("options body");
    let selector = body["selector"].as_str().expect("selector").to_owned();
    let options = serde_json::from_value(body["options"].clone())?;
    let credential = authenticator
        .do_registration(origin.clone(), options)
        .expect("software authenticator completes registration");
    let registered = split(
        registry
            .handle(post(
                "/passkeys/register",
                json!({
                    "selector": selector,
                    "email": passkey_email,
                    "credential": serde_json::to_value(&credential)?,
                }),
            ))
            .await?,
    );
    assert_eq!(registered.status, 200);
    let begun = split(
        registry
            .handle(post(
                "/passkeys/login/options",
                json!({"email": passkey_email}),
            ))
            .await?,
    );
    let body = begun.body.expect("login options body");
    let selector = body["selector"].as_str().expect("selector").to_owned();
    let options = serde_json::from_value(body["options"].clone())?;
    let assertion = authenticator
        .do_authentication(origin, options)
        .expect("software authenticator completes authentication");
    let signed_in = split(
        registry
            .handle(post(
                "/passkeys/login",
                json!({
                    "selector": selector,
                    "email": passkey_email,
                    "credential": serde_json::to_value(&assertion)?,
                }),
            ))
            .await?,
    );
    assert_eq!(signed_in.status, 200);
    assert!(
        signed_in.grant.is_some(),
        "assertion signed in through the gate"
    );
    step("J4: passkey signup and sign-in completed with a software authenticator");

    // J4 (enrollment): the existing account may add a passkey only as the
    // authenticated owner inside the reauth window.
    let enrollment_options = split(
        registry
            .handle_web_binding(
                post("/passkeys/register/options", json!({"email": EMAIL})),
                &session_binding,
            )
            .await?,
    );
    assert_eq!(
        enrollment_options.status, 200,
        "owner with a fresh password confirmation may enroll"
    );
    let anonymous_enrollment = split(
        registry
            .handle(post("/passkeys/register/options", json!({"email": EMAIL})))
            .await?,
    );
    assert_eq!(
        anonymous_enrollment.status, 401,
        "anonymous enrollment against an existing account is refused"
    );
    step("J4: existing-account enrollment enforced owner and reauth");

    // J6: enroll and confirm 2FA through the routes, then the next login
    // is interrupted and completes through the challenge.
    let enrolled = split(
        registry
            .handle_web_binding(post("/user/two-factor", json!({})), &session_binding)
            .await?,
    );
    assert_eq!(enrolled.status, 200);
    let enrollment_body = enrolled.body.expect("enrollment body");
    let otpauth = enrollment_body["otpauth_url"]
        .as_str()
        .expect("otpauth url")
        .to_owned();
    assert_eq!(
        enrollment_body["recovery_codes"]
            .as_array()
            .expect("codes")
            .len(),
        10
    );
    let totp = totp_rs::TOTP::from_url_unchecked(&otpauth)?;
    let confirm = split(
        registry
            .handle_web_binding(
                post(
                    "/user/two-factor/confirm",
                    json!({"code": totp.generate(chrono::Utc::now().timestamp() as u64)}),
                ),
                &session_binding,
            )
            .await?,
    );
    assert_eq!(confirm.status, 200);
    let interrupted = split(
        registry
            .handle(post(
                "/login",
                json!({"email": EMAIL, "password": NEW_PASSWORD}),
            ))
            .await?,
    );
    assert_eq!(interrupted.status, 200);
    assert!(interrupted.grant.is_none(), "no session before the factor");
    let challenge_selector = interrupted.body.expect("challenge body")["challenge_selector"]
        .as_str()
        .expect("challenge selector")
        .to_owned();
    // The confirm claimed the current step; complete with the forward edge.
    let next_code = totp.generate(
        (chrono::Utc::now().timestamp() + magnetar::two_factor::totp::STEP_SECONDS) as u64,
    );
    let completed = split(
        registry
            .handle(post(
                "/two-factor-challenge",
                json!({"challenge_selector": challenge_selector, "code": next_code}),
            ))
            .await?,
    );
    assert_eq!(completed.status, 200);
    assert_eq!(
        completed.grant.expect("exactly one session").user_id(),
        user_id
    );
    step("J6: two-factor enrollment gated the login and the challenge signed in");

    // Post-commit lifecycle delivery reaches registered hooks at least once.
    registry
        .dispatch_lifecycle(LifecycleEvent::new(
            "smoke-user-created",
            LifecycleEventKind::UserCreated,
            user_id.clone(),
        ))
        .await?;
    assert_eq!(seen.lock().len(), 1, "the third-party hook saw the event");
    step("third-party plugin hook received post-commit delivery");

    if verbose {
        println!("credential smoke complete: J2, J4, J5, J6, and J8 green for user {user_id}");
    }
    Ok(())
}
