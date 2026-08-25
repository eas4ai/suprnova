//! Laravel `Arr::set` / `Arr::get` dot-notation semantics
//! (`reference/framework-13.25.0/src/Illuminate/Collections/Arr.php:1018-1046,487-514`),
//! reimplemented over `serde_json::Value` for Inertia prop keys.
//!
//! Three entry points, all crate-internal:
//!
//! - `arr_set` sets one dotted key on a map, nesting as needed.
//! - `unpack_map` applies `arr_set` to every entry of a flat map, in
//!   order - `InertiaResponse::resolve`'s final pass over the fully
//!   resolved prop bag, mirroring Laravel's `resolveArrayableProperties`
//!   unpack step (`reference/inertia-laravel-2.0.25/src/Response.php:344-368`).
//! - `arr_get` reads a (possibly dotted) key back out of a nested value -
//!   `InertiaRegistry::shared_value`'s read-back, Laravel's `Inertia::getShared`.
//!
//! All three operate on `serde_json::Map`, which the framework builds with
//! `serde_json`'s `preserve_order` feature (`framework/Cargo.toml:24`) -
//! iteration order matches insertion order, so repeated `arr_set` calls
//! compose exactly like repeated `Arr::set` calls on the same PHP array.

use serde_json::Value;

/// Laravel's `Arr::set($array, $key, $value)` (`Arr.php:1018-1046`), minus
/// the `$key === null` "replace the whole array" case - callers here
/// always have a real key.
///
/// Splits `key` on `.`; every non-final segment becomes (or stays) a
/// nested object. A segment that already holds a non-object value is
/// **overwritten** with a fresh empty object - `Arr::set`'s
/// `! is_array($array[$key])` branch - so an intermediate scalar is
/// silently discarded rather than causing an error. Call it repeatedly on
/// the same `map` to accumulate sibling leaves under one parent, the same
/// way multiple `Arr::set` calls build up one nested PHP array.
///
/// There is no way to opt a key out of nesting - a literal `.` in a key
/// (for example `"config.json"`) always splits. This matches Laravel:
/// `Arr::set` has no escaping mechanism either, so `Inertia::share('config.json', …)`
/// nests the same way on the PHP side.
pub(crate) fn arr_set(map: &mut serde_json::Map<String, Value>, key: &str, value: Value) {
    match key.split_once('.') {
        None => {
            map.insert(key.to_string(), value);
        }
        Some((head, rest)) => {
            let entry = map
                .entry(head.to_string())
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            if !entry.is_object() {
                *entry = Value::Object(serde_json::Map::new());
            }
            arr_set(
                entry
                    .as_object_mut()
                    .expect("just ensured this is an object"),
                rest,
                value,
            );
        }
    }
}

/// Apply `arr_set` to every entry of `map`, in iteration (= insertion)
/// order. A flat prop bag where some keys happen to contain `.` becomes
/// the nested tree those dots describe - repeated calls with the same
/// prefix accumulate, and a later plain key overwrites whatever an
/// earlier dotted key built at that position, exactly as sequential
/// `Arr::set` calls would. Never recurses into a value that was already a
/// `Value::Object` when it arrived - only the top-level keys of `map`
/// itself carry dot meaning.
pub(crate) fn unpack_map(map: serde_json::Map<String, Value>) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for (key, value) in map {
        arr_set(&mut out, &key, value);
    }
    out
}

