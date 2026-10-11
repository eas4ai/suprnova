//! The artisan generators that write one file from a stub (PAR-191):
//! `make:factory`, `make:seeder`, `make:cast`, `make:channel`,
//! `make:class`, `make:enum`, `make:event`, `make:listener`, `make:job`,
//! `make:job-middleware`, `make:mail`, `make:notification`,
//! `make:observer`, `make:policy`, `make:provider`, `make:resource`,
//! `make:rule`, `make:scope`, `make:test`, `make:trait` and
//! `make:interface`.
//!
//! Each test runs the built `suprnova` binary in a temporary directory laid
//! out as `suprnova new` lays out a project, as far as the generators read
//! it. The ignored test at the end scaffolds a real project, runs every
//! generator in it and compiles the result.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

/// What `suprnova new` writes to `src/lib.rs`.
const LIB_RS: &str = include_str!("../src/templates/files/backend/lib.rs.tpl");

/// Each generator, a name it takes, and the file that name writes.
const GENERATORS: &[(&str, &str, &str)] = &[
    ("make:factory", "User", "src/factories/user_factory.rs"),
    ("make:seeder", "Users", "src/seeders/users_seeder.rs"),
    ("make:cast", "AsMoney", "src/casts/as_money.rs"),
    (
        "make:channel",
        "OrderUpdates",
        "src/broadcasting/order_updates.rs",
    ),
    ("make:class", "Invoice", "src/invoice.rs"),
    ("make:enum", "Status", "src/enums/status.rs"),
    ("make:event", "OrderShipped", "src/events/order_shipped.rs"),
    (
        "make:listener",
        "AuditEvents",
        "src/listeners/audit_events.rs",
    ),
    ("make:job", "ProcessPodcast", "src/jobs/process_podcast.rs"),
    (
        "make:job-middleware",
        "RateLimitedByUser",
        "src/jobs/middleware/rate_limited_by_user.rs",
    ),
    ("make:mail", "Welcome", "src/mail/welcome.rs"),
    (
        "make:notification",
        "InvoicePaid",
        "src/notifications/invoice_paid.rs",
    ),
    ("make:observer", "User", "src/observers/user_observer.rs"),
    ("make:policy", "Note", "src/policies/note_policy.rs"),
    (
        "make:provider",
        "PaymentServiceProvider",
        "src/providers/payment_service_provider.rs",
    ),
    ("make:resource", "User", "src/resources/user_resource.rs"),
    ("make:rule", "Uppercase", "src/rules/uppercase.rs"),
    ("make:scope", "Active", "src/scopes/active_scope.rs"),
    ("make:test", "UserTest", "tests/user_test.rs"),
    ("make:trait", "HasUuid", "src/concerns/has_uuid.rs"),
    (
        "make:interface",
        "PaymentGateway",
        "src/contracts/payment_gateway.rs",
    ),
];

fn cli_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_suprnova"))
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(cli_binary())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the suprnova binary runs")
}

fn combined(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr)
}

fn succeeds(dir: &Path, args: &[&str]) -> String {
    let output = run(dir, args);
    let text = combined(&output);
    assert!(
        output.status.success(),
        "`suprnova {args:?}` failed:\n{text}"
    );
    text
}

fn read(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn write(path: impl AsRef<Path>, content: &str) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create the parent directory");
    }
    fs::write(path, content).expect("write the fixture");
}

/// A project root as `suprnova new` lays one out, as far as the generators
/// read it: the manifest, the crate root and the two models every new
/// project has.
fn project() -> TempDir {
    let tmp = TempDir::new().expect("a temporary directory");
    write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname = \"shop-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    write(tmp.path().join("src/lib.rs"), LIB_RS);
    write(
        tmp.path().join("src/models/mod.rs"),
        "//! Application models\n\npub mod note;\npub mod user;\n",
    );
    write(tmp.path().join("src/models/user.rs"), "pub struct User;\n");
    write(tmp.path().join("src/models/note.rs"), "pub struct Note;\n");
    tmp
}

