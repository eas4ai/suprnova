//! The welcome mail demo exists only where a delivering transport cannot be
//! abused: local, development and testing environments. Anywhere else,
//! custom labels included, the route answers 404 before any mail is queued.

use app::controllers::welcome::welcome_route_available;
use suprnova::config::Environment;

#[test]
fn welcome_mail_route_exists_only_for_local_work_and_tests() {
    for environment in [
        Environment::Local,
        Environment::Development,
        Environment::Testing,
    ] {
        assert!(welcome_route_available(&environment), "{environment:?}");
    }
}

#[test]
fn welcome_mail_route_does_not_exist_in_production_staging_or_custom_environments() {
    for environment in [
        Environment::Production,
        Environment::Staging,
        Environment::Custom("qa".to_owned()),
    ] {
        assert!(!welcome_route_available(&environment), "{environment:?}");
    }
}
