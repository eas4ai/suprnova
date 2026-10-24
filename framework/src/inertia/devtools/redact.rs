//! What an entry must not keep: Laravel's `RedactsSensitiveData`.
//!
//! An entry stores headers, bodies and prop values as the application saw
//! them, on disk, readable by anyone the endpoints admit. Before it is
//! stored, the value of every object key named by the redaction keys is
//! replaced by `[REDACTED]`, at any depth, as are the query parameters of
//! the same names in the entry's URLs and the values of the redaction
//! headers. Names are compared without case, so `Password` is caught by
//! `password`.
//!
//! A name is also caught by any of its parts split on `[`, `]` and `.`:
//! a multipart field keeps its flat name (`user[password]`), and a dotted
//! prop path keeps its own `propValues` leaf (`auth.password`), so the
//! whole-name match Laravel makes on PHP's nested arrays would miss both.
//!
//! A URL can carry a token anywhere the entry holds text, not only under
//! `url` and `redirectLocation`: a `Location` or `Referer` header, a
//! `redirect_to` form field, a prop. So the query of every string that is
//! a URL is redacted too, as are the URLs inside a `Link` or `Refresh`
//! header and the whole value of a header that always holds a URL.

use serde_json::Value;

/// What a redacted value is replaced by.
pub(crate) const REDACTED: &str = "[REDACTED]";

/// What a value that cannot be written as JSON is replaced by: a header
/// value that is not text, for one.
pub(crate) const UNSERIALIZABLE: &str = "[UNSERIALIZABLE]";

/// The headers whose whole value is a URL, which may be a relative
/// reference such as `reset?token=abc`, so it is redacted whatever it
/// starts with.
const URL_HEADERS: [&str; 4] = [
    "location",
    "x-inertia-location",
    "referer",
    "content-location",
];

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

    /// Whether `name` names a redaction key: as a whole, or in any of its
    /// parts split on `[`, `]` and `.`, so the flattened `user[password]`,
    /// `data[0][token]` and `auth.password` are caught as their nested
    /// forms are. A longer word (`passwords`) is not a part, so it is kept.
    fn is_sensitive_key(&self, name: &str) -> bool {
        is_listed(&self.keys, name)
            || name
                .split(['[', ']', '.'])
                .filter(|part| !part.is_empty())
                .any(|part| is_listed(&self.keys, part))
    }

    /// Whether `name` is one of the redaction headers.
    fn is_sensitive_header(&self, name: &str) -> bool {
        is_listed(&self.headers, name)
    }

    /// The storage pass over a whole entry, Laravel's
    /// `redactSensitiveStoragePayload`: the keys at any depth, then the
    /// query parameters of every `url` and `redirectLocation` and of every
    /// other string that is a URL, then the values of the redaction headers
    /// and the URLs of the other headers in both header bags.
    pub(crate) fn redact_entry(&self, entry: &mut Value) {
        self.redact_keys(entry);
        self.redact_urls(entry);
        self.redact_header_bags(entry);
    }

    /// Replace the value of every object key that names a redaction key,
    /// whole or by a part, at any depth, by `[REDACTED]`.
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
    /// `redirectLocation` key, and of every other string that
    /// [looks like a URL](looks_like_url), at any depth.
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
            Value::String(text) if looks_like_url(text) => *text = self.redact_url(text),
            _ => {}
        }
    }

    /// The value of the header `name` with the query of every URL it
    /// carries redacted: the whole value of a [`URL_HEADERS`] header, each
    /// `<...>` target of a `Link`, the target of a `Refresh`, and any other
    /// value that [looks like a URL](looks_like_url).
    fn redact_header_urls(&self, name: &str, value: &str) -> String {
        let name = name.to_ascii_lowercase();
        match name.as_str() {
            "link" => self.redact_link(value),
            "refresh" => self.redact_refresh(value),
            _ if URL_HEADERS.contains(&name.as_str()) || looks_like_url(value) => {
                self.redact_url(value)
            }
            _ => value.to_string(),
        }
    }

    /// A `Link` value with every `<...>` target redacted; the parameters
    /// after each target are kept as sent.
    fn redact_link(&self, value: &str) -> String {
        let mut redacted = String::with_capacity(value.len());
        let mut rest = value;
        while let Some(open) = rest.find('<') {
            let Some(length) = rest[open..].find('>') else {
                break;
            };
            let close = open + length;
            redacted.push_str(&rest[..=open]);
            redacted.push_str(&self.redact_url(&rest[open + 1..close]));
            redacted.push('>');
            rest = &rest[close + 1..];
        }
        redacted.push_str(rest);
        redacted
    }

    /// A `Refresh` value (`5; url=/next?token=t`) with its target
    /// redacted. A quoted target keeps its quotes; a value without `url=`
    /// is redacted whole, which reaches a target written last.
    fn redact_refresh(&self, value: &str) -> String {
        let Some(at) = value.to_ascii_lowercase().find("url=") else {
            return self.redact_url(value);
        };
        let (head, target) = value.split_at(at + "url=".len());
        let quote = target.chars().next().filter(|c| *c == '"' || *c == '\'');
        let Some(quote) = quote else {
            return format!("{head}{}", self.redact_url(target));
        };
        let quoted = &target[quote.len_utf8()..];
        match quoted.find(quote) {
            Some(end) => format!(
                "{head}{quote}{}{quote}{}",
                self.redact_url(&quoted[..end]),
                &quoted[end + quote.len_utf8()..]
            ),
            None => format!("{head}{quote}{}", self.redact_url(quoted)),
        }
    }

    /// `url` with the value of every query parameter named by the
    /// redaction keys replaced by `[REDACTED]`, percent-encoded as a query
    /// writes it. A bracketed name (`filter[secret]`) is caught by any of
    /// its parts, as an object key is, and as Laravel matches nested keys
    /// at their own depth. The rest of the URL, the other parameters and
    /// the fragment are left as they were sent.
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

    /// Whether the raw query name `name` (`token`, `filter%5Bsecret%5D`),
    /// once decoded, names a redaction key by the rule object keys follow.
    fn query_name_is_sensitive(&self, name: &str) -> bool {
        let decoded: String = url::form_urlencoded::parse(name.as_bytes())
            .next()
            .map(|(decoded, _)| decoded.into_owned())
            .unwrap_or_default();
        self.is_sensitive_key(&decoded)
    }

    /// Replace the value of every redaction header in the `requestHeaders`
    /// and `responseHeaders` objects, at any depth, by `[REDACTED]`, and
    /// redact the URLs the other headers carry.
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
                                } else if let Value::String(text) = header {
                                    *text = self.redact_header_urls(name, text);
                                }
                            }
                        }
                    } else {
                        self.redact_header_bags(member);
                    }
                }
            }
            Value::Array(items) => items
                .iter_mut()
                .for_each(|item| self.redact_header_bags(item)),
            _ => {}
        }
    }
}