/// Every file under `root`, relative to it, sorted.
fn files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).expect("read a directory") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path.strip_prefix(root).expect("under root").to_path_buf());
            }
        }
    }
    found.sort();
    found
}

/// Run `args` in a fresh project, then check that it printed the path of
/// `file`, that `file` holds each snippet of `shape`, and that each module
/// file of `declared` holds its declaration exactly once. Returns the
/// project and the file's content for further checks.
fn generates(
    args: &[&str],
    file: &str,
    shape: &[&str],
    declared: &[(&str, &str)],
) -> (TempDir, String) {
    let tmp = project();
    let output = succeeds(tmp.path(), args);
    assert!(
        output.contains(&format!("Created {file}")),
        "the run names the file it wrote:\n{output}"
    );
    let source = read(tmp.path().join(file));
    for snippet in shape {
        assert!(
            source.contains(snippet),
            "{file} lacks {snippet:?}:\n{source}"
        );
    }
    for (module_file, declaration) in declared {
        let module = read(tmp.path().join(module_file));
        assert_eq!(
            module.matches(declaration).count(),
            1,
            "{module_file} declares {declaration:?} once:\n{module}"
        );
    }
    (tmp, source)
}

// One test per generator: the file, its shape, and its declarations.

#[test]
fn make_factory_writes_a_factory_of_the_model() {
    generates(
        &["make:factory", "User"],
        "src/factories/user_factory.rs",
        &[
            "use suprnova::Factory;",
            "use crate::models::user::User;",
            "pub struct UserFactory;",
            "impl Factory for UserFactory {",
            "type Model = User;",
            "fn definition() -> User {",
            "..Default::default()",
        ],
        &[
            ("src/lib.rs", "pub mod factories;"),
            ("src/factories/mod.rs", "pub mod user_factory;"),
        ],
    );
}

#[test]
fn make_seeder_writes_a_seeder_db_seed_finds_by_name() {
    generates(
        &["make:seeder", "Users"],
        "src/seeders/users_seeder.rs",
        &[
            "pub struct UsersSeeder;",
            "impl Seeder for UsersSeeder {",
            "\"UsersSeeder\"",
            "async fn run() -> Result<(), FrameworkError>",
        ],
        &[
            ("src/lib.rs", "pub mod seeders;"),
            ("src/seeders/mod.rs", "pub mod users_seeder;"),
        ],
    );
}

#[test]
fn make_cast_writes_a_pass_through_cast() {
    generates(
        &["make:cast", "AsMoney"],
        "src/casts/as_money.rs",
        &[
            "use suprnova::{Cast, FrameworkError};",
            "pub struct AsMoney;",
            "impl Cast for AsMoney {",
            "type Runtime = String;",
            "type Storage = String;",
            "fn to_storage(value: &String) -> Result<String, FrameworkError>",
            "fn from_storage(stored: &String) -> Result<String, FrameworkError>",
        ],
        &[
            ("src/lib.rs", "pub mod casts;"),
            ("src/casts/mod.rs", "pub mod as_money;"),
        ],
    );
}

#[test]
fn make_channel_writes_a_private_channel_that_authorizes_the_session_user() {
    generates(
        &["make:channel", "OrderUpdates"],
        "src/broadcasting/order_updates.rs",
        &[
            "pub struct OrderUpdates;",
            "impl Channel for OrderUpdates {",
            "\"order.updates\"",
            "async fn authorize(",
            "req.auth_user_id().is_some()",
            "impl PrivateChannel for OrderUpdates {}",
        ],
        &[
            ("src/lib.rs", "pub mod broadcasting;"),
            ("src/broadcasting/mod.rs", "pub mod order_updates;"),
        ],
    );
}

#[test]
fn make_class_writes_a_struct_declared_in_the_crate_root() {
    generates(
        &["make:class", "Invoice"],
        "src/invoice.rs",
        &[
            "/// A plain Rust type: give `Invoice` the fields and methods it needs.",
            "pub struct Invoice;",
            "impl Invoice {",
            "pub fn new() -> Self",
        ],
        &[("src/lib.rs", "pub mod invoice;")],
    );
}

