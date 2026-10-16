//! The configuration rows of Laravel's infrastructure surface: an unset
//! `APP_ENV` is production, as Laravel's `config/app.php` reads
//! `env('APP_ENV', 'production')`, and `Config::register_default` and
//! `Config::merge` let a crate register defaults under the application's
//! values, as Laravel's `ServiceProvider::mergeConfigFrom` does.
//!
//! The config repository is process-wide and cannot unregister a type, so
//! each repository test uses a type no other test registers.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde_json::{Value, json};
use suprnova::config::{__reset_loaded_keys_for_tests, AppConfig, Environment, load_dotenv};
use suprnova::{Config, MergeConfig};

use crate::env_snapshot::{EnvSnapshot, set_env};

fn write_env_file(dir: &Path, name: &str, contents: &str) {
    std::fs::write(dir.join(name), contents).expect("write env file");
}

// ---- An unset APP_ENV is production --------------------------------------

#[test]
fn an_unset_app_env_detects_as_production() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(&["APP_ENV"]);
    set_env("APP_ENV", None);

    assert_eq!(Environment::detect(), Environment::Production);
    assert!(Environment::detect().is_production());
    if !Config::has::<AppConfig>() {
        assert_eq!(Config::environment(), Environment::Production);
        assert!(Config::is_production());
    }
}

#[test]
fn an_unset_app_env_and_app_debug_leave_debug_off() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(&["APP_ENV", "APP_DEBUG"]);
    set_env("APP_ENV", None);
    set_env("APP_DEBUG", None);

    let lenient = AppConfig::from_env();
    assert_eq!(lenient.environment, Environment::Production);
    assert!(!lenient.debug, "debug defaults to off in production");

    let strict = AppConfig::try_from_env().expect("strict read");
    assert_eq!(strict.environment, Environment::Production);
    assert!(!strict.debug);
}

#[test]
fn a_set_app_env_keeps_its_meaning() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(&["APP_ENV", "APP_DEBUG"]);
    set_env("APP_DEBUG", None);

    set_env("APP_ENV", Some("local"));
    assert_eq!(Environment::detect(), Environment::Local);
    assert!(AppConfig::from_env().debug, "local keeps debug on");

    set_env("APP_ENV", Some("testing"));
    assert_eq!(Environment::detect(), Environment::Testing);
}

#[test]
fn detect_explicit_answers_none_for_an_unset_app_env() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(&["APP_ENV"]);

    set_env("APP_ENV", None);
    assert_eq!(Environment::detect_explicit(), None);

    set_env("APP_ENV", Some("staging"));
    assert_eq!(Environment::detect_explicit(), Some(Environment::Staging));

    set_env("APP_ENV", Some("PROD"));
    assert_eq!(
        Environment::detect_explicit(),
        Some(Environment::Production)
    );

    set_env("APP_ENV", Some("QA"));
    assert_eq!(
        Environment::detect_explicit(),
        Some(Environment::Custom("QA".to_string()))
    );
}

#[test]
fn an_unset_app_env_loads_no_environment_file_but_keeps_env_local() {
    let _env = crate::env_lock::lock_env();
    __reset_loaded_keys_for_tests();
    let _snap = EnvSnapshot::capture(&[
        "APP_ENV",
        "INFRA_GAPS_BASE",
        "INFRA_GAPS_LOCAL",
        "INFRA_GAPS_PRODUCTION",
        "INFRA_GAPS_PRODUCTION_LOCAL",
    ]);
    set_env("APP_ENV", None);
    set_env("INFRA_GAPS_BASE", None);
    set_env("INFRA_GAPS_LOCAL", None);
    set_env("INFRA_GAPS_PRODUCTION", None);
    set_env("INFRA_GAPS_PRODUCTION_LOCAL", None);

    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    write_env_file(dir, ".env", "INFRA_GAPS_BASE=base\n");
    write_env_file(dir, ".env.local", "INFRA_GAPS_LOCAL=local\n");
    write_env_file(dir, ".env.production", "INFRA_GAPS_PRODUCTION=production\n");
    write_env_file(
        dir,
        ".env.production.local",
        "INFRA_GAPS_PRODUCTION_LOCAL=production-local\n",
    );

    let env = load_dotenv(dir).expect("load_dotenv");

    assert_eq!(env, Environment::Production);
    assert_eq!(std::env::var("INFRA_GAPS_BASE").as_deref(), Ok("base"));
    assert_eq!(
        std::env::var("INFRA_GAPS_LOCAL").as_deref(),
        Ok("local"),
        ".env.local keeps loading for an unset APP_ENV"
    );
    assert!(
        std::env::var("INFRA_GAPS_PRODUCTION").is_err(),
        ".env.production must not load when APP_ENV is unset"
    );
    assert!(
        std::env::var("INFRA_GAPS_PRODUCTION_LOCAL").is_err(),
        ".env.production.local must not load when APP_ENV is unset"
    );
}

#[test]
fn an_app_env_set_in_the_base_file_still_picks_its_environment_file() {
    let _env = crate::env_lock::lock_env();
    __reset_loaded_keys_for_tests();
    let _snap = EnvSnapshot::capture(&["APP_ENV", "INFRA_GAPS_STAGING"]);
    set_env("APP_ENV", None);
    set_env("INFRA_GAPS_STAGING", None);

    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    write_env_file(dir, ".env", "APP_ENV=staging\n");
    write_env_file(dir, ".env.staging", "INFRA_GAPS_STAGING=staging\n");

    let env = load_dotenv(dir).expect("load_dotenv");

    assert_eq!(env, Environment::Staging);
    assert_eq!(
        std::env::var("INFRA_GAPS_STAGING").as_deref(),
        Ok("staging")
    );
}