/// Whether `text` is a URL with a query: it holds a `?` and starts with
/// `/` (a path or a scheme-relative URL), with `?` (a bare query), or with
/// a scheme such as `https:` or `mailto:`. Text that starts otherwise,
/// prose with a question mark included, is not taken for one.
fn looks_like_url(text: &str) -> bool {
    text.contains('?') && (text.starts_with('/') || text.starts_with('?') || has_scheme(text))
}

/// Whether `text` starts with a URL scheme and its `:`, RFC 3986's
/// `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"`.
fn has_scheme(text: &str) -> bool {
    let Some((scheme, _)) = text.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
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
    list.contains(&lowered)
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
    fn indt_a_flattened_key_is_redacted_when_any_part_names_a_key() {
        let mut value = json!({
            "user[password]": "hunter2",
            "auth.password": "p",
            "data[0][token]": "t",
            "filter[secret]": "s",
            "Profile.API_KEY": "k",
            "user[name]": "Ada",
            "passwords": "a longer word",
            "filter[passwords]": "a longer word too",
            "nested": {"user[password]": "deep"},
        });
        redactor().redact_keys(&mut value);
        assert_eq!(
            value,
            json!({
                "user[password]": REDACTED,
                "auth.password": REDACTED,
                "data[0][token]": REDACTED,
                "filter[secret]": REDACTED,
                "Profile.API_KEY": REDACTED,
                "user[name]": "Ada",
                "passwords": "a longer word",
                "filter[passwords]": "a longer word too",
                "nested": {"user[password]": REDACTED},
            })
        );
    }

    #[test]
    fn indt_a_configured_key_with_a_separator_is_matched_whole() {
        let redactor = Redactor::new(&["user.pin".to_string()], &[]);
        let mut value = json!({"User.PIN": "1234", "pin": "kept", "user": "kept"});
        redactor.redact_keys(&mut value);
        assert_eq!(
            value,
            json!({"User.PIN": REDACTED, "pin": "kept", "user": "kept"})
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
        assert_eq!(
            entry["__meta"]["url"],
            "http://app.test/x?access_token=%5BREDACTED%5D"
        );
        assert_eq!(
            entry["__meta"]["redirectLocation"],
            "/y?secret=%5BREDACTED%5D"
        );
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
    fn indt_url_query_values_are_redacted_in_header_values() {
        let mut entry = json!({
            "http": {
                "requestHeaders": {
                    "referer": "http://app.test/reset?token=abc&page=2",
                    "accept": "text/html, application/xhtml+xml",
                    "x-note": "why?token=kept",
                },
                "responseHeaders": {
                    "location": "/reset?token=abc",
                    "X-Inertia-Location": "https://billing.example/portal?access_token=a#top",
                    "content-location": "reset?secret=s&page=2",
                    "refresh": "5; url='/reset?token=abc'",
                    "link": "</next?token=n>; rel=\"next\", </prev?page=1>; rel=\"prev\"",
                    "x-custom": "/elsewhere?api_key=k",
                    "x-scheme": "mailto:ada@example.com?secret=s",
                    "content-type": "text/html; charset=utf-8",
                },
            },
        });
        redactor().redact_entry(&mut entry);
        let request = &entry["http"]["requestHeaders"];
        assert_eq!(
            request["referer"],
            "http://app.test/reset?token=%5BREDACTED%5D&page=2"
        );
        assert_eq!(request["accept"], "text/html, application/xhtml+xml");
        assert_eq!(
            request["x-note"], "why?token=kept",
            "not a URL: no scheme, and no leading / or ?"
        );
        let response = &entry["http"]["responseHeaders"];
        assert_eq!(response["location"], "/reset?token=%5BREDACTED%5D");
        assert_eq!(
            response["X-Inertia-Location"],
            "https://billing.example/portal?access_token=%5BREDACTED%5D#top"
        );
        assert_eq!(
            response["content-location"], "reset?secret=%5BREDACTED%5D&page=2",
            "a URL header may hold a relative reference"
        );
        assert_eq!(response["refresh"], "5; url='/reset?token=%5BREDACTED%5D'");
        assert_eq!(
            response["link"],
            "</next?token=%5BREDACTED%5D>; rel=\"next\", </prev?page=1>; rel=\"prev\""
        );
        assert_eq!(response["x-custom"], "/elsewhere?api_key=%5BREDACTED%5D");
        assert_eq!(
            response["x-scheme"],
            "mailto:ada@example.com?secret=%5BREDACTED%5D"
        );
        assert_eq!(response["content-type"], "text/html; charset=utf-8");
    }

    #[test]
    fn indt_a_refresh_target_is_redacted_quoted_bare_or_without_url() {
        let redactor = redactor();
        let refresh = |value: &str| {
            let mut entry = json!({"responseHeaders": {"Refresh": value}});
            redactor.redact_entry(&mut entry);
            entry["responseHeaders"]["Refresh"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(
            refresh("0;URL=\"/a?token=t\""),
            "0;URL=\"/a?token=%5BREDACTED%5D\""
        );
        assert_eq!(
            refresh("0; url=/a?token=t&page=2"),
            "0; url=/a?token=%5BREDACTED%5D&page=2"
        );
        assert_eq!(refresh("0; /a?token=t"), "0; /a?token=%5BREDACTED%5D");
        assert_eq!(refresh("30"), "30");
    }

    #[test]
    fn indt_a_string_value_that_is_a_url_loses_sensitive_query_values() {
        let mut entry = json!({
            "http": {"requestBody": {"status": "present", "value": {
                "redirect_to": "/reset?token=abc&page=2",
                "back": "https://app.test/x?secret=s#top",
                "list": ["?token=t", "/plain?page=2"],
                "note": "see the docs?",
            }}},
            "propValues": {"links": {"reset": "http://app.test/reset?api_key=k"}},
        });
        redactor().redact_entry(&mut entry);
        let input = &entry["http"]["requestBody"]["value"];
        assert_eq!(input["redirect_to"], "/reset?token=%5BREDACTED%5D&page=2");
        assert_eq!(
            input["back"],
            "https://app.test/x?secret=%5BREDACTED%5D#top"
        );
        assert_eq!(
            input["list"],
            json!(["?token=%5BREDACTED%5D", "/plain?page=2"])
        );
        assert_eq!(input["note"], "see the docs?");
        assert_eq!(
            entry["propValues"]["links"]["reset"],
            "http://app.test/reset?api_key=%5BREDACTED%5D"
        );
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
