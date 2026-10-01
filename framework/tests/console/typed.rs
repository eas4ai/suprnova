//! Typed console command integration tests.
//!
//! Exercises the `#[derive(Command)]` + `TypedCommand` path
//! end-to-end: clap's `Parser` derive describes the args, our
//! derive macro wires the inventory + adapter, the trait impl
//! provides the body. Tests pin:
//!
//!   - typed args parsed by clap reach the handler as struct fields
//!   - `#[arg]` flags (short/long, default values) work as expected
//!   - missing required args yield a clap parse error → dispatch
//!     returns Err
//!   - `<command> --help` prints the per-command help block (clap
//!     auto-generates from the struct + attribute) and returns Ok
//!
//! Tests share three statics that record what the `typed:greet`
//! handler saw on its last run, so they must be `#[serial]`.

use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use clap::Parser;
use serial_test::serial;
use suprnova::{Command, FrameworkError, TypedCommand, console};

static GREET_RAN: AtomicUsize = AtomicUsize::new(0);
static LAST_GREET_TARGET: OnceLock<Mutex<String>> = OnceLock::new();
static LAST_GREET_LOUD: AtomicUsize = AtomicUsize::new(0); // 0 = unset, 1 = false, 2 = true

fn target_slot() -> &'static Mutex<String> {
    LAST_GREET_TARGET.get_or_init(|| Mutex::new(String::new()))
}

#[derive(Parser, Command, Debug)]
#[console(name = "typed:greet", description = "Greet someone (typed)")]
struct TypedGreet {
    #[arg(short, long, default_value = "world")]
    name: String,

    #[arg(long, default_value_t = false)]
    loud: bool,
}

