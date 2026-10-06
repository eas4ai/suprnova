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
/// Implemented for `(name, value)` string pairs in every form a call
/// written before route binding passed: a borrowed or mutable array, slice
/// or `Vec`, a borrowed slice, and, for an array of up to 32 pairs, values
/// and names that deref to a string (`&String`, `&Cow<str>`, `&&str`); for
/// one value of any [`RouteValue`] type, which fills the route's first
/// parameter; for one `(name, value)` pair; and for a tuple of up to six
/// `(name, value)` pairs, whose values may mix strings and bound values.
pub trait RouteParameters {
    /// The text for the parameter `name`, the `position`th of the route,
    /// whose binding field is `field`.
    #[doc(hidden)]
    fn __route_value(&self, name: &str, field: Option<&str>, position: usize) -> Option<String>;
}

/// The value of the pair named `name`, the first one when several are.
fn pair_value<K: AsRef<str>, V: AsRef<str>>(pairs: &[(K, V)], name: &str) -> Option<String> {
    pairs
        .iter()
        .find(|(key, _)| key.as_ref() == name)
        .map(|(_, value)| value.as_ref().to_owned())
}

/// String pairs held by a container a `&[(&str, &str)]` parameter took by
/// coercion: `&[..; N]`, `&mut [..; N]`, `&mut [..]`, `&&[..]`, `&Vec` and
/// `&mut Vec`.
macro_rules! string_pair_containers {
    ($($container:ty),* $(,)?) => {
        $(
            impl RouteParameters for $container {
                fn __route_value(
                    &self,
                    name: &str,
                    _field: Option<&str>,
                    _position: usize,
                ) -> Option<String> {
                    pair_value(&self[..], name)
                }
            }
        )*
    };
}

string_pair_containers!(
    &[(&str, &str)],
    &mut [(&str, &str)],
    &&[(&str, &str)],
    &Vec<(&str, &str)>,
    &mut Vec<(&str, &str)>,
);

impl<const N: usize> RouteParameters for &[(&str, &str); N] {
    fn __route_value(&self, name: &str, _field: Option<&str>, _position: usize) -> Option<String> {
        pair_value(&self[..], name)
    }
}

impl<const N: usize> RouteParameters for &mut [(&str, &str); N] {
    fn __route_value(&self, name: &str, _field: Option<&str>, _position: usize) -> Option<String> {
        pair_value(&self[..], name)
    }
}

/// A reference that derefs to a string other than `&str` itself: what a
/// `&[(&str, &str)]` parameter coerced a pair's name or value from inside
/// an array literal, as in `&[("id", &id.to_string())]`.
///
/// `&str` is left out so the arrays below never overlap the
/// `&[(&str, &str); N]` impl, which alone covers the empty `&[]`.
#[doc(hidden)]
pub trait DerefText {
    /// The string this reference derefs to.
    fn deref_text(&self) -> &str;
}

macro_rules! deref_texts {
    ($($text:ty),* $(,)?) => {
        $(
            impl DerefText for $text {
                fn deref_text(&self) -> &str {
                    self
                }
            }
        )*
    };
}

deref_texts!(
    &String,
    &&str,
    &&String,
    &mut String,
    &mut str,
    &std::borrow::Cow<'_, str>,
    &Box<str>,
    &std::rc::Rc<str>,
    &std::sync::Arc<str>,
);

/// Arrays of pairs whose name or value is a [`DerefText`]. They are
/// implemented per length, from 1, because an impl generic over the length
/// would also match `&[]` and leave its pair type unknown.
macro_rules! deref_text_pair_arrays {
    ($($len:literal)*) => {
        $(
            impl<V: DerefText> RouteParameters for &[(&str, V); $len] {
                fn __route_value(
                    &self,
                    name: &str,
                    _field: Option<&str>,
                    _position: usize,
                ) -> Option<String> {
                    self.iter()
                        .find(|(key, _)| *key == name)
                        .map(|(_, value)| value.deref_text().to_owned())
                }
            }

            impl<K: DerefText> RouteParameters for &[(K, &str); $len] {
                fn __route_value(
                    &self,
                    name: &str,
                    _field: Option<&str>,
                    _position: usize,
                ) -> Option<String> {
                    self.iter()
                        .find(|(key, _)| key.deref_text() == name)
                        .map(|(_, value)| (*value).to_owned())
                }
            }

            impl<K: DerefText, V: DerefText> RouteParameters for &[(K, V); $len] {
                fn __route_value(
                    &self,
                    name: &str,
                    _field: Option<&str>,
                    _position: usize,
                ) -> Option<String> {
                    self.iter()
                        .find(|(key, _)| key.deref_text() == name)
                        .map(|(_, value)| value.deref_text().to_owned())
                }
            }
        )*
    };
}

deref_text_pair_arrays!(
    1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32
);

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