#[test]
fn make_enum_writes_an_enum() {
    generates(
        &["make:enum", "Status"],
        "src/enums/status.rs",
        &[
            "/// The values a `Status` can take",
            "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]",
            "pub enum Status {}",
        ],
        &[
            ("src/lib.rs", "pub mod enums;"),
            ("src/enums/mod.rs", "pub mod status;"),
        ],
    );
}

#[test]
fn make_event_writes_an_event() {
    generates(
        &["make:event", "OrderShipped"],
        "src/events/order_shipped.rs",
        &[
            "#[derive(Debug, Clone)]",
            "pub struct OrderShipped {}",
            "impl Event for OrderShipped {",
            "\"OrderShipped\"",
        ],
        &[
            ("src/lib.rs", "pub mod events;"),
            ("src/events/mod.rs", "pub mod order_shipped;"),
        ],
    );
}

#[test]
fn make_listener_writes_a_listener_of_any_event() {
    generates(
        &["make:listener", "AuditEvents"],
        "src/listeners/audit_events.rs",
        &[
            "pub struct AuditEvents;",
            "impl<E: Event> Listener<E> for AuditEvents {",
            "async fn handle(&self, _event: &E) -> Result<(), FrameworkError>",
        ],
        &[
            ("src/lib.rs", "pub mod listeners;"),
            ("src/listeners/mod.rs", "pub mod audit_events;"),
        ],
    );
}

#[test]
fn make_listener_with_an_event_writes_a_listener_of_that_event() {
    let tmp = project();
    succeeds(tmp.path(), &["make:event", "OrderShipped"]);
    let output = succeeds(
        tmp.path(),
        &[
            "make:listener",
            "SendShipmentNotification",
            "--event",
            "OrderShipped",
        ],
    );
    assert!(!output.contains("not found"), "{output}");
    let source = read(
        tmp.path()
            .join("src/listeners/send_shipment_notification.rs"),
    );
    for snippet in [
        "use crate::events::order_shipped::OrderShipped;",
        "impl Listener<OrderShipped> for SendShipmentNotification {",
        "async fn handle(&self, _event: &OrderShipped)",
    ] {
        assert!(source.contains(snippet), "lacks {snippet:?}:\n{source}");
    }
}

#[test]
fn make_job_writes_a_queued_job() {
    generates(
        &["make:job", "ProcessPodcast"],
        "src/jobs/process_podcast.rs",
        &[
            "#[derive(Debug, Clone, Serialize, Deserialize)]",
            "pub struct ProcessPodcast {}",
            "impl Job for ProcessPodcast {",
            "\"ProcessPodcast\"",
            "async fn handle(self) -> Result<(), FrameworkError>",
        ],
        &[
            ("src/lib.rs", "pub mod jobs;"),
            ("src/jobs/mod.rs", "pub mod process_podcast;"),
        ],
    );
}

#[test]
fn make_job_middleware_writes_under_jobs_and_declares_both_modules() {
    generates(
        &["make:job-middleware", "RateLimitedByUser"],
        "src/jobs/middleware/rate_limited_by_user.rs",
        &[
            "pub struct RateLimitedByUser;",
            "impl JobMiddleware for RateLimitedByUser {",
            "next: JobMiddlewareNext,",
            "next(envelope).await",
        ],
        &[
            ("src/lib.rs", "pub mod jobs;"),
            ("src/jobs/mod.rs", "pub mod middleware;"),
            (
                "src/jobs/middleware/mod.rs",
                "pub mod rate_limited_by_user;",
            ),
        ],
    );
}

#[test]
fn make_mail_writes_a_mailable_with_a_subject_and_bodies() {
    generates(
        &["make:mail", "OrderShipped"],
        "src/mail/order_shipped.rs",
        &[
            "pub struct OrderShipped {}",
            "impl Mailable for OrderShipped {",
            "\"Order Shipped\".to_string()",
            "fn html_template_source(&self) -> Option<String>",
            "fn text_template_source(&self) -> Option<String>",
        ],
        &[
            ("src/lib.rs", "pub mod mail;"),
            ("src/mail/mod.rs", "pub mod order_shipped;"),
        ],
    );
}