#[async_trait]
impl TypedCommand for TypedGreet {
    async fn run(self) -> Result<(), FrameworkError> {
        GREET_RAN.fetch_add(1, Ordering::SeqCst);
        *target_slot().lock().unwrap() = self.name;
        LAST_GREET_LOUD.store(if self.loud { 2 } else { 1 }, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Parser, Command, Debug)]
#[console(name = "typed:require", description = "Requires a positional arg")]
struct TypedRequire {
    /// Required positional that has no default - missing it forces
    /// a clap parse error.
    #[arg(value_name = "TARGET")]
    target: String,
}

#[async_trait]
impl TypedCommand for TypedRequire {
    async fn run(self) -> Result<(), FrameworkError> {
        // Body unreachable for the missing-arg test; the parse step
        // fails before this runs.
        Ok(())
    }
}

/// Rebuild the search index
///
/// The second paragraph is for the command's own `--help` and is not a
/// part of the one line the list of commands shows.
#[derive(Parser, Command, Debug)]
#[console(name = "typed:documented")]
struct TypedDocumented {}

#[async_trait]
impl TypedCommand for TypedDocumented {
    async fn run(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[derive(Parser, Command, Debug)]
#[command(about = "Send the weekly digest")]
#[console(name = "typed:about-attribute")]
struct TypedAboutAttribute {}

#[async_trait]
impl TypedCommand for TypedAboutAttribute {
    async fn run(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// The doc comment says one thing
#[derive(Parser, Command, Debug)]
#[console(name = "typed:both", description = "The description says another")]
struct TypedBoth {}

#[async_trait]
impl TypedCommand for TypedBoth {
    async fn run(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// The list of commands the console's top-level help prints, for the
/// commands named. The console builds its root the same way: one
/// subcommand from each entry's `clap_builder`.
fn help_listing(names: &[&str]) -> String {
    let mut root = clap::Command::new("console");
    for name in names {
        let entry = console::find(name).unwrap_or_else(|| panic!("{name} is registered"));
        root = root.subcommand((entry.clap_builder)());
    }
    root.render_help().to_string()
}

/// The line of `listing` that is about the command `name`.
fn line_of<'a>(listing: &'a str, name: &str) -> &'a str {
    listing
        .lines()
        .find(|line| line.trim_start().starts_with(name))
        .unwrap_or_else(|| panic!("the help lists {name}:\n{listing}"))
}

#[test]
fn a_command_with_no_description_keeps_the_about_text_of_its_doc_comment() {
    let entry = console::find("typed:documented").expect("typed:documented is registered");

    assert_eq!(entry.description, "", "the attribute declared none");
    assert_eq!(entry.about().as_deref(), Some("Rebuild the search index"));

    let listing = help_listing(&["typed:documented"]);
    assert!(
        line_of(&listing, "typed:documented").ends_with("Rebuild the search index"),
        "the list of commands must show the doc line:\n{listing}"
    );
}

#[test]
fn a_command_with_no_description_keeps_an_explicit_clap_about() {
    let entry =
        console::find("typed:about-attribute").expect("typed:about-attribute is registered");

    assert_eq!(entry.about().as_deref(), Some("Send the weekly digest"));
    let listing = help_listing(&["typed:about-attribute"]);
    assert!(
        line_of(&listing, "typed:about-attribute").ends_with("Send the weekly digest"),
        "the list of commands must show the clap about text:\n{listing}"
    );
}

#[test]
fn a_description_overrides_the_doc_comment() {
    let entry = console::find("typed:both").expect("typed:both is registered");

    assert_eq!(entry.description, "The description says another");
    assert_eq!(
        entry.about().as_deref(),
        Some("The description says another")
    );
    let listing = help_listing(&["typed:both"]);
    assert!(
        line_of(&listing, "typed:both").ends_with("The description says another"),
        "the description must win over the doc comment:\n{listing}"
    );
}

#[tokio::test]
async fn typed_command_is_registered_via_derive() {
    let entry = console::find("typed:greet").expect("derive(Command) auto-registered typed:greet");
    assert_eq!(entry.name, "typed:greet");
    assert_eq!(entry.description, "Greet someone (typed)");
}

#[tokio::test]
#[serial]
async fn typed_command_parses_and_forwards_args() {
    GREET_RAN.store(0, Ordering::SeqCst);
    *target_slot().lock().unwrap() = String::new();
    LAST_GREET_LOUD.store(0, Ordering::SeqCst);

    let argv = vec![
        "console".to_string(),
        "typed:greet".to_string(),
        "--name".to_string(),
        "alice".to_string(),
        "--loud".to_string(),
    ];
    console::dispatch_argv(argv).await.expect("dispatch ok");

    assert_eq!(GREET_RAN.load(Ordering::SeqCst), 1);
    assert_eq!(*target_slot().lock().unwrap(), "alice");
    assert_eq!(
        LAST_GREET_LOUD.load(Ordering::SeqCst),
        2,
        "--loud flag parsed as true"
    );
}

#[tokio::test]
#[serial]
async fn typed_command_uses_clap_defaults_when_args_omitted() {
    GREET_RAN.store(0, Ordering::SeqCst);
    *target_slot().lock().unwrap() = String::new();
    LAST_GREET_LOUD.store(0, Ordering::SeqCst);

    let argv = vec!["console".to_string(), "typed:greet".to_string()];
    console::dispatch_argv(argv).await.expect("dispatch ok");

    assert_eq!(GREET_RAN.load(Ordering::SeqCst), 1);
    assert_eq!(*target_slot().lock().unwrap(), "world");
    assert_eq!(
        LAST_GREET_LOUD.load(Ordering::SeqCst),
        1,
        "--loud absent ⇒ default false"
    );
}

#[tokio::test]
#[serial]
async fn typed_command_uses_short_flag_alias() {
    GREET_RAN.store(0, Ordering::SeqCst);
    *target_slot().lock().unwrap() = String::new();

    let argv = vec![
        "console".to_string(),
        "typed:greet".to_string(),
        "-n".to_string(),
        "bob".to_string(),
    ];
    console::dispatch_argv(argv).await.expect("dispatch ok");

    assert_eq!(*target_slot().lock().unwrap(), "bob");
}

#[tokio::test]
async fn typed_command_missing_required_arg_returns_err() {
    let argv = vec!["console".to_string(), "typed:require".to_string()];
    let err = console::dispatch_argv(argv)
        .await
        .expect_err("missing required positional ⇒ clap parse error ⇒ Err");
    // Clap formatted the error to stderr inside dispatch (the user sees
    // it via the binary). The returned Err is silent so the binary's
    // main doesn't double-print - same contract that
    // `dispatch_returns_err_for_unknown_command` pins in tests/console.rs.
    assert!(
        err.is_silent(),
        "clap-reported missing-arg errors are silent"
    );
}

#[tokio::test]
async fn typed_command_help_flag_returns_ok() {
    // Per-subcommand --help should print and resolve cleanly.
    let argv = vec![
        "console".to_string(),
        "typed:greet".to_string(),
        "--help".to_string(),
    ];
    console::dispatch_argv(argv)
        .await
        .expect("'<typed-cmd> --help' prints help and returns Ok");
}
