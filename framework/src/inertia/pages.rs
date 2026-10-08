//! The run-time page check `InertiaConfig::ensure_pages_exist` turns on -
//! Laravel's `ResponseFactory::findComponentOrFail`.
//!
//! `inertia_response!` checks its component when the crate compiles. A name
//! given as a string to `InertiaResponse::new` or `Router::inertia` is not
//! seen then, and a typo in it reaches the browser as a page the client
//! cannot resolve. With the check on, such a render is an error naming the
//! component and the directory, which the error page shows the developer.

use std::path::PathBuf;

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
    if find_page_file(config, component).is_some() {
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

/// The page file of `component` under the configured directory with the
/// first configured extension that has one, Laravel's view finder; `None`
/// when there is none, or the name would leave the directory. Inertia
/// DevTools records it as the entry's `componentPath`.
pub(crate) fn find_page_file(config: &InertiaConfig, component: &str) -> Option<PathBuf> {
    let stays_inside = !component.contains('\\')
        && component
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
    if !stays_inside {
        return None;
    }
    config
        .page_extensions
        .iter()
        .map(|extension| config.pages_dir.join(format!("{component}.{extension}")))
        .find(|path| path.is_file())
}