// ---- Config::register_default ---------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct RegisteredProbe {
    a: u32,
}

#[derive(Clone, Debug, PartialEq)]
struct AbsentProbe {
    a: u32,
}

#[derive(Clone, Debug, PartialEq)]
struct RacedProbe {
    writer: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct ReplacedProbe {
    a: u32,
}

#[test]
fn register_default_keeps_a_registered_value() {
    Config::register(RegisteredProbe { a: 1 });

    assert!(!Config::register_default(RegisteredProbe { a: 2 }));
    assert_eq!(
        Config::get::<RegisteredProbe>(),
        Some(RegisteredProbe { a: 1 })
    );
}

#[test]
fn register_default_registers_a_value_when_none_is_registered() {
    assert!(!Config::has::<AbsentProbe>());

    assert!(Config::register_default(AbsentProbe { a: 2 }));
    assert_eq!(Config::get::<AbsentProbe>(), Some(AbsentProbe { a: 2 }));

    assert!(
        !Config::register_default(AbsentProbe { a: 3 }),
        "the second default finds the first registered"
    );
    assert_eq!(Config::get::<AbsentProbe>(), Some(AbsentProbe { a: 2 }));
}

#[test]
fn register_default_registers_once_under_concurrent_callers() {
    let winners: usize = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..16)
            .map(|writer| scope.spawn(move || Config::register_default(RacedProbe { writer })))
            .collect();
        handles
            .into_iter()
            .map(|handle| usize::from(handle.join().expect("writer thread")))
            .sum()
    });

    assert_eq!(winners, 1, "exactly one caller registers the default");
    assert!(Config::has::<RacedProbe>());
}

#[test]
fn register_keeps_replacing_the_value() {
    Config::register(ReplacedProbe { a: 1 });
    Config::register(ReplacedProbe { a: 2 });

    assert_eq!(Config::get::<ReplacedProbe>(), Some(ReplacedProbe { a: 2 }));
}

// ---- Config::merge --------------------------------------------------------

#[test]
fn merge_keeps_the_registered_keys_of_a_json_map() {
    let mut registered = serde_json::Map::new();
    registered.insert("a".to_string(), json!(9));
    Config::register(registered);

    let mut defaults = serde_json::Map::new();
    defaults.insert("a".to_string(), json!(2));
    defaults.insert("b".to_string(), json!(3));
    Config::merge(defaults);

    let merged = Config::get::<serde_json::Map<String, Value>>().expect("registered map");
    assert_eq!(Value::Object(merged), json!({"a": 9, "b": 3}));
}

#[test]
fn merge_registers_the_defaults_when_none_is_registered() {
    assert!(!Config::has::<BTreeMap<String, u16>>());

    Config::merge(BTreeMap::from([
        ("a".to_string(), 2_u16),
        ("b".to_string(), 3_u16),
    ]));

    assert_eq!(
        Config::get::<BTreeMap<String, u16>>(),
        Some(BTreeMap::from([
            ("a".to_string(), 2_u16),
            ("b".to_string(), 3_u16),
        ]))
    );
}

#[test]
fn merge_keeps_the_registered_keys_of_a_hash_map() {
    Config::register(HashMap::from([("a".to_string(), 9_i64)]));

    Config::merge(HashMap::from([
        ("a".to_string(), 2_i64),
        ("b".to_string(), 3_i64),
    ]));

    assert_eq!(
        Config::get::<HashMap<String, i64>>(),
        Some(HashMap::from([
            ("a".to_string(), 9_i64),
            ("b".to_string(), 3_i64),
        ]))
    );
}

#[test]
fn merge_keeps_a_registered_key_whose_value_is_null() {
    // A key the application set, even to null, is the application's.
    let mut registered = BTreeMap::new();
    registered.insert("feature".to_string(), Value::Null);
    Config::register(registered);

    Config::merge(BTreeMap::from([
        ("feature".to_string(), json!(true)),
        ("other".to_string(), json!("x")),
    ]));

    assert_eq!(
        Config::get::<BTreeMap<String, Value>>(),
        Some(BTreeMap::from([
            ("feature".to_string(), Value::Null),
            ("other".to_string(), json!("x")),
        ]))
    );
}

/// A configuration struct that merges field by field, for an application
/// that keeps its options typed rather than in a map.
#[derive(Clone, Debug, PartialEq)]
struct MailerOptions {
    from: Option<String>,
    retries: Option<u8>,
}

impl MergeConfig for MailerOptions {
    fn merge_defaults(&mut self, defaults: Self) {
        self.from = self.from.take().or(defaults.from);
        self.retries = self.retries.or(defaults.retries);
    }
}

#[test]
fn merge_accepts_a_type_that_implements_merge_config() {
    Config::register(MailerOptions {
        from: Some("app@example.com".to_string()),
        retries: None,
    });

    Config::merge(MailerOptions {
        from: Some("crate@example.com".to_string()),
        retries: Some(3),
    });

    assert_eq!(
        Config::get::<MailerOptions>(),
        Some(MailerOptions {
            from: Some("app@example.com".to_string()),
            retries: Some(3),
        })
    );
}
