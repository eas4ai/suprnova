//! What agent auth publishes: the `agent_auth` block of the authorization
//! server metadata and the `/auth.md` document, both written from the
//! configuration so neither names an endpoint, identity type or scope the
//! application does not serve.

use std::fmt::Write as _;

use serde_json::{Map, Value, json};

use super::{ASSERTION_REVOKED_EVENT, CLAIM_GRANT, Core, ID_JAG_ASSERTION_TYPE, JWT_BEARER_GRANT};

impl Core {
    /// The identity types that are on, in auth.md's order.
    fn enabled_types(&self) -> Vec<&'static str> {
        let types = self.config.identity_types;
        let mut enabled = Vec::new();
        if types.anonymous {
            enabled.push("anonymous");
        }
        if types.identity_assertion {
            enabled.push("identity_assertion");
        }
        if types.service_auth {
            enabled.push("service_auth");
        }
        enabled
    }

    /// Whether the events endpoint is served: only with ID-JAGs on, since
    /// only a trusted provider sends events.
    pub(crate) fn serves_events(&self) -> bool {
        self.config.identity_types.identity_assertion
    }

    pub(crate) fn metadata_block(&self) -> Value {
        let mut block = Map::new();
        block.insert("skill".to_owned(), json!(self.skill_url()));
        block.insert(
            "identity_endpoint".to_owned(),
            json!(self.endpoint("identity")),
        );
        block.insert(
            "claim_endpoint".to_owned(),
            json!(self.endpoint("identity/claim")),
        );
        if self.serves_events() {
            block.insert(
                "events_endpoint".to_owned(),
                json!(self.endpoint("event/notify")),
            );
        }
        block.insert(
            "identity_types_supported".to_owned(),
            json!(self.enabled_types()),
        );
        if self.config.identity_types.identity_assertion {
            block.insert(
                "identity_assertion".to_owned(),
                json!({"assertion_types_supported": [ID_JAG_ASSERTION_TYPE]}),
            );
            block.insert(
                "events_supported".to_owned(),
                json!([ASSERTION_REVOKED_EVENT]),
            );
        }
        Value::Object(block)
    }

    fn scope_lines(&self, names: &[String]) -> String {
        if names.is_empty() {
            return "none\n".to_owned();
        }
        let mut lines = String::new();
        for scope in self.describe(names) {
            let _ = writeln!(lines, "- `{}`: {}", scope.name, scope.description);
        }
        lines
    }

    /// The `/auth.md` document, in the shape of auth.md v0.6.0's `AUTH.md`.
    pub(crate) fn auth_md(&self) -> String {
        let types = self.config.identity_types;
        let identity = self.endpoint("identity");
        let claim = self.endpoint("identity/claim");
        let token = &self.token_endpoint;
        let revoke = &self.revocation_endpoint;
        let name = &self.config.service_name;
        let mut doc = String::new();
        let _ = writeln!(doc, "# auth.md\n");
        let _ = writeln!(
            doc,
            "You are an agent. {name} supports **agentic registration**: discover, register, \
             claim if needed, exchange for an access_token, call the API, handle revocation. \
             Follow the steps in order; do not skip ahead.\n"
        );
        let _ = writeln!(doc, "## Step 1 - Discover\n");
        let _ = writeln!(
            doc,
            "A 401 from the API carries `WWW-Authenticate: Bearer resource_metadata=\"...\"`. \
             Fetch that protected resource metadata, then the authorization server metadata at \
             `{}/.well-known/oauth-authorization-server`. Its `agent_auth` block lists the \
             endpoints below.\n",
            self.issuer
        );
        let _ = writeln!(doc, "- Issuer: `{}`", self.issuer);
        let _ = writeln!(doc, "- Identity endpoint: `{identity}`");
        let _ = writeln!(doc, "- Claim endpoint: `{claim}`");
        let _ = writeln!(doc, "- Token endpoint: `{token}`");
        let _ = writeln!(doc, "- Revocation endpoint: `{revoke}`");
        if self.serves_events() {
            let _ = writeln!(
                doc,
                "- Events endpoint (providers only): `{}`",
                self.endpoint("event/notify")
            );
        }
        let _ = writeln!(
            doc,
            "- Identity types supported: {}\n",
            self.enabled_types()
                .iter()
                .map(|kind| format!("`{kind}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let _ = writeln!(doc, "Scopes after a user claims or links the agent:\n");
        doc.push_str(&self.scope_lines(&self.config.post_claim_scopes));
        doc.push('\n');
        if types.anonymous {
            let _ = writeln!(doc, "Scopes of an anonymous registration before a claim:\n");
            doc.push_str(&self.scope_lines(&self.config.pre_claim_scopes));
            doc.push('\n');
        }
        let _ = writeln!(doc, "## Step 2 - Pick a method\n");
        let mut step = 1;
        if types.identity_assertion {
            let _ = writeln!(
                doc,
                "{step}. You can get an ID-JAG for the user from a trusted provider: use \
                 `identity_assertion`."
            );
            step += 1;
        }
        if types.service_auth {
            let _ = writeln!(
                doc,
                "{step}. You have only the user's email: use `service_auth`. A claim ceremony \
                 follows."
            );
            step += 1;
        }
        if types.anonymous {
            let _ = writeln!(
                doc,
                "{step}. You have neither: use `anonymous`. A claim is optional and can come \
                 later."
            );
        }
        doc.push('\n');
        let _ = writeln!(doc, "## Step 3 - Register\n");
        let _ = writeln!(doc, "POST JSON to `{identity}`.\n");
        if types.identity_assertion {
            let _ = writeln!(doc, "### identity_assertion\n");
            let _ = writeln!(
                doc,
                "Trusted providers, by the name this service shows users:\n"
            );
            for entry in &self.config.trusted_issuers {
                let _ = writeln!(doc, "- {} (`{}`)", entry.display_name, entry.issuer);
            }
            let _ = writeln!(
                doc,
                "\nMint the ID-JAG with `aud` = this service's issuer or resource, a fresh `jti`, \
                 an `exp` about five minutes out, `email_verified: true` or \
                 `phone_number_verified: true`, and `auth_time` no older than {} seconds.\n",
                self.config.id_jag_max_auth_age.as_secs()
            );
            let _ = writeln!(
                doc,
                "```json\n{{ \"type\": \"identity_assertion\", \"assertion_type\": \"{ID_JAG_ASSERTION_TYPE}\", \"assertion\": \"<ID-JAG>\" }}\n```\n"
            );
            let _ = writeln!(
                doc,
                "A 200 carries `identity_assertion`: go to Step 5. A 401 `interaction_required` \
                 carries a `claim` block: the account exists and the user must confirm the link \
                 (Step 4b). A 401 `login_required` means `auth_time` is missing or too old: \
                 re-authenticate the user at the provider and mint a fresh ID-JAG.\n"
            );
        }
        if types.service_auth {
            let _ = writeln!(doc, "### service_auth\n");
            let _ = writeln!(
                doc,
                "```json\n{{ \"type\": \"service_auth\", \"login_hint\": \"user@example.com\" }}\n```\n"
            );
            let _ = writeln!(
                doc,
                "The 200 carries `claim_token` and a `claim` block (`user_code`, \
                 `verification_uri`, `expires_in`, `interval`) and no `identity_assertion` yet. \
                 Go to Step 4b. `claim_token` is returned exactly once.\n"
            );
        }
        if types.anonymous {
            let _ = writeln!(doc, "### anonymous\n");
            let _ = writeln!(doc, "```json\n{{ \"type\": \"anonymous\" }}\n```\n");
            let _ = writeln!(
                doc,
                "The 200 carries `identity_assertion` (pre-claim scopes) and `claim_token`. The \
                 registration window is {} hours. `claim_token` is returned exactly once.\n",
                self.config.registration_window.as_secs() / 3600
            );
        }
        let _ = writeln!(doc, "## Step 4 - Claim ceremony\n");
        if types.anonymous {
            let _ = writeln!(
                doc,
                "4a. For an anonymous registration, POST `{{ \"claim_token\": \"clm_...\", \
                 \"email\": \"user@example.com\" }}` to `{claim}`. The 200 carries a \
                 `claim_attempt` block. Only a user signed in with that email can complete the \
                 claim.\n"
            );
        }
        let _ = writeln!(
            doc,
            "4b. Give the user `verification_uri` and the six-digit `user_code` in one message. \
             They sign in at {name} and type the code on its page; the code goes to the page, not \
             back to you. The code works for {} seconds.\n",
            self.config.user_code_lifetime.as_secs()
        );
        let _ = writeln!(
            doc,
            "4c. Poll `{token}` (form-encoded) with `grant_type={CLAIM_GRANT}` and \
             `claim_token=<clm_...>`, waiting `interval` seconds ({}) between polls. \
             `authorization_pending` means wait; `slow_down` means poll slower; `expired_token` \
             means call `{claim}` again with the same claim token for a new code, or register \
             again if that answers `claim_expired`. Success is a token response with an \
             `identity_assertion` that carries the user's email; any pre-claim access tokens \
             stop working.\n",
            self.config.poll_interval.as_secs()
        );
        let _ = writeln!(doc, "## Step 5 - Exchange the assertion\n");
        let _ = writeln!(
            doc,
            "POST to `{token}` (form-encoded) with `grant_type={JWT_BEARER_GRANT}`, \
             `assertion=<identity_assertion>` and optionally `resource`. The answer is an \
             access token and never a refresh token: exchange the same assertion again when \
             the access token expires. `invalid_grant` means the assertion expired, was \
             superseded by a claim, or the registration was revoked: register again.\n"
        );
        let _ = writeln!(doc, "## Step 6 - Use the access_token\n");
        let _ = writeln!(
            doc,
            "Send `Authorization: Bearer <access_token>`. On a 401 for a token that worked, try \
             Step 5 once; if that answers `invalid_grant`, restart at Step 1.\n"
        );
        let _ = writeln!(doc, "## Errors\n");
        let _ = writeln!(doc, "| Code | Where | What to do |");
        let _ = writeln!(doc, "| --- | --- | --- |");
        let mut row = |code: &str, place: &str, action: &str| {
            let _ = writeln!(doc, "| `{code}` | {place} | {action} |");
        };
        if !types.anonymous {
            row(
                "anonymous_not_enabled",
                "identity endpoint",
                "Pick another method.",
            );
        }
        if !types.service_auth {
            row(
                "service_auth_not_enabled",
                "identity endpoint",
                "Pick another method.",
            );
        }
        if types.identity_assertion {
            row(
                "invalid_issuer",
                "identity endpoint",
                "Your provider is not on the trust list. Pick another method.",
            );
            for code in [
                "invalid_signature",
                "expired",
                "replay_detected",
                "invalid_audience",
                "invalid_client_id",
                "missing_verified_email",
            ] {
                row(code, "identity endpoint", "Mint a fresh, correct ID-JAG.");
            }
            row(
                "interaction_required (401)",
                "identity endpoint",
                "Hand the `claim` block to the user (Step 4b).",
            );
            row(
                "login_required (401)",
                "identity endpoint",
                "Re-authenticate the user at the provider and mint a fresh ID-JAG.",
            );
        } else {
            row(
                "issuer_not_enabled",
                "identity endpoint",
                "Pick another method.",
            );
        }
        row(
            "invalid_request",
            "identity and claim endpoints",
            "Fix the body; a `login_hint` must be the user's email address.",
        );
        row(
            "invalid_claim_token",
            "claim endpoint",
            "The claim token is wrong or expired. Restart at Step 3.",
        );
        row(
            "claimed_or_in_flight",
            "claim endpoint",
            "Already claimed. Poll the token endpoint.",
        );
        row(
            "claim_expired (410)",
            "claim endpoint",
            "The registration expired. Restart at Step 3.",
        );
        row(
            "invalid_grant",
            "token endpoint",
            "Restart at Step 3 for a fresh assertion.",
        );
        row(
            "unsupported_grant_type",
            "token endpoint",
            "Use one of the two grant types above.",
        );
        row(
            "authorization_pending",
            "token endpoint",
            "Wait `interval` and poll again.",
        );
        row(
            "slow_down",
            "token endpoint",
            "Add at least 5 seconds to your interval.",
        );
        row(
            "expired_token",
            "token endpoint",
            "Call the claim endpoint again for a new code.",
        );
        row(
            "rate_limited (429)",
            "identity, claim and token endpoints",
            "Back off and retry.",
        );
        doc.push('\n');
        let _ = writeln!(doc, "## Revocation\n");
        let _ = writeln!(
            doc,
            "POST `token=<access_token>` to `{revoke}` to revoke one access token; your identity \
             assertion stays valid. {}A user can revoke a registration at {name} at any time; you \
             learn of it when the token endpoint answers `invalid_grant`.",
            if self.serves_events() {
                "A trusted provider can revoke the whole registration with a Security Event \
                 Token. "
            } else {
                ""
            }
        );
        doc
    }
}