#[test]
fn make_notification_writes_a_mail_notification() {
    generates(
        &["make:notification", "InvoicePaid"],
        "src/notifications/invoice_paid.rs",
        &[
            "#[derive(Debug, Clone, Serialize, Deserialize, NotificationMailable)]",
            "subject = \"Invoice Paid\"",
            "pub struct InvoicePaid {}",
            "impl Notification for InvoicePaid {",
            "vec![\"mail\"]",
        ],
        &[
            ("src/lib.rs", "pub mod notifications;"),
            ("src/notifications/mod.rs", "pub mod invoice_paid;"),
        ],
    );
}

#[test]
fn make_observer_writes_an_observer_of_the_model() {
    generates(
        &["make:observer", "User"],
        "src/observers/user_observer.rs",
        &[
            "use crate::models::user::User;",
            "pub struct UserObserver;",
            "#[suprnova::observer(crate::models::user::User)]\n#[async_trait]\nimpl Observer<User> for UserObserver {",
            "async fn created(&self, _user: &User)",
            "async fn force_deleted(&self, _user: &User)",
        ],
        &[
            ("src/lib.rs", "pub mod observers;"),
            ("src/observers/mod.rs", "pub mod user_observer;"),
        ],
    );
}

#[test]
fn make_policy_writes_a_policy_that_denies_until_written() {
    let (_tmp, source) = generates(
        &["make:policy", "Note"],
        "src/policies/note_policy.rs",
        &[
            "use crate::models::user::User;",
            "use crate::models::note::Note;",
            "#[policy(User, Note)]",
            "impl NotePolicy {",
            "fn view_any(_user: &User, _note: &Note) -> bool",
            "fn force_delete(_user: &User, _note: &Note) -> bool",
        ],
        &[
            ("src/lib.rs", "pub mod policies;"),
            ("src/policies/mod.rs", "pub mod note_policy;"),
        ],
    );
    assert_eq!(source.matches("false").count(), 7, "{source}");
}

#[test]
fn make_provider_writes_the_register_function_the_bootstrap_calls() {
    generates(
        &["make:provider", "PaymentServiceProvider"],
        "src/providers/payment_service_provider.rs",
        &[
            "pub async fn register() {",
            "crate::providers::payment_service_provider::register().await;",
        ],
        &[
            ("src/lib.rs", "pub mod providers;"),
            ("src/providers/mod.rs", "pub mod payment_service_provider;"),
        ],
    );
}

#[test]
fn make_resource_writes_a_json_api_data_struct() {
    generates(
        &["make:resource", "User"],
        "src/resources/user_resource.rs",
        &[
            "#[derive(Debug, Clone, Data, Validate)]",
            "#[json_resource(\"users\")]",
            "pub struct UserResource {",
            "pub id: u64,",
        ],
        &[
            ("src/lib.rs", "pub mod resources;"),
            ("src/resources/mod.rs", "pub mod user_resource;"),
        ],
    );
}

#[test]
fn make_rule_writes_a_validation_rule() {
    generates(
        &["make:rule", "Uppercase"],
        "src/rules/uppercase.rs",
        &[
            "use suprnova::{Rule, ValidationMessage};",
            "pub struct Uppercase;",
            "impl Rule for Uppercase {",
            "fn passes(&self, _value: &str) -> Result<(), ValidationMessage>",
        ],
        &[
            ("src/lib.rs", "pub mod rules;"),
            ("src/rules/mod.rs", "pub mod uppercase;"),
        ],
    );
}

