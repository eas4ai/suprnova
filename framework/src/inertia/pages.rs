//! The run-time page check `InertiaConfig::ensure_pages_exist` turns on -
//! Laravel's `ResponseFactory::findComponentOrFail`.
//!
//! `inertia_response!` checks its component when the crate compiles. A name
//! given as a string to `InertiaResponse::new` or `Router::inertia` is not
//! seen then, and a typo in it reaches the browser as a page the client
//! cannot resolve. With the check on, such a render is an error naming the
//! component and the directory, which the error page shows the developer.

use super::config::InertiaConfig;
use crate::FrameworkError;

/// `Ok` when `component` has a page file under the configured directory
/// with one of the configured extensions.
///
/// A name with an empty, `.` or `..` segment, or a backslash, is never
/// looked up: the component name is joined onto a directory, and such a
/// name would name a file outside it.
pub(crate) fn ensure_page_exists(
    config: &InertiaConfig,
    component: &str,
) -> Result<(), FrameworkError> {
    let stays_inside = !component.contains('\\')
        && component
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
    let found = stays_inside
        && config.page_extensions.iter().any(|extension| {
            config
                .pages_dir
                .join(format!("{component}.{extension}"))
                .is_file()
        });
    if found {
        return Ok(());
    }
    Err(FrameworkError::internal(format!(
        "Inertia page component '{component}' not found: no file '{component}.{{{}}}' under \
         '{}' (InertiaConfig::ensure_pages_exist is on). Check the component name, or point \
         InertiaConfig::pages_dir at the directory the pages live in.",
        config.page_extensions.join(","),
        config.pages_dir.display()
    )))
}
