//! The values [`route`](super::route) and [`try_route`](super::try_route)
//! fill a route's parameters with (BIND-012).
//!
//! A parameter takes a string or a bound value. A bound value fills a
//! `{post}` parameter with its route key and a `{post:slug}` parameter
//! with the value of its `slug` column, as Laravel's URL generator does:
//!
//! ```rust,ignore
//! route("posts.show", &[("post", "hello-world")]); // a string, named
//! route("posts.show", &post);                       // one bound value
//! route("posts.show", ("post", &post));             // one value, named
//! route("users.posts.show", (("user", &user), ("post", "hello-world")));
//! ```

use crate::database::route_binding::{RouteBinding, RouteParam};

/// A value `route()` can fill a parameter with: a string, a number, or a
/// bound value.
///
/// `#[model]` and `#[derive(RouteBinding)]` implement it for the types they
/// define, through [`bound_route_value`]. A type that implements
/// [`RouteBinding`] by hand implements it the same way:
///
/// ```rust,ignore
/// impl suprnova::RouteValue for Region {
///     fn route_value(&self, field: Option<&str>) -> Option<String> {
///         suprnova::bound_route_value(self, field)
///     }
/// }
/// ```
pub trait RouteValue {
    /// The text this value fills a parameter with. `field` is the
    /// parameter's binding field, `slug` in `{post:slug}`. `None` when the
    /// value has nothing to fill it with.
    fn route_value(&self, field: Option<&str>) -> Option<String>;
}

/// The text a bound value fills a parameter with: the value of its binding
/// column `field` ([`RouteBinding::route_field`]), else its route key.
pub fn bound_route_value<T: RouteBinding>(value: &T, field: Option<&str>) -> Option<String> {
    match field {
        Some(field) => value.route_field(field),
        None => Some(value.route_key()),
    }
}

impl RouteValue for str {
    fn route_value(&self, _field: Option<&str>) -> Option<String> {
        Some(self.to_owned())
    }
}

impl RouteValue for String {
    fn route_value(&self, _field: Option<&str>) -> Option<String> {
        Some(self.clone())
    }
}

impl<V: RouteValue + ?Sized> RouteValue for &V {
    fn route_value(&self, field: Option<&str>) -> Option<String> {
        (**self).route_value(field)
    }
}

impl<T: RouteBinding> RouteValue for RouteParam<T> {
    fn route_value(&self, field: Option<&str>) -> Option<String> {
        bound_route_value(&self.0, field)
    }
}

macro_rules! number_route_values {
    ($($number:ty),*) => {
        $(
            impl RouteValue for $number {
                fn route_value(&self, _field: Option<&str>) -> Option<String> {
                    Some(self.to_string())
                }
            }
        )*
    };
}

number_route_values!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

/// The parameters of one `route()` call.
///
/// Implemented for `&[(name, value)]` string pairs, the form every call
/// written before route binding uses; for one value of any [`RouteValue`]
/// type, which fills the route's first parameter; for one `(name, value)`
/// pair; and for a tuple of up to six `(name, value)` pairs, whose values
/// may mix strings and bound values.
pub trait RouteParameters {
    /// The text for the parameter `name`, the `position`th of the route,
    /// whose binding field is `field`.
    #[doc(hidden)]
    fn __route_value(&self, name: &str, field: Option<&str>, position: usize) -> Option<String>;
}

impl<const N: usize> RouteParameters for &[(&str, &str); N] {
    fn __route_value(&self, name: &str, _field: Option<&str>, _position: usize) -> Option<String> {
        self.iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).to_owned())
    }
}

impl RouteParameters for &[(&str, &str)] {
    fn __route_value(&self, name: &str, _field: Option<&str>, _position: usize) -> Option<String> {
        self.iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).to_owned())
    }
}

/// One value, positional: it fills the route's first parameter.
impl<V: RouteValue> RouteParameters for V {
    fn __route_value(&self, _name: &str, field: Option<&str>, position: usize) -> Option<String> {
        (position == 0).then(|| self.route_value(field)).flatten()
    }
}

/// One `(name, value)` pair.
impl<V: RouteValue> RouteParameters for (&str, V) {
    fn __route_value(&self, name: &str, field: Option<&str>, _position: usize) -> Option<String> {
        (self.0 == name)
            .then(|| self.1.route_value(field))
            .flatten()
    }
}

/// A `(name, value)` pair inside a tuple of pairs.
pub trait NamedRouteValue {
    /// The text for the parameter `name` when this pair names it.
    fn named_route_value(&self, name: &str, field: Option<&str>) -> Option<String>;
}

impl<V: RouteValue> NamedRouteValue for (&str, V) {
    fn named_route_value(&self, name: &str, field: Option<&str>) -> Option<String> {
        (self.0 == name)
            .then(|| self.1.route_value(field))
            .flatten()
    }
}

macro_rules! named_route_tuples {
    ($(($($pair:ident . $index:tt),+)),*) => {
        $(
            impl<$($pair: NamedRouteValue),+> RouteParameters for ($($pair,)+) {
                fn __route_value(&self, name: &str, field: Option<&str>, _position: usize) -> Option<String> {
                    None$(.or_else(|| self.$index.named_route_value(name, field)))+
                }
            }
        )*
    };
}

named_route_tuples!(
    (A.0, B.1),
    (A.0, B.1, C.2),
    (A.0, B.1, C.2, D.3),
    (A.0, B.1, C.2, D.3, E.4),
    (A.0, B.1, C.2, D.3, E.4, F.5)
);

#[cfg(test)]
mod tests {
    use super::*;

    fn value_of(params: impl RouteParameters, name: &str, position: usize) -> Option<String> {
        params.__route_value(name, None, position)
    }

    #[test]
    fn string_pairs_fill_by_name() {
        assert_eq!(value_of(&[("id", "7")], "id", 0).as_deref(), Some("7"));
        assert_eq!(value_of(&[("id", "7")], "slug", 1), None);
        let empty: &[(&str, &str); 0] = &[];
        assert_eq!(value_of(empty, "id", 0), None);
    }

    #[test]
    fn one_value_fills_the_first_parameter_only() {
        assert_eq!(value_of("7", "id", 0).as_deref(), Some("7"));
        assert_eq!(value_of("7", "other", 1), None);
        assert_eq!(value_of(42_u64, "id", 0).as_deref(), Some("42"));
    }

    #[test]
    fn named_pairs_fill_by_name() {
        assert_eq!(value_of(("slug", "a"), "slug", 3).as_deref(), Some("a"));
        let pairs = (("user", "1"), ("post", String::from("hello")));
        assert_eq!(value_of(pairs.clone(), "post", 1).as_deref(), Some("hello"));
        assert_eq!(value_of(pairs, "user", 0).as_deref(), Some("1"));
    }
}
