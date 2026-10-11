//! The types the Rust scan tracks, so a method call can be classified by
//! the method it resolves to on its receiver's type (REG-030). Tracking is
//! deliberately shallow: a type the scan cannot name is `Unknown`, and a
//! method called on an `Unknown` receiver is refused, so a gap here costs
//! an author an annotation, never a missed capability.

/// A type as far as the scan can name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Ty {
    /// The scan cannot name the type.
    Unknown,
    /// `()`.
    Unit,
    /// A primitive: `bool`, `char`, `str`, an integer or a float.
    Prim(String),
    /// An effect-free `std` type by its last segment (`String`, `Vec`,
    /// `Option`, `Result`, `HashMap`, `Box`, ...), with its type arguments.
    /// `Iter` stands for any iterator over its one argument.
    Std(String, Vec<Ty>),
    /// A type the component defines, by full path.
    Own(String),
    /// A Suprnova type, by the path the allowlist admitted.
    Api(String),
    /// A tuple.
    Tuple(Vec<Ty>),
    /// A slice or an array.
    Slice(Box<Ty>),
    /// A generic parameter, `impl Trait` or `dyn Trait`, by its bounds.
    Bounded(Vec<Bound>),
    /// A closure, by the type its body evaluates to.
    Closure(Box<Ty>),
}

/// One trait bound of a generic parameter or trait object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Bound {
    /// A trait the component defines.
    Own(String),
    /// A Suprnova trait.
    Api(String),
    /// An effect-free `std` trait.
    Std(String),
}

/// The primitive type names.
pub(super) const PRIMITIVES: &[&str] = &[
    "bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64",
    "i128", "isize", "f32", "f64",
];

/// Method names every type gets from the `std` traits a component may
/// derive or implement (`Clone`, `PartialEq`, `Ord`, `Hash`, `Debug`,
/// `Display`, `Default`, `Into`, `AsRef`, `Serialize`, ...). Calling one
/// runs the type's implementation of a fixed `std` trait.
pub(super) const STD_TRAIT_METHODS: &[&str] = &[
    "clone",
    "clone_from",
    "to_owned",
    "to_string",
    "eq",
    "ne",
    "lt",
    "le",
    "gt",
    "ge",
    "cmp",
    "partial_cmp",
    "max",
    "min",
    "clamp",
    "hash",
    "fmt",
    "into",
    "try_into",
    "as_ref",
    "as_mut",
    "borrow",
    "borrow_mut",
    "serialize",
    "deserialize",
];

impl Ty {
    /// The string type.
    pub(super) fn string() -> Ty {
        Ty::Std("String".into(), Vec::new())
    }

    /// A primitive by name.
    pub(super) fn prim(name: &str) -> Ty {
        Ty::Prim(name.into())
    }

    /// An `std` generic by name.
    pub(super) fn std(name: &str, args: Vec<Ty>) -> Ty {
        Ty::Std(name.into(), args)
    }

    fn arg(&self, index: usize) -> Ty {
        match self {
            Ty::Std(_, args) => args.get(index).cloned().unwrap_or(Ty::Unknown),
            _ => Ty::Unknown,
        }
    }

    /// The type a smart pointer points at, for auto-deref.
    pub(super) fn deref_target(&self) -> Option<Ty> {
        match self {
            Ty::Std(name, args) if matches!(name.as_str(), "Box" | "Rc" | "Arc" | "Cow") => {
                args.first().cloned()
            }
            _ => None,
        }
    }

    /// Whether every part of the type is effect-free `std`, a primitive or
    /// a tuple of those, so its methods are `std`'s.
    pub(super) fn is_std(&self) -> bool {
        matches!(
            self,
            Ty::Unit | Ty::Prim(_) | Ty::Std(..) | Ty::Tuple(_) | Ty::Slice(_) | Ty::Closure(_)
        )
    }

    /// The element type iterating this type yields.
    pub(super) fn element(&self) -> Ty {
        match self {
            Ty::Std(name, args) => match name.as_str() {
                "Vec" | "VecDeque" | "HashSet" | "BTreeSet" | "Iter" | "Option" | "BinaryHeap" => {
                    args.first().cloned().unwrap_or(Ty::Unknown)
                }
                "HashMap" | "BTreeMap" => Ty::Tuple(vec![self.arg(0), self.arg(1)]),
                "Result" => self.arg(0),
                _ => Ty::Unknown,
            },
            Ty::Slice(inner) => (**inner).clone(),
            _ => Ty::Unknown,
        }
    }
}

