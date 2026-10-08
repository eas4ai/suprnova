//! What an entry must not keep: Laravel's `RedactsSensitiveData`.
//!
//! An entry stores headers, bodies and prop values as the application saw
//! them, on disk, readable by anyone the endpoints admit. Before it is
//! stored, the value of every object key named by the redaction keys is
//! replaced by `[REDACTED]`, at any depth, as are the query parameters of
//! the same names in the entry's URLs and the values of the redaction
//! headers. Names are compared without case, so `Password` is caught by
//! `password`.

use serde_json::Value;

/// What a redacted value is replaced by.
pub(crate) const REDACTED: &str = "[REDACTED]";

/// What a value that cannot be written as JSON is replaced by: a header
/// value that is not text, for one.
pub(crate) const UNSERIALIZABLE: &str = "[UNSERIALIZABLE]";

/// The redaction lists of one configuration, lower-cased once.
#[derive(Debug, Clone, Default)]
pub(crate) struct Redactor {
    keys: Vec<String>,
    headers: Vec<String>,
}

impl Redactor {
    /// The redactor for `keys` and `headers`. Empty names are dropped.
    pub(crate) fn new(keys: &[String], headers: &[String]) -> Self {
        Self {
            keys: normalize(keys),
            headers: normalize(headers),
        }
    }

    /// Whether `name` is one of the redaction keys.
    fn is_sensitive_key(&self, name: &str) -> bool {
        is_listed(&self.keys, name)
    }

    /// Whether `name` is one of the redaction headers.
    fn is_sensitive_header(&self, name: &str) -> bool {
        is_listed(&self.headers, name)
    }

    /// The storage pass over a whole entry, Laravel's
    /// `redactSensitiveStoragePayload`: the keys at any depth, then the
    /// query parameters of every `url` and `redirectLocation`, then the
    /// values of the redaction headers in both header bags.
    pub(crate) fn redact_entry(&self, entry: &mut Value) {
        self.redact_keys(entry);
        self.redact_urls(entry);
        self.redact_header_bags(entry);
    }

