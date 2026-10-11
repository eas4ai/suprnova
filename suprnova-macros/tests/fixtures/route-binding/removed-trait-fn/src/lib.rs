//! BIND-001: the `RouteBinding` trait with `param_name` is removed. This
//! crate must fail to compile.

pub fn parameter_of<T: suprnova::RouteBinding>() -> &'static str {
    T::param_name()
}