/// The types of the parameters of a closure passed as argument `index` of
/// `method` on a receiver of type `receiver`, when the scan knows them.
pub(super) fn closure_params(receiver: &Ty, method: &str, index: usize, first_arg: &Ty) -> Vec<Ty> {
    let element = receiver.element();
    match receiver {
        Ty::Std(name, _) if name == "Option" => match (method, index) {
            (
                "map" | "and_then" | "filter" | "is_some_and" | "is_none_or" | "inspect"
                | "take_if",
                0,
            )
            | ("map_or", 1)
            | ("map_or_else", 1) => vec![element],
            _ => Vec::new(),
        },
        Ty::Std(name, _) if name == "Result" => match (method, index) {
            ("map" | "and_then" | "is_ok_and" | "inspect", 0)
            | ("map_or", 1)
            | ("map_or_else", 1) => {
                vec![receiver.arg(0)]
            }
            ("map_err" | "or_else" | "unwrap_or_else" | "is_err_and" | "inspect_err", 0)
            | ("map_or_else", 0) => vec![receiver.arg(1)],
            _ => Vec::new(),
        },
        Ty::Std(name, _) if matches!(name.as_str(), "HashMap" | "BTreeMap") => match method {
            "retain" => vec![receiver.arg(0), receiver.arg(1)],
            _ => Vec::new(),
        },
        Ty::Std(name, _) if name == "Entry" => match method {
            "and_modify" => vec![receiver.arg(1)],
            _ => Vec::new(),
        },
        Ty::Std(..) | Ty::Slice(_) => match (method, index) {
            (
                "map"
                | "filter"
                | "for_each"
                | "any"
                | "all"
                | "find"
                | "position"
                | "rposition"
                | "filter_map"
                | "flat_map"
                | "take_while"
                | "skip_while"
                | "map_while"
                | "inspect"
                | "max_by_key"
                | "min_by_key"
                | "partition"
                | "find_map"
                | "retain"
                | "retain_mut"
                | "sort_by_key"
                | "sort_unstable_by_key"
                | "sort_by_cached_key"
                | "dedup_by_key"
                | "is_sorted_by_key"
                | "binary_search_by_key"
                | "binary_search_by"
                | "count_by",
                0,
            ) => vec![element],
            ("sort_by" | "sort_unstable_by" | "max_by" | "min_by" | "dedup_by" | "reduce", 0) => {
                vec![element.clone(), element]
            }
            ("fold" | "try_fold", 1) => vec![first_arg.clone(), element],
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// The type a `std` method returns on a `std` receiver, as far as the scan
/// knows it. `args` holds each argument's type, closures as
/// `Ty::Closure(<body type>)`; `turbofish` the method's first explicit type
/// argument.
pub(super) fn std_method_result(
    receiver: &Ty,
    method: &str,
    args: &[Ty],
    turbofish: Option<&Ty>,
) -> Ty {
    let same = receiver.clone();
    let closure_result = |index: usize| match args.get(index) {
        Some(Ty::Closure(body)) => (**body).clone(),
        _ => Ty::Unknown,
    };
    let element = receiver.element();
    // Methods every type shares through the std traits.
    match method {
        "clone" | "to_owned" | "max" | "min" | "clamp" => return same,
        "to_string" => return Ty::string(),
        "eq" | "ne" | "lt" | "le" | "gt" | "ge" => return Ty::prim("bool"),
        "cmp" => return Ty::std("Ordering", Vec::new()),
        "partial_cmp" => return Ty::std("Option", vec![Ty::std("Ordering", Vec::new())]),
        "into" | "try_into" | "parse" | "collect" | "sum" | "product" => {
            return match (method, turbofish) {
                ("parse", Some(target)) => Ty::std("Result", vec![target.clone(), Ty::Unknown]),
                ("collect", Some(Ty::Std(name, args)))
                    if args.iter().all(|a| *a == Ty::Unknown) =>
                {
                    match name.as_str() {
                        "HashMap" | "BTreeMap" => match element {
                            Ty::Tuple(parts) => Ty::Std(name.clone(), parts),
                            _ => Ty::Std(name.clone(), vec![Ty::Unknown, Ty::Unknown]),
                        },
                        "String" => Ty::string(),
                        _ => Ty::Std(name.clone(), vec![element]),
                    }
                }
                (_, Some(target)) => target.clone(),
                _ => Ty::Unknown,
            };
        }
        _ => {}
    }
    match receiver {
        Ty::Prim(name) if name == "str" => string_method(method, args),
        Ty::Std(name, _) if name == "String" => string_method(method, args),
        Ty::Prim(name) if name == "bool" => match method {
            "then" => Ty::std("Option", vec![closure_result(0)]),
            "then_some" => Ty::std("Option", vec![args.first().cloned().unwrap_or(Ty::Unknown)]),
            _ => Ty::Unknown,
        },
        Ty::Prim(name) if name == "char" => match method {
            "to_ascii_uppercase" | "to_ascii_lowercase" => same,
            "to_digit" => Ty::std("Option", vec![Ty::prim("u32")]),
            "len_utf8" | "len_utf16" => Ty::prim("usize"),
            "to_uppercase" | "to_lowercase" => Ty::std("Iter", vec![Ty::prim("char")]),
            m if m.starts_with("is_") || m == "eq_ignore_ascii_case" => Ty::prim("bool"),
            _ => Ty::Unknown,
        },
        Ty::Prim(_) => number_method(receiver, method),
        Ty::Std(name, _) => match name.as_str() {
            "Option" => match method {
                "is_some" | "is_none" | "is_some_and" | "is_none_or" => Ty::prim("bool"),
                "unwrap" | "expect" | "unwrap_or" | "unwrap_or_default" | "unwrap_or_else"
                | "get_or_insert" | "get_or_insert_with" | "insert" => element,
                "map" => Ty::std("Option", vec![closure_result(0)]),
                "and_then" => closure_result(0),
                "filter" | "take" | "replace" | "or" | "or_else" | "xor" | "as_ref" | "as_mut"
                | "as_deref" | "as_deref_mut" | "cloned" | "copied" | "inspect" | "take_if" => same,
                "ok_or" | "ok_or_else" => Ty::std("Result", vec![element, Ty::Unknown]),
                "map_or" => args.first().cloned().unwrap_or(Ty::Unknown),
                "map_or_else" => closure_result(1),
                "iter" | "iter_mut" | "into_iter" => Ty::std("Iter", vec![element]),
                "zip" => Ty::std(
                    "Option",
                    vec![Ty::Tuple(vec![
                        element,
                        args.first().map(Ty::element).unwrap_or(Ty::Unknown),
                    ])],
                ),
                "flatten" => element,
                _ => Ty::Unknown,
            },
            "Result" => match method {
                "is_ok" | "is_err" | "is_ok_and" | "is_err_and" => Ty::prim("bool"),
                "unwrap" | "expect" | "unwrap_or" | "unwrap_or_default" | "unwrap_or_else" => {
                    receiver.arg(0)
                }
                "unwrap_err" | "expect_err" => receiver.arg(1),
                "ok" => Ty::std("Option", vec![receiver.arg(0)]),
                "err" => Ty::std("Option", vec![receiver.arg(1)]),
                "map" => Ty::std("Result", vec![closure_result(0), receiver.arg(1)]),
                "map_err" => Ty::std("Result", vec![receiver.arg(0), closure_result(0)]),
                "and_then" => closure_result(0),
                "as_ref" | "as_mut" | "inspect" | "inspect_err" | "or" | "or_else" | "cloned"
                | "copied" => same,
                "iter" | "into_iter" => Ty::std("Iter", vec![receiver.arg(0)]),
                _ => Ty::Unknown,
            },
            "Vec" | "VecDeque" | "BinaryHeap" => sequence_method(receiver, method, args),
            "HashMap" | "BTreeMap" => map_method(receiver, method),
            "Entry" => match method {
                "or_insert" | "or_insert_with" | "or_default" | "or_insert_with_key" => {
                    receiver.arg(1)
                }
                "and_modify" => same,
                "key" => receiver.arg(0),
                _ => Ty::Unknown,
            },
            "HashSet" | "BTreeSet" => match method {
                "insert" | "remove" | "contains" | "is_empty" | "is_subset" | "is_superset"
                | "is_disjoint" => Ty::prim("bool"),
                "len" => Ty::prim("usize"),
                "iter"
                | "into_iter"
                | "drain"
                | "union"
                | "intersection"
                | "difference"
                | "symmetric_difference" => Ty::std("Iter", vec![element]),
                "get" | "take" | "first" | "last" | "pop_first" | "pop_last" | "replace" => {
                    Ty::std("Option", vec![element])
                }
                "clear" | "retain" | "extend" => Ty::Unit,
                _ => Ty::Unknown,
            },
            "Iter" => iterator_method(receiver, method, args, &closure_result),
            "Duration" => match method {
                "as_secs" | "as_millis" | "as_micros" | "as_nanos" => Ty::prim("u128"),
                "subsec_millis" | "subsec_micros" | "subsec_nanos" => Ty::prim("u32"),
                "as_secs_f64" | "as_secs_f32" => Ty::prim("f64"),
                "is_zero" => Ty::prim("bool"),
                "checked_add" | "checked_sub" | "checked_mul" | "checked_div" => {
                    Ty::std("Option", vec![same])
                }
                "saturating_add" | "saturating_sub" | "saturating_mul" | "mul_f64" | "mul_f32"
                | "div_f64" | "div_f32" | "abs_diff" => same,
                _ => Ty::Unknown,
            },
            "Ordering" => match method {
                "reverse" | "then" | "then_with" => same,
                m if m.starts_with("is_") => Ty::prim("bool"),
                _ => Ty::Unknown,
            },
            _ => Ty::Unknown,
        },
        Ty::Slice(_) => sequence_method(&Ty::std("Vec", vec![element]), method, args),
        _ => Ty::Unknown,
    }
}

fn string_method(method: &str, args: &[Ty]) -> Ty {
    let _ = args;
    match method {
        "len" | "capacity" => Ty::prim("usize"),
        "is_empty"
        | "contains"
        | "starts_with"
        | "ends_with"
        | "eq_ignore_ascii_case"
        | "is_char_boundary"
        | "is_ascii" => Ty::prim("bool"),
        "trim" | "trim_start" | "trim_end" | "as_str" | "trim_matches" | "trim_start_matches"
        | "trim_end_matches" => Ty::prim("str"),
        "strip_prefix" | "strip_suffix" | "get" => Ty::std("Option", vec![Ty::prim("str")]),
        "to_lowercase" | "to_uppercase" | "to_ascii_lowercase" | "to_ascii_uppercase"
        | "replace" | "replacen" | "repeat" | "into_string" | "into_boxed_str" => Ty::string(),
        "push"
        | "push_str"
        | "clear"
        | "insert"
        | "insert_str"
        | "truncate"
        | "retain"
        | "make_ascii_lowercase"
        | "make_ascii_uppercase"
        | "reserve"
        | "shrink_to_fit" => Ty::Unit,
        "pop" => Ty::std("Option", vec![Ty::prim("char")]),
        "remove" => Ty::prim("char"),
        "find" | "rfind" => Ty::std("Option", vec![Ty::prim("usize")]),
        "split_once" | "rsplit_once" => Ty::std(
            "Option",
            vec![Ty::Tuple(vec![Ty::prim("str"), Ty::prim("str")])],
        ),
        "as_bytes" | "bytes" => match method {
            "bytes" => Ty::std("Iter", vec![Ty::prim("u8")]),
            _ => Ty::Slice(Box::new(Ty::prim("u8"))),
        },
        "chars" => Ty::std("Iter", vec![Ty::prim("char")]),
        "char_indices" => Ty::std(
            "Iter",
            vec![Ty::Tuple(vec![Ty::prim("usize"), Ty::prim("char")])],
        ),
        "split"
        | "rsplit"
        | "splitn"
        | "rsplitn"
        | "split_whitespace"
        | "split_terminator"
        | "lines"
        | "split_ascii_whitespace"
        | "matches"
        | "split_inclusive" => Ty::std("Iter", vec![Ty::prim("str")]),
        _ => Ty::Unknown,
    }
}

fn number_method(receiver: &Ty, method: &str) -> Ty {
    let same = receiver.clone();
    match method {
        m if m.starts_with("checked_") => Ty::std("Option", vec![same]),
        m if m.starts_with("overflowing_") => Ty::Tuple(vec![same, Ty::prim("bool")]),
        m if m.starts_with("saturating_") || m.starts_with("wrapping_") => same,
        m if m.starts_with("is_") => Ty::prim("bool"),
        "count_ones" | "count_zeros" | "leading_zeros" | "trailing_zeros" | "leading_ones"
        | "trailing_ones" | "ilog2" | "ilog10" => Ty::prim("u32"),
        "pow" | "abs" | "signum" | "rem_euclid" | "div_euclid" | "rotate_left" | "rotate_right"
        | "swap_bytes" | "reverse_bits" | "floor" | "ceil" | "round" | "trunc" | "fract"
        | "sqrt" | "powi" | "powf" | "exp" | "ln" | "log10" | "log2" | "sin" | "cos" | "tan"
        | "abs_diff" | "unsigned_abs" | "next_power_of_two" | "midpoint" | "copysign" | "recip"
        | "to_degrees" | "to_radians" | "hypot" | "mul_add" | "cbrt" | "round_ties_even" => same,
        _ => Ty::Unknown,
    }
}

fn sequence_method(receiver: &Ty, method: &str, args: &[Ty]) -> Ty {
    let element = receiver.element();
    let _ = args;
    match method {
        "len" | "capacity" => Ty::prim("usize"),
        "is_empty" | "contains" | "starts_with" | "ends_with" | "is_sorted" => Ty::prim("bool"),
        "push"
        | "push_back"
        | "push_front"
        | "insert"
        | "clear"
        | "truncate"
        | "sort"
        | "sort_unstable"
        | "sort_by"
        | "sort_by_key"
        | "sort_unstable_by"
        | "sort_unstable_by_key"
        | "sort_by_cached_key"
        | "reverse"
        | "retain"
        | "retain_mut"
        | "dedup"
        | "dedup_by"
        | "dedup_by_key"
        | "extend"
        | "extend_from_slice"
        | "resize"
        | "swap"
        | "fill"
        | "shrink_to_fit"
        | "reserve"
        | "append"
        | "rotate_left"
        | "rotate_right" => Ty::Unit,
        "pop" | "pop_back" | "pop_front" | "first" | "last" | "get" | "get_mut" | "first_mut"
        | "last_mut" | "front" | "back" | "front_mut" | "back_mut" | "peek" => {
            Ty::std("Option", vec![element])
        }
        "remove" | "swap_remove" => element,
        "iter" | "iter_mut" | "into_iter" | "drain" => Ty::std("Iter", vec![element]),
        "to_vec" | "split_off" | "into_vec" => Ty::std("Vec", vec![element]),
        "as_slice" | "as_mut_slice" => Ty::Slice(Box::new(element)),
        "join" | "concat" => Ty::string(),
        "windows" | "chunks" | "chunks_exact" | "rchunks" => {
            Ty::std("Iter", vec![Ty::Slice(Box::new(element))])
        }
        "binary_search" | "binary_search_by" | "binary_search_by_key" => {
            Ty::std("Result", vec![Ty::prim("usize"), Ty::prim("usize")])
        }
        _ => Ty::Unknown,
    }
}

fn map_method(receiver: &Ty, method: &str) -> Ty {
    let key = receiver.arg(0);
    let value = receiver.arg(1);
    match method {
        "get" | "get_mut" | "remove" | "insert" => Ty::std("Option", vec![value]),
        "contains_key" | "is_empty" => Ty::prim("bool"),
        "len" => Ty::prim("usize"),
        "keys" | "into_keys" => Ty::std("Iter", vec![key]),
        "values" | "values_mut" | "into_values" => Ty::std("Iter", vec![value]),
        "iter" | "iter_mut" | "into_iter" | "drain" => {
            Ty::std("Iter", vec![Ty::Tuple(vec![key, value])])
        }
        "entry" => Ty::std("Entry", vec![key, value]),
        "get_key_value" | "first_key_value" | "last_key_value" | "pop_first" | "pop_last"
        | "remove_entry" => Ty::std("Option", vec![Ty::Tuple(vec![key, value])]),
        "clear" | "retain" | "extend" => Ty::Unit,
        _ => Ty::Unknown,
    }
}

fn iterator_method(
    receiver: &Ty,
    method: &str,
    args: &[Ty],
    closure_result: &dyn Fn(usize) -> Ty,
) -> Ty {
    let element = receiver.element();
    let iter = |inner: Ty| Ty::std("Iter", vec![inner]);
    match method {
        "map" => iter(closure_result(0)),
        "filter" | "skip" | "take" | "rev" | "peekable" | "chain" | "cycle" | "step_by"
        | "fuse" | "inspect" | "skip_while" | "take_while" | "cloned" | "copied" | "by_ref" => {
            receiver.clone()
        }
        "enumerate" => iter(Ty::Tuple(vec![Ty::prim("usize"), element])),
        "zip" => iter(Ty::Tuple(vec![
            element,
            args.first().map(Ty::element).unwrap_or(Ty::Unknown),
        ])),
        "filter_map" | "map_while" => match closure_result(0) {
            Ty::Std(name, inner) if name == "Option" => {
                iter(inner.into_iter().next().unwrap_or(Ty::Unknown))
            }
            _ => iter(Ty::Unknown),
        },
        "flat_map" => iter(closure_result(0).element()),
        "flatten" => iter(element.element()),
        "next" | "last" | "nth" | "min" | "max" | "find" | "max_by_key" | "min_by_key"
        | "max_by" | "min_by" | "reduce" | "next_back" | "peek" => Ty::std("Option", vec![element]),
        "find_map" => closure_result(0),
        "count" => Ty::prim("usize"),
        "position" | "rposition" => Ty::std("Option", vec![Ty::prim("usize")]),
        "any" | "all" | "is_sorted" => Ty::prim("bool"),
        "for_each" => Ty::Unit,
        "fold" => args.first().cloned().unwrap_or(Ty::Unknown),
        _ => Ty::Unknown,
    }
}
