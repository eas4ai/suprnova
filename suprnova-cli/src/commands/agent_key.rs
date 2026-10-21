//! The agent auth signing key `suprnova new` writes into a new project's
//! `.env`, so the project starts with agent auth on and no operator step.
//!
//! The framework's agent auth signs the identity assertions it gives agents
//! with a P-256 key the application configures, and refuses to start
//! without one. The application's `agents:key` command prints a key as two
//! environment lines; this module mints the same kind of key and writes the
//! same two lines, without linking the framework into the CLI, as
//! `key_generate` does for `APP_KEY`.

use p256::SecretKey;
use p256::elliptic_curve::zeroize::{Zeroize, Zeroizing};
use p256::pkcs8::{EncodePrivateKey, LineEnding};

/// The environment variable that names the key in every assertion.
const KEY_ID_VARIABLE: &str = "AGENT_AUTH_KEY_ID";
/// The environment variable that holds the key, a PKCS#8 PEM on one line.
const PRIVATE_KEY_VARIABLE: &str = "AGENT_AUTH_PRIVATE_KEY";

/// A signing key: its identifier and its P-256 private key in PKCS#8 PEM.
///
/// The key never prints: `Debug` shows the identifier only, and the type
/// has no `Display`. [`env_lines`] is the one way out, into `.env`.
pub struct AgentAuthKey {
    /// `agent-<UTC date YYYYMMDD>-<8 hex>`, which every assertion the key
    /// signs names, so a verifier can tell a rotated key from this one.
    pub kid: String,
    pem: Zeroizing<String>,
}

impl std::fmt::Debug for AgentAuthKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentAuthKey")
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

/// Mint a new key from the operating system's random source.
///
/// # Errors
///
/// When the operating system gives no randomness, or the key cannot be
/// encoded.
pub fn generate() -> Result<AgentAuthKey, String> {
    let secret = loop {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes)
            .map_err(|error| format!("operating system randomness: {error}"))?;
        let candidate = SecretKey::from_slice(&bytes);
        bytes.zeroize();
        // Zero and the values from the group order up are not keys. The
        // chance of drawing one is about 2^-128, so the loop ends.
        if let Ok(secret) = candidate {
            break secret;
        }
    };
    let pem = secret
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|error| format!("encode the agent auth signing key: {error}"))?;
    let mut suffix = [0_u8; 4];
    getrandom::fill(&mut suffix)
        .map_err(|error| format!("operating system randomness: {error}"))?;
    let suffix: String = suffix.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(AgentAuthKey {
        kid: format!("agent-{}-{suffix}", chrono::Utc::now().format("%Y%m%d")),
        pem,
    })
}

/// The two environment lines `agents:key` prints for `key`, the PEM's line
/// breaks written as `\n` so the key fits one line of an environment file.
/// The framework reads a literal `\n` back as a line break.
pub fn env_lines(key: &AgentAuthKey) -> String {
    let one_line = key.pem.trim_end().replace('\n', "\\n");
    format!(
        "{KEY_ID_VARIABLE}={}\n{PRIVATE_KEY_VARIABLE}=\"{one_line}\"\n",
        key.kid
    )
}

#[cfg(test)]
mod tests {
    use p256::pkcs8::DecodePrivateKey;

    use super::*;

    /// The PEM `env_lines` wrote for `key`, its line breaks restored.
    fn written_pem(key: &AgentAuthKey) -> String {
        let lines = env_lines(key);
        let quoted = lines
            .lines()
            .find_map(|line| line.strip_prefix("AGENT_AUTH_PRIVATE_KEY="))
            .expect("the key line");
        quoted.trim_matches('"').replace("\\n", "\n")
    }

    #[test]
    fn agent_007_key_identifier_names_the_day_and_eight_hex_digits() {
        let key = generate().expect("a key");
        let shape = regex::Regex::new(r"^agent-\d{8}-[0-9a-f]{8}$").expect("a pattern");
        assert!(shape.is_match(&key.kid), "{}", key.kid);
        let day = &key.kid["agent-".len().."agent-".len() + 8];
        let today = chrono::Utc::now().date_naive();
        let named = chrono::NaiveDate::parse_from_str(day, "%Y%m%d").expect("a date");
        assert!((today - named).num_days().abs() <= 1, "{day}");
    }

    #[test]
    fn agent_007_env_lines_are_two_lines_with_the_pem_on_one() {
        let key = generate().expect("a key");
        let lines = env_lines(&key);
        let lines: Vec<&str> = lines.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], format!("AGENT_AUTH_KEY_ID={}", key.kid));
        assert!(
            lines[1].starts_with("AGENT_AUTH_PRIVATE_KEY=\"-----BEGIN PRIVATE KEY-----\\n")
                && lines[1].ends_with("\\n-----END PRIVATE KEY-----\""),
            "the key line must be a quoted one-line PKCS#8 PEM"
        );
    }

    #[test]
    fn agent_007_written_key_reads_back_as_the_p256_key_minted() {
        let key = generate().expect("a key");
        let read = SecretKey::from_pkcs8_pem(&written_pem(&key)).expect("a P-256 PKCS#8 key");
        let minted = SecretKey::from_pkcs8_pem(&key.pem).expect("the minted key");
        assert!(
            read == minted,
            "the written key differs from the minted one"
        );
    }

    #[test]
    fn agent_007_every_call_mints_a_new_key() {
        let first = generate().expect("a key");
        let second = generate().expect("a key");
        assert_ne!(first.kid, second.kid);
        assert!(written_pem(&first) != written_pem(&second));
    }

    #[test]
    fn agent_007_debug_shows_the_identifier_and_never_the_key() {
        let key = generate().expect("a key");
        let shown = format!("{key:?}");
        assert!(shown.contains(&key.kid), "{shown}");
        assert!(!shown.contains("PRIVATE KEY"), "{shown}");
        for line in key.pem.lines().filter(|line| !line.starts_with("-----")) {
            assert!(!shown.contains(line), "Debug printed part of the key");
        }
    }
}
