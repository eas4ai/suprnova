//! Every form a `route()` or `try_route()` call passed its string pairs in
//! before route binding, when the parameter was `&[(&str, &str)]`: each
//! must still compile (BIND-012), apart from the four the specification
//! lists, which compile as the manual rewrites them.

use std::borrow::Cow;
use std::rc::Rc;
use std::sync::Arc;

use suprnova::{route, try_route};

/// Calls written against the slice parameter.
pub fn every_form(
    id: String,
    name: &str,
    slice: &[(&str, &str)],
    mut pairs: Vec<(&str, &str)>,
    mut array: [(&str, &str); 2],
    cow: Cow<'_, str>,
    boxed: Box<str>,
    rc: Rc<str>,
    arc: Arc<str>,
) -> Vec<Option<String>> {
    let mut owned = id.clone();
    let mut urls = vec![
        // Arrays, empty and not.
        route("a", &[]),
        route("a", &mut []),
        route("a", &[("id", "1")]),
        route("a", &array),
        route("a", &mut array),
        // Slices, borrowed and mutable.
        route("a", slice),
        route("a", &slice),
        route("a", &mut array[..]),
        route("a", &pairs[..]),
        route("a", pairs.as_slice()),
        // Vectors, borrowed and mutable.
        route("a", &pairs),
        route("a", &mut pairs),
        route("a", &vec![("id", "1")]),
        route("a", &vec![]),
        // Names and values that deref to a string.
        route("a", &[("id", &id)]),
        route("a", &[("id", &name)]),
        route("a", &[("id", &&id)]),
        route("a", &[("id", &mut owned)]),
        route("a", &[("id", &cow)]),
        route("a", &[("id", &boxed)]),
        route("a", &[("id", &rc)]),
        route("a", &[("id", &arc)]),
        route("a", &[(&id, "1")]),
        route("a", &[(&id, &id)]),
        route("a", &[("other", "x"), ("id", &id)]),
        route("a", &[("id", id.as_str())]),
        route("a", &[("id", &*id)]),
    ];
    urls.extend([
        try_route("a", &[]).ok(),
        try_route("a", slice).ok(),
        try_route("a", &pairs).ok(),
        try_route("a", &mut pairs).ok(),
        try_route("a", &[("id", &id)]).ok(),
    ]);
    urls
}

/// The same calls through a mutable borrow: a `&mut` array literal was
/// coerced to the slice parameter element by element, as a `&` one was, and
/// an empty mutable container took its pair type from the parameter.
pub fn mutable_forms(
    id: String,
    name: &str,
    cow: Cow<'_, str>,
    boxed: Box<str>,
    rc: Rc<str>,
    arc: Arc<str>,
) -> Vec<Option<String>> {
    let mut owned = id.clone();
    vec![
        // Names and values that deref to a string, in a mutable array.
        route("a", &mut [("id", &id)]),
        try_route("a", &mut [("id", &id)]).ok(),
        route("a", &mut [("id", &name)]),
        try_route("a", &mut [("id", &name)]).ok(),
        route("a", &mut [("id", &&id)]),
        try_route("a", &mut [("id", &&id)]).ok(),
        route("a", &mut [("id", &mut owned)]),
        try_route("a", &mut [("id", &mut owned)]).ok(),
        route("a", &mut [("id", &cow)]),
        try_route("a", &mut [("id", &cow)]).ok(),
        route("a", &mut [("id", &boxed)]),
        try_route("a", &mut [("id", &boxed)]).ok(),
        route("a", &mut [("id", &rc)]),
        try_route("a", &mut [("id", &rc)]).ok(),
        route("a", &mut [("id", &arc)]),
        try_route("a", &mut [("id", &arc)]).ok(),
        route("a", &mut [(&id, "1")]),
        try_route("a", &mut [(&id, "1")]).ok(),
        route("a", &mut [(&id, &id)]),
        try_route("a", &mut [(&id, &id)]).ok(),
        route("a", &mut [("other", "x"), ("id", &id)]),
        try_route("a", &mut [("other", "x"), ("id", &id)]).ok(),
        route("a", &mut [("id", id.as_str())]),
        try_route("a", &mut [("id", id.as_str())]).ok(),
        route("a", &mut [("id", &*id)]),
        try_route("a", &mut [("id", &*id)]).ok(),
        // Thirty-two pairs, the most an array of `&String` values takes.
        route(
            "a",
            &mut [
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
            ],
        ),
        try_route(
            "a",
            &mut [
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
                ("id", &id),
            ],
        )
        .ok(),
        // Empty mutable containers, whose pair type the parameter set.
        route("a", &mut []),
        try_route("a", &mut []).ok(),
        route("a", &mut [][..]),
        try_route("a", &mut [][..]).ok(),
        route("a", &mut vec![]),
        try_route("a", &mut vec![]).ok(),
        route("a", &mut Vec::new()),
        try_route("a", &mut Vec::new()).ok(),
        route("a", &Vec::new()),
        try_route("a", &Vec::new()).ok(),
        try_route("a", &vec![]).ok(),
    ]
}

/// The four forms that compiled only because the slice parameter set the
/// pairs' type, written as the manual says (BIND-012): `.as_str()` on the
/// value, and a closure in place of the function pointer.
pub fn rewritten_forms(id: String) -> Vec<Option<String>> {
    let pointer: fn(&str, &[(&str, &str)]) -> Option<String> = |name, params| route(name, params);
    vec![
        // Was `("id", id.as_ref())`.
        route("a", &[("id", id.as_str())]),
        // Was `[("id", &id), ("other", "x")]`.
        route("a", &[("id", id.as_str()), ("other", "x")]),
        // Was more than 32 pairs holding `&id`.
        route("a", &[("id", id.as_str()); 33]),
        // Was `let pointer: fn(&str, &[(&str, &str)]) -> Option<String> = route;`.
        pointer("a", &[("id", "1")]),
    ]
}