#[test]
fn make_scope_writes_a_global_scope_of_user_unless_told_otherwise() {
    generates(
        &["make:scope", "Active"],
        "src/scopes/active_scope.rs",
        &[
            "use crate::models::user::User;",
            "pub struct ActiveScope;",
            "impl GlobalScope<User> for ActiveScope {",
            "fn apply(&self, query: Builder<User>) -> Builder<User>",
        ],
        &[
            ("src/lib.rs", "pub mod scopes;"),
            ("src/scopes/mod.rs", "pub mod active_scope;"),
        ],
    );
    generates(
        &["make:scope", "Published", "--model", "Note"],
        "src/scopes/published_scope.rs",
        &[
            "use crate::models::note::Note;",
            "impl GlobalScope<Note> for PublishedScope {",
        ],
        &[("src/scopes/mod.rs", "pub mod published_scope;")],
    );
}

#[test]
fn make_test_writes_an_integration_test_and_declares_nothing() {
    let (tmp, _) = generates(
        &["make:test", "UserTest"],
        "tests/user_test.rs",
        &[
            "use suprnova::testing::TestClient;",
            "#[tokio::test]",
            "async fn example()",
            ".assert_ok()",
        ],
        &[],
    );
    assert_eq!(
        read(tmp.path().join("src/lib.rs")),
        LIB_RS,
        "make:test declares no module"
    );
    assert!(!tmp.path().join("tests/mod.rs").exists());
}

#[test]
fn make_trait_writes_a_trait_with_a_default_method() {
    generates(
        &["make:trait", "HasUuid"],
        "src/concerns/has_uuid.rs",
        &[
            "pub trait HasUuid {",
            "fn type_name(&self) -> &'static str {",
        ],
        &[
            ("src/lib.rs", "pub mod concerns;"),
            ("src/concerns/mod.rs", "pub mod has_uuid;"),
        ],
    );
}

#[test]
fn make_interface_writes_a_trait_with_required_methods_only() {
    let (_tmp, source) = generates(
        &["make:interface", "PaymentGateway"],
        "src/contracts/payment_gateway.rs",
        &[
            "pub trait PaymentGateway: Send + Sync {",
            "fn name(&self) -> &str;",
        ],
        &[
            ("src/lib.rs", "pub mod contracts;"),
            ("src/contracts/mod.rs", "pub mod payment_gateway;"),
        ],
    );
    assert!(!source.contains("&str {"), "no default body:\n{source}");
}

// What every generator shares.

#[test]
fn every_generator_refuses_a_name_that_is_not_an_identifier() {
    for (command, _, _) in GENERATORS {
        for bad in ["1st", "order-shipped", "Admin/Users", "Type"] {
            let tmp = project();
            let before = files(tmp.path());
            let output = run(tmp.path(), &[command, bad]);
            let text = combined(&output);
            assert_eq!(
                output.status.code(),
                Some(1),
                "`{command} {bad}` must exit 1:\n{text}"
            );
            assert!(
                text.contains(&format!("'{bad}' is not a valid")),
                "`{command} {bad}` names the problem:\n{text}"
            );
            assert_eq!(files(tmp.path()), before, "`{command} {bad}` wrote nothing");
        }
    }
}

#[test]
fn every_generator_keeps_an_existing_file_unless_forced() {
    for (command, name, file) in GENERATORS {
        let tmp = project();
        succeeds(tmp.path(), &[command, name]);
        let path = tmp.path().join(file);
        let generated = read(&path);

        let edited = "// edited by hand\n";
        fs::write(&path, edited).expect("edit the generated file");
        let output = succeeds(tmp.path(), &[command, name]);
        assert!(
            output.contains(&format!("{file} already exists; nothing was written")),
            "`{command} {name}` says it kept the file:\n{output}"
        );
        assert_eq!(
            read(&path),
            edited,
            "`{command} {name}` left the file alone"
        );

        let output = succeeds(tmp.path(), &[command, name, "--force"]);
        assert!(
            output.contains(&format!("Updated {file}")),
            "`{command} {name} --force` names the file:\n{output}"
        );
        assert_eq!(read(&path), generated, "`--force` writes the stub again");
    }
}