/// Laravel's `Arr::get($array, $key, $default)` (`Arr.php:487-514`), minus
/// the default - callers get `Option` and choose their own fallback.
/// `root` must be a JSON object; anything else returns `None` (`Arr::get`'s
/// `! static::accessible($array)` branch).
///
/// Tries an exact top-level match first - `Arr::get`'s
/// `static::exists($array, $key)` check - so a literal dotted key (one
/// inserted directly rather than via `arr_set`) is still found without
/// dot-traversal. Only when that misses, and `key` contains a `.`, does it
/// walk segments.
pub(crate) fn arr_get(root: &Value, key: &str) -> Option<Value> {
    let object = root.as_object()?;
    if let Some(v) = object.get(key) {
        return Some(v.clone());
    }
    if !key.contains('.') {
        return None;
    }
    let mut current = root;
    for segment in key.split('.') {
        current = current.as_object()?.get(segment)?;
    }
    Some(current.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn arr_set_flat_key_is_a_plain_insert() {
        let mut map = serde_json::Map::new();
        arr_set(&mut map, "appName", json!("Suprnova"));
        assert_eq!(map.get("appName"), Some(&json!("Suprnova")));
    }

    #[test]
    fn arr_set_nests_a_dotted_key() {
        let mut map = serde_json::Map::new();
        arr_set(&mut map, "user.name", json!("Todd"));
        assert_eq!(
            map,
            json!({ "user": { "name": "Todd" } })
                .as_object()
                .unwrap()
                .clone()
        );
    }

    #[test]
    fn arr_set_accumulates_multiple_dotted_keys_under_one_parent() {
        let mut map = serde_json::Map::new();
        arr_set(&mut map, "user.name", json!("Todd"));
        arr_set(&mut map, "user.age", json!(30));
        assert_eq!(
            map,
            json!({ "user": { "name": "Todd", "age": 30 } })
                .as_object()
                .unwrap()
                .clone()
        );
    }

    #[test]
    fn arr_set_overwrites_a_scalar_intermediate_with_a_fresh_object() {
        let mut map = serde_json::Map::new();
        arr_set(&mut map, "user", json!("scalar"));
        arr_set(&mut map, "user.name", json!("Todd"));
        // The scalar is gone - `Arr::set`'s documented behaviour, not an error.
        assert_eq!(
            map,
            json!({ "user": { "name": "Todd" } })
                .as_object()
                .unwrap()
                .clone()
        );
    }

    #[test]
    fn arr_set_a_later_plain_key_overwrites_an_earlier_nested_object() {
        let mut map = serde_json::Map::new();
        arr_set(&mut map, "user.name", json!("Todd"));
        arr_set(&mut map, "user", json!("scalar"));
        assert_eq!(map.get("user"), Some(&json!("scalar")));
    }

    // ---- edge-case keys: trailing / doubled / leading dot, empty key ----
    //
    // These pin behavior the recursion falls out of naturally (no
    // special-casing) rather than by explicit design - hand-traced against
    // PHP's `Arr::set` (`explode('.', $key)` walks the same segment list)
    // and confirmed byte-for-byte identical.

    #[test]
    fn arr_set_trailing_dot_nests_the_empty_final_segment() {
        let mut map = serde_json::Map::new();
        arr_set(&mut map, "user.", json!("Todd"));
        assert_eq!(
            map,
            json!({ "user": { "": "Todd" } })
                .as_object()
                .unwrap()
                .clone()
        );
    }

    #[test]
    fn arr_set_doubled_dot_nests_an_empty_intermediate_segment() {
        let mut map = serde_json::Map::new();
        arr_set(&mut map, "user..name", json!("Todd"));
        assert_eq!(
            map,
            json!({ "user": { "": { "name": "Todd" } } })
                .as_object()
                .unwrap()
                .clone()
        );
    }

    #[test]
    fn arr_set_leading_dot_nests_under_an_empty_first_segment() {
        let mut map = serde_json::Map::new();
        arr_set(&mut map, ".user", json!("Todd"));
        assert_eq!(
            map,
            json!({ "": { "user": "Todd" } })
                .as_object()
                .unwrap()
                .clone()
        );
    }

    #[test]
    fn arr_set_empty_key_is_a_plain_insert_under_the_empty_string() {
        let mut map = serde_json::Map::new();
        arr_set(&mut map, "", json!("Todd"));
        assert_eq!(map.get(""), Some(&json!("Todd")));
    }

    // ---- deeper nesting: every prior test uses exactly one level ----

    #[test]
    fn arr_set_nests_four_levels_deep() {
        let mut map = serde_json::Map::new();
        arr_set(&mut map, "a.b.c.d", json!("leaf"));
        assert_eq!(
            map,
            json!({ "a": { "b": { "c": { "d": "leaf" } } } })
                .as_object()
                .unwrap()
                .clone()
        );
    }

    #[test]
    fn arr_set_sibling_deep_paths_compose_under_the_shared_intermediate_map() {
        let mut map = serde_json::Map::new();
        arr_set(&mut map, "a.b.c", json!("c-value"));
        arr_set(&mut map, "a.b.d", json!("d-value"));
        assert_eq!(
            map,
            json!({ "a": { "b": { "c": "c-value", "d": "d-value" } } })
                .as_object()
                .unwrap()
                .clone()
        );
    }

    #[test]
    fn unpack_map_nests_every_dotted_top_level_key_in_order() {
        let mut map = serde_json::Map::new();
        map.insert("user.name".to_string(), json!("Todd"));
        map.insert("user.age".to_string(), json!(30));
        map.insert("errors".to_string(), json!({ "user.email": "Required" }));
        let out = unpack_map(map);
        assert_eq!(out["user"], json!({ "name": "Todd", "age": 30 }));
        // Never recurses into a prop's *value* - only top-level keys nest.
        assert_eq!(out["errors"], json!({ "user.email": "Required" }));
    }

    #[test]
    fn arr_get_exact_top_level_match_wins_over_dot_traversal() {
        let mut map = serde_json::Map::new();
        // A literal dotted key inserted directly, not via arr_set.
        map.insert("user.name".to_string(), json!("literal"));
        map.insert("user".to_string(), json!({ "name": "nested" }));
        let root = Value::Object(map);
        assert_eq!(arr_get(&root, "user.name"), Some(json!("literal")));
    }

    #[test]
    fn arr_get_traverses_nested_object() {
        let root = json!({ "user": { "name": "Todd" } });
        assert_eq!(arr_get(&root, "user.name"), Some(json!("Todd")));
        assert_eq!(arr_get(&root, "user"), Some(json!({ "name": "Todd" })));
    }

    #[test]
    fn arr_get_returns_none_for_missing_path() {
        let root = json!({ "user": { "name": "Todd" } });
        assert_eq!(arr_get(&root, "user.email"), None);
        assert_eq!(arr_get(&root, "org.name"), None);
    }

    #[test]
    fn arr_get_returns_none_when_a_middle_segment_is_not_an_object() {
        let root = json!({ "user": "scalar" });
        assert_eq!(arr_get(&root, "user.name"), None);
    }
}