    /// Replace the value of every object key named by the redaction keys,
    /// at any depth, by `[REDACTED]`.
    pub(crate) fn redact_keys(&self, value: &mut Value) {
        if self.keys.is_empty() {
            return;
        }
        match value {
            Value::Object(map) => {
                for (key, member) in map.iter_mut() {
                    if self.is_sensitive_key(key) {
                        *member = Value::String(REDACTED.to_string());
                    } else {
                        self.redact_keys(member);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| self.redact_keys(item)),
            _ => {}
        }
    }

    /// Redact the query of every string under a `url` or
    /// `redirectLocation` key, at any depth.
    fn redact_urls(&self, value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (key, member) in map.iter_mut() {
                    match member {
                        Value::String(url)
                            if key.eq_ignore_ascii_case("url")
                                || key.eq_ignore_ascii_case("redirectLocation") =>
                        {
                            *url = self.redact_url(url);
                        }
                        other => self.redact_urls(other),
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| self.redact_urls(item)),
            _ => {}
        }
    }

    /// `url` with the value of every query parameter named by the
    /// redaction keys replaced by `[REDACTED]`, percent-encoded as a query
    /// writes it. A bracketed name (`filter[secret]`) is caught by any of
    /// its parts, as Laravel matches nested keys at their own depth. The
    /// rest of the URL, the other parameters and the fragment are left as
    /// they were sent.
    pub(crate) fn redact_url(&self, url: &str) -> String {
        if self.keys.is_empty() {
            return url.to_string();
        }
        let Some((base, rest)) = url.split_once('?') else {
            return url.to_string();
        };
        let (query, fragment) = match rest.split_once('#') {
            Some((query, fragment)) => (query, Some(fragment)),
            None => (rest, None),
        };
        let mut changed = false;
        let pairs: Vec<String> = query
            .split('&')
            .map(|pair| {
                let name = pair.split_once('=').map_or(pair, |(name, _)| name);
                if self.query_name_is_sensitive(name) {
                    changed = true;
                    format!("{name}=%5BREDACTED%5D")
                } else {
                    pair.to_string()
                }
            })
            .collect();
        if !changed {
            return url.to_string();
        }
        let mut redacted = format!("{base}?{}", pairs.join("&"));
        if let Some(fragment) = fragment {
            redacted.push('#');
            redacted.push_str(fragment);
        }
        redacted
    }

    /// Whether the raw query name `name` (`token`, `filter%5Bsecret%5D`)
    /// names a redaction key in any of its bracketed parts.
    fn query_name_is_sensitive(&self, name: &str) -> bool {
        let decoded: String = url::form_urlencoded::parse(name.as_bytes())
            .next()
            .map(|(decoded, _)| decoded.into_owned())
            .unwrap_or_default();
        decoded
            .split(['[', ']'])
            .filter(|part| !part.is_empty())
            .any(|part| self.is_sensitive_key(part))
    }

    /// Replace the value of every redaction header in the `requestHeaders`
    /// and `responseHeaders` objects, at any depth, by `[REDACTED]`.
    fn redact_header_bags(&self, value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (key, member) in map.iter_mut() {
                    if key.eq_ignore_ascii_case("requestHeaders")
                        || key.eq_ignore_ascii_case("responseHeaders")
                    {
                        if let Value::Object(headers) = member {
                            for (name, header) in headers.iter_mut() {
                                if self.is_sensitive_header(name) {
                                    *header = Value::String(REDACTED.to_string());
                                }
                            }
                        }
                    } else {
                        self.redact_header_bags(member);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| self.redact_header_bags(item)),
            _ => {}
        }
    }
}

/// The names of `list`, lower-cased, the empty ones dropped.
fn normalize(list: &[String]) -> Vec<String> {
    let mut names: Vec<String> = list
        .iter()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_lowercase())
        .collect();
    names.dedup();
    names
}

/// Whether `name`, compared without case, is in the lower-cased `list`.
fn is_listed(list: &[String], name: &str) -> bool {
    let lowered = name.to_lowercase();
    list.iter().any(|listed| *listed == lowered)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn redactor() -> Redactor {
        let config = crate::inertia::devtools::DevToolsConfig::new();
        Redactor::new(&config.redact_keys, &config.redact_headers)
    }

    #[test]
    fn indt_a_key_is_redacted_at_any_depth_and_in_any_case() {
        let mut value = json!({
            "password": "hunter2",
            "user": {"Password": "x", "name": "Ada", "tokens": [{"api_key": "k"}]},
            "PASSWORD_CONFIRMATION": "y",
        });
        redactor().redact_keys(&mut value);
        assert_eq!(
            value,
            json!({
                "password": REDACTED,
                "user": {"Password": REDACTED, "name": "Ada", "tokens": [{"api_key": REDACTED}]},
                "PASSWORD_CONFIRMATION": REDACTED,
            })
        );
    }

    #[test]
    fn indt_a_url_loses_the_values_of_sensitive_query_parameters_only() {
        let redactor = redactor();
        assert_eq!(
            redactor.redact_url("http://app.test/reset?token=abc&page=2#top"),
            "http://app.test/reset?token=%5BREDACTED%5D&page=2#top"
        );
        assert_eq!(
            redactor.redact_url("/search?filter%5Bsecret%5D=s&filter%5Bname%5D=n&Token=t"),
            "/search?filter%5Bsecret%5D=%5BREDACTED%5D&filter%5Bname%5D=n&Token=%5BREDACTED%5D"
        );
        assert_eq!(redactor.redact_url("/plain?page=2"), "/plain?page=2");
        assert_eq!(redactor.redact_url("/plain"), "/plain");
    }

    #[test]
    fn indt_the_storage_pass_redacts_keys_urls_and_header_bags() {
        let mut entry = json!({
            "__meta": {"url": "http://app.test/x?access_token=a", "redirectLocation": "/y?secret=s"},
            "http": {
                "requestHeaders": {"cookie": "laravel_session=1", "accept": "text/html"},
                "responseHeaders": {"set-cookie": "a=b", "Authorization": "Bearer t"},
                "requestBody": {"status": "present", "value": {"password": "p", "email": "e"}},
            },
            "propValues": {"auth": {"token": "t", "name": "n"}},
        });
        redactor().redact_entry(&mut entry);
        assert_eq!(entry["__meta"]["url"], "http://app.test/x?access_token=%5BREDACTED%5D");
        assert_eq!(entry["__meta"]["redirectLocation"], "/y?secret=%5BREDACTED%5D");
        assert_eq!(entry["http"]["requestHeaders"]["cookie"], REDACTED);
        assert_eq!(entry["http"]["requestHeaders"]["accept"], "text/html");
        assert_eq!(entry["http"]["responseHeaders"]["set-cookie"], REDACTED);
        assert_eq!(entry["http"]["responseHeaders"]["Authorization"], REDACTED);
        assert_eq!(entry["http"]["requestBody"]["value"]["password"], REDACTED);
        assert_eq!(entry["http"]["requestBody"]["value"]["email"], "e");
        assert_eq!(entry["propValues"]["auth"]["token"], REDACTED);
        assert_eq!(entry["propValues"]["auth"]["name"], "n");
    }

    #[test]
    fn indt_empty_lists_redact_nothing() {
        let redactor = Redactor::new(&[String::new()], &[]);
        let mut value = json!({"password": "p", "requestHeaders": {"cookie": "c"}});
        let before = value.clone();
        redactor.redact_entry(&mut value);
        assert_eq!(value, before);
    }
}
