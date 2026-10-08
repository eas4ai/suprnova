//! The violating example the three mechanisms are bound on for this round:
//! the devtools configuration PAR-071 names does not exist yet, so the
//! inertia binary does not compile and every check records a fail.
//! Removed once the mechanisms are bound.

use suprnova::DevToolsConfig;

#[tokio::test]
async fn indt_red_the_devtools_config_exists() {
    let config = DevToolsConfig::default();
    let _ = config;
}