#[test]
fn a_second_file_of_a_kind_joins_the_module_file() {
    let tmp = project();
    succeeds(tmp.path(), &["make:event", "OrderShipped"]);
    let output = succeeds(tmp.path(), &["make:event", "OrderCancelled"]);
    assert!(output.contains("Updated src/events/mod.rs"), "{output}");
    assert!(
        !output.contains("src/lib.rs"),
        "lib.rs already declares events:\n{output}"
    );

    let module = read(tmp.path().join("src/events/mod.rs"));
    assert_eq!(
        module,
        "//! Application events.\n\npub mod order_shipped;\npub mod order_cancelled;\n"
    );
    let lib = read(tmp.path().join("src/lib.rs"));
    assert_eq!(lib.matches("pub mod events;").count(), 1, "{lib}");

    // The job middleware goes into the `jobs` module `make:job` started.
    succeeds(tmp.path(), &["make:job", "ProcessPodcast"]);
    succeeds(tmp.path(), &["make:job-middleware", "RateLimitedByUser"]);
    assert_eq!(
        read(tmp.path().join("src/jobs/mod.rs")),
        "//! Queued jobs.\n\npub mod process_podcast;\npub mod middleware;\n"
    );
}

#[test]
fn a_directory_module_of_the_same_name_is_refused() {
    let tmp = project();
    write(
        tmp.path().join("src/events/order_shipped/mod.rs"),
        "pub mod details;\n",
    );
    let before = files(tmp.path());
    let output = run(tmp.path(), &["make:event", "OrderShipped", "--force"]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(
        text.contains("src/events/order_shipped/mod.rs already holds"),
        "{text}"
    );
    assert_eq!(files(tmp.path()), before);
}

#[test]
fn make_class_refuses_the_crate_root_files() {
    for name in ["Lib", "main"] {
        let tmp = project();
        let output = run(tmp.path(), &["make:class", name, "--force"]);
        assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
        assert_eq!(read(tmp.path().join("src/lib.rs")), LIB_RS);
    }
}

#[test]
fn a_name_that_clashes_with_the_stub_imports_is_refused() {
    let tmp = project();
    let output = run(tmp.path(), &["make:rule", "Rule"]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("imports a `Rule` of its own"), "{text}");
    assert!(!tmp.path().join("src/rules").exists());
}

#[test]
fn a_model_the_project_lacks_is_named_in_a_warning() {
    let tmp = project();
    let output = succeeds(tmp.path(), &["make:factory", "Post"]);
    assert!(
        output.contains("src/models/post.rs not found"),
        "the run says the model is missing:\n{output}"
    );
    let source = read(tmp.path().join("src/factories/post_factory.rs"));
    assert!(
        source.contains("use crate::models::post::Post;"),
        "{source}"
    );

    let output = succeeds(tmp.path(), &["make:observer", "Audit", "--model", "Note"]);
    assert!(!output.contains("not found"), "{output}");
    let source = read(tmp.path().join("src/observers/audit_observer.rs"));
    assert!(
        source.contains("impl Observer<Note> for AuditObserver"),
        "{source}"
    );
}

#[test]
fn a_policy_of_the_user_model_takes_the_user_once() {
    let tmp = project();
    succeeds(tmp.path(), &["make:policy", "User"]);
    let source = read(tmp.path().join("src/policies/user_policy.rs"));
    assert_eq!(
        source.matches("use crate::models::user::User;").count(),
        1,
        "{source}"
    );
    assert!(source.contains("#[policy(User, User)]"), "{source}");
    assert!(
        source.contains("fn view(_user: &User, _model: &User) -> bool"),
        "{source}"
    );
}

#[test]
fn a_project_without_a_crate_root_is_told_what_to_declare() {
    let tmp = TempDir::new().expect("a temporary directory");
    let output = succeeds(tmp.path(), &["make:enum", "Status"]);
    assert!(
        output.contains("src/lib.rs not found: declare `pub mod enums;`"),
        "{output}"
    );
    assert!(tmp.path().join("src/enums/status.rs").is_file());
    assert!(!tmp.path().join("src/lib.rs").exists());
}

// ---------------------------------------------------------------------------
// Compile check - slow (a full build of the framework); ignored by default.
// ---------------------------------------------------------------------------

/// Substrings no generated file may hold: every past occurrence in a
/// scaffold was a punted implementation. The same list as
/// `scaffold_snapshot.rs`'s.
const FORBIDDEN_MARKERS: &[&str] = &["TODO", "FIXME", "unimplemented!", "panic!("];

/// Rewrite the scaffolded `Cargo.toml` to build against the in-tree
/// `framework/` crate, exactly as `scaffold_snapshot.rs` does: every
/// `suprnova = ...` line becomes a path dependency, and an empty
/// `[workspace]` keeps the project out of this repository's workspace.
fn patch_local_suprnova(project: &Path) {
    let framework_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("suprnova-cli must have a workspace parent")
        .join("framework");
    let cargo_toml = project.join("Cargo.toml");
    let original = read(&cargo_toml);

    let mut rewritten = String::with_capacity(original.len() + 14);
    let mut replaced = false;
    let mut has_workspace = false;
    for line in original.lines() {
        if line.trim() == "[workspace]" {
            has_workspace = true;
        }
        if line.trim_start().starts_with("suprnova = ") {
            rewritten.push_str(&format!(
                "suprnova = {{ path = \"{}\" }}\n",
                framework_dir.display(),
            ));
            replaced = true;
        } else {
            rewritten.push_str(line);
            rewritten.push('\n');
        }
    }
    assert!(
        replaced,
        "scaffolded Cargo.toml must contain a `suprnova = ...` dependency line"
    );
    if !has_workspace {
        rewritten.push_str("\n[workspace]\n");
    }
    fs::write(&cargo_toml, rewritten).expect("write patched Cargo.toml");
}

/// Every forbidden marker in the `.rs` files under `dir`.
fn marker_hits(dir: &Path) -> Vec<String> {
    let mut hits = Vec::new();
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let content = read(path);
        for (index, line) in content.lines().enumerate() {
            for marker in FORBIDDEN_MARKERS {
                if line.contains(marker) {
                    hits.push(format!("{}:{}: {}", path.display(), index + 1, line.trim()));
                }
            }
        }
    }
    hits
}

#[test]
#[ignore = "compile check - scaffolds a project, runs every generator in it and runs `cargo check`; slow"]
fn generators_compile_in_a_fresh_scaffold() {
    let tmp = TempDir::new().expect("a temporary directory");
    let output = Command::new(cli_binary())
        .args([
            "new",
            "generated_app",
            "--no-interaction",
            "--no-git",
            "--frontend",
            "svelte",
        ])
        .current_dir(tmp.path())
        .output()
        .expect("`suprnova new` runs");
    assert!(
        output.status.success(),
        "`suprnova new` failed:\n{}",
        combined(&output)
    );
    let project = tmp.path().join("generated_app");

    // Every generator once, plus the forms that render a different stub:
    // a listener of one event and a policy of the user model.
    let mut runs: Vec<Vec<&str>> = GENERATORS
        .iter()
        .map(|(command, name, _)| vec![*command, *name])
        .collect();
    runs.push(vec![
        "make:listener",
        "SendShipmentNotification",
        "--event",
        "OrderShipped",
    ]);
    runs.push(vec!["make:policy", "User"]);
    for args in &runs {
        succeeds(&project, args);
    }
    for (_, _, file) in GENERATORS {
        assert!(project.join(file).is_file(), "{file} was written");
    }

    let hits: Vec<String> = ["src", "tests"]
        .iter()
        .flat_map(|dir| marker_hits(&project.join(dir)))
        .collect();
    assert!(
        hits.is_empty(),
        "generated files hold stub markers:\n{}",
        hits.join("\n")
    );

    patch_local_suprnova(&project);
    // `--all-targets` compiles `tests/` too, where `make:test` writes.
    let check = Command::new(env!("CARGO"))
        .args(["check", "--all-targets"])
        .current_dir(&project)
        .output()
        .expect("cargo check runs");
    assert!(
        check.status.success(),
        "the generated project failed cargo check:\n{}",
        combined(&check)
    );
}
