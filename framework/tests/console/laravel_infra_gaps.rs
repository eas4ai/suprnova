//! The console surface of the Laravel infrastructure gaps: marked lines and
//! verbosity flags (PAR-140), prompts (PAR-141), and `down` (PAR-142).
//!
//! The commands are this file's own. `harness:report` from `harness.rs` is
//! registered in this binary too, and the verbosity tests run it as it is.

use std::path::Path;
use std::process::{Command as Process, Output, Stdio};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use clap::Parser;
use suprnova::console::{self, FormAnswers, Progress, Verbosity};
use suprnova::{Command, FrameworkError, TypedCommand, command};

// ---------------------------------------------------------------------------
// PAR-140: marked lines and the verbosity flags
// ---------------------------------------------------------------------------

#[derive(Parser, Command, Debug)]
#[console(name = "infra:verbosity", description = "Writes at every level")]
struct WritesAtEveryLevel {}

#[async_trait]
impl TypedCommand for WritesAtEveryLevel {
    async fn run(self) -> Result<(), FrameworkError> {
        console::line(format!("level {:?}", console::verbosity()));
        console::error_at("detail", Verbosity::Verbose);
        console::line_at("more detail", Verbosity::VeryVerbose);
        console::line_at("debug detail", Verbosity::Debug);
        Ok(())
    }
}

#[derive(Parser, Command, Debug)]
#[console(name = "infra:marks", description = "Writes one line of each mark")]
struct WritesMarks {}

#[async_trait]
impl TypedCommand for WritesMarks {
    async fn run(self) -> Result<(), FrameworkError> {
        console::error("boom");
        console::warn("careful");
        console::info("done");
        Ok(())
    }
}

#[derive(Parser, Command, Debug)]
#[console(name = "infra:line", description = "Writes one plain line")]
struct WritesOneLine {}

#[async_trait]
impl TypedCommand for WritesOneLine {
    async fn run(self) -> Result<(), FrameworkError> {
        console::line("x");
        Ok(())
    }
}

/// A command with a `-v` of its own: the console must not take it.
#[derive(Parser, Command, Debug)]
#[console(name = "infra:own-flag", description = "Declares its own -v")]
struct DeclaresItsOwnVerbose {
    /// Show the version of the importer.
    #[arg(short, long)]
    verbose: bool,
}

#[async_trait]
impl TypedCommand for DeclaresItsOwnVerbose {
    async fn run(self) -> Result<(), FrameworkError> {
        console::line(format!(
            "own {} level {:?}",
            self.verbose,
            console::verbosity()
        ));
        Ok(())
    }
}

#[command(name = "infra:raw", description = "Takes raw arguments")]
async fn takes_raw_arguments(args: Vec<String>) -> Result<(), FrameworkError> {
    console::line(format!("args {args:?} level {:?}", console::verbosity()));
    Ok(())
}

#[tokio::test]
async fn quiet_before_the_command_name_silences_every_line() {
    let run = console::test(["-q", "harness:report"]).run().await;

    run.assert_successful();
    assert_eq!(run.output(), "");
    assert_eq!(run.errors(), "");
}

#[tokio::test]
async fn quiet_after_the_command_name_silences_every_line() {
    for flag in ["-q", "--quiet"] {
        let run = console::test(["harness:report", flag]).run().await;

        run.assert_successful();
        assert_eq!(run.output(), "", "{flag}");
        assert_eq!(run.errors(), "", "{flag}");
    }
}

#[tokio::test]
async fn quiet_keeps_the_error_of_a_failed_command() {
    let run = console::test(["-q", "harness:report", "--fail"])
        .run()
        .await;

    run.assert_failed();
    assert_eq!(run.output(), "");
    assert_eq!(
        run.errors(),
        "error: the report could not be written\n",
        "the dispatcher's own failure message is written past the flag"
    );
}

#[tokio::test]
async fn quiet_silences_the_marked_lines() {
    let run = console::test(["infra:marks", "-q"]).run().await;

    run.assert_successful();
    assert_eq!(run.output(), "");
    assert_eq!(run.errors(), "");
}

#[tokio::test]
async fn a_line_at_verbose_is_written_only_with_v() {
    let normal = console::test(["infra:verbosity"]).run().await;
    normal.assert_successful();
    assert_eq!(normal.output(), "level Normal\n");
    assert_eq!(normal.errors(), "", "no `detail` without -v");

    for argv in [
        ["-v", "infra:verbosity"],
        ["infra:verbosity", "-v"],
        ["infra:verbosity", "--verbose"],
    ] {
        let verbose = console::test(argv).run().await;
        verbose.assert_successful();
        assert_eq!(verbose.output(), "level Verbose\n", "{argv:?}");
        assert_eq!(verbose.errors(), "detail\n", "{argv:?}");
    }
}

#[tokio::test]
async fn each_v_raises_the_level_by_one() {
    let very = console::test(["infra:verbosity", "-vv"]).run().await;
    very.assert_successful();
    assert_eq!(very.output(), "level VeryVerbose\nmore detail\n");
    assert_eq!(very.errors(), "detail\n");

    let debug = console::test(["-v", "infra:verbosity", "-vv"]).run().await;
    debug.assert_successful();
    assert_eq!(
        debug.output(),
        "level Debug\nmore detail\ndebug detail\n",
        "the counts before and after the name add up"
    );

    let quiet = console::test(["-vvv", "infra:verbosity", "-q"]).run().await;
    quiet.assert_successful();
    assert_eq!(quiet.output(), "", "quiet wins over -v");
    assert_eq!(quiet.errors(), "");
}

#[tokio::test]
async fn the_marked_lines_are_plain_text_on_their_streams() {
    let run = console::test(["infra:marks"]).run().await;

    run.assert_successful();
    assert_eq!(run.errors(), "ERROR boom\nWARN careful\n");
    assert_eq!(run.output(), "INFO done\n");
    assert!(
        !run.errors().contains('\u{1b}') && !run.output().contains('\u{1b}'),
        "captured output holds no escape sequence"
    );
}

#[tokio::test]
async fn a_plain_line_is_its_text_and_a_line_ending() {
    let run = console::test(["infra:line"]).run().await;

    run.assert_successful();
    assert_eq!(run.output(), "x\n");
    assert_eq!(run.errors(), "");
}

#[tokio::test]
async fn a_command_keeps_its_own_v() {
    let own = console::test(["infra:own-flag", "-v"]).run().await;
    own.assert_successful();
    assert_eq!(own.output(), "own true level Normal\n");

    let console_flag = console::test(["-v", "infra:own-flag"]).run().await;
    console_flag.assert_successful();
    assert_eq!(console_flag.output(), "own false level Verbose\n");

    let quiet = console::test(["infra:own-flag", "-q"]).run().await;
    quiet.assert_successful();
    assert_eq!(
        quiet.output(),
        "",
        "the -q it does not declare still applies"
    );
}

#[tokio::test]
async fn a_raw_command_gets_its_arguments_as_typed() {
    let after = console::test(["infra:raw", "-v", "--quiet"]).run().await;
    after.assert_successful();
    assert_eq!(
        after.output(),
        "args [\"-v\", \"--quiet\"] level Normal\n",
        "after the name, the flags are the command's arguments"
    );

    let before = console::test(["-v", "infra:raw", "a"]).run().await;
    before.assert_successful();
    assert_eq!(before.output(), "args [\"a\"] level Verbose\n");
}

// ---------------------------------------------------------------------------
// PAR-141: prompts
// ---------------------------------------------------------------------------

/// What `prompt:demo` received, for the test to read without the command
/// printing it.
static SECRET_SEEN: Mutex<Option<String>> = Mutex::new(None);

#[derive(Parser, Command, Debug)]
#[console(name = "prompt:demo", description = "Asks for a password")]
struct AsksForAPassword {}

#[async_trait]
impl TypedCommand for AsksForAPassword {
    async fn run(self) -> Result<(), FrameworkError> {
        let password = console::secret("Password?")?;
        *SECRET_SEEN.lock().unwrap_or_else(|p| p.into_inner()) = Some(password);
        console::line("stored");
        Ok(())
    }
}

#[derive(Parser, Command, Debug)]
#[console(name = "prompt:select", description = "Asks for one role")]
struct AsksForOneRole {}

#[async_trait]
impl TypedCommand for AsksForOneRole {
    async fn run(self) -> Result<(), FrameworkError> {
        let index = console::select("Role?", &["Member", "Owner"], None)?;
        console::line(format!("index {index}"));
        Ok(())
    }
}

#[derive(Parser, Command, Debug)]
#[console(
    name = "prompt:select-default",
    description = "Asks for a role with a default"
)]
struct AsksForARoleWithADefault {}

#[async_trait]
impl TypedCommand for AsksForARoleWithADefault {
    async fn run(self) -> Result<(), FrameworkError> {
        let index = console::select("Role?", &["Member", "Owner"], Some(1))?;
        console::line(format!("index {index}"));
        Ok(())
    }
}

#[derive(Parser, Command, Debug)]
#[console(name = "prompt:keyed", description = "Asks for a plan by key")]
struct AsksForAPlan {}

#[async_trait]
impl TypedCommand for AsksForAPlan {
    async fn run(self) -> Result<(), FrameworkError> {
        let plan = console::select_keyed(
            "Plan?",
            &[("basic", "Basic"), ("pro", "Professional")],
            Some("basic"),
        )?;
        console::line(format!("plan {plan}"));
        Ok(())
    }
}

#[derive(Parser, Command, Debug)]
#[console(name = "prompt:multi", description = "Asks for several roles")]
struct AsksForSeveralRoles {}

#[async_trait]
impl TypedCommand for AsksForSeveralRoles {
    async fn run(self) -> Result<(), FrameworkError> {
        let indexes = console::multiselect("Roles?", &["Member", "Owner"], &[1])?;
        console::line(format!("indexes {indexes:?}"));
        Ok(())
    }
}

#[derive(Parser, Command, Debug)]
#[console(name = "prompt:default", description = "Asks with a default")]
struct AsksWithADefault {}

#[async_trait]
impl TypedCommand for AsksWithADefault {
    async fn run(self) -> Result<(), FrameworkError> {
        let name = console::ask_with_default("Name?", "Ada")?;
        console::line(format!("name {name}"));
        Ok(())
    }
}

#[derive(Parser, Command, Debug)]
#[console(
    name = "prompt:progress",
    description = "Maps three items behind a bar"
)]
struct MapsBehindABar {}

#[async_trait]
impl TypedCommand for MapsBehindABar {
    async fn run(self) -> Result<(), FrameworkError> {
        let mut calls = 0;
        let doubled = console::progress("Sync", 0..3, |n, _bar| {
            calls += 1;
            n * 2
        });
        console::line(format!("calls {calls} results {doubled:?}"));
        Ok(())
    }
}

#[derive(Parser, Command, Debug)]
#[console(name = "prompt:bar", description = "Drives a bar by hand")]
struct DrivesABar {}

#[async_trait]
impl TypedCommand for DrivesABar {
    async fn run(self) -> Result<(), FrameworkError> {
        let mut bar = Progress::new("Upload", 4);
        bar.advance(2);
        bar.label("Uploading").hint("b.txt");
        bar.advance(5);
        console::line(format!("done {} of {}", bar.done(), bar.total()));
        bar.finish();
        Ok(())
    }
}

/// What `prompt:form` received.
static FORM_SEEN: Mutex<Option<FormAnswers>> = Mutex::new(None);

#[derive(Parser, Command, Debug)]
#[console(name = "prompt:form", description = "Asks a form")]
struct AsksAForm {}

#[async_trait]
impl TypedCommand for AsksAForm {
    async fn run(self) -> Result<(), FrameworkError> {
        let answers = console::form()
            .text("name", "Name?")
            .secret("token", "Token?")
            .confirm("admin", "Admin?", false)
            .select("plan", "Plan?", &["Basic", "Professional"], None)
            .submit()?;
        *FORM_SEEN.lock().unwrap_or_else(|p| p.into_inner()) = Some(answers);
        console::line("submitted");
        Ok(())
    }
}

#[derive(Parser, Command, Debug)]
#[console(name = "prompt:form-twice", description = "Names two prompts alike")]
struct NamesTwoPromptsAlike {}

#[async_trait]
impl TypedCommand for NamesTwoPromptsAlike {
    async fn run(self) -> Result<(), FrameworkError> {
        console::form()
            .text("name", "First name?")
            .text("name", "Last name?")
            .submit()?;
        Ok(())
    }
}

#[tokio::test]
async fn a_secret_is_received_and_never_captured() {
    let run = console::test(["prompt:demo"])
        .expects_question("Password?", "hunter2")
        .run()
        .await;

    run.assert_successful().assert_every_question_was_asked();
    assert_eq!(
        SECRET_SEEN
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_deref(),
        Some("hunter2")
    );
    assert_eq!(run.output(), "Password?\nstored\n");
    assert!(!run.output().contains("hunter2") && !run.errors().contains("hunter2"));
}

#[tokio::test]
async fn select_answers_the_index_of_the_chosen_option() {
    let run = console::test(["prompt:select"])
        .expects_choice("Role?", "Owner", ["Member", "Owner"])
        .run()
        .await;

    run.assert_successful().assert_output_contains("index 1");

    let by_question = console::test(["prompt:select"])
        .expects_question("Role?", "Member")
        .run()
        .await;
    by_question
        .assert_successful()
        .assert_output_contains("index 0");

    let by_number = console::test(["prompt:select"])
        .expects_question("Role?", "1")
        .run()
        .await;
    by_number
        .assert_successful()
        .assert_output_contains("index 1");
}

#[tokio::test]
async fn select_refuses_an_answer_that_is_no_option_and_names_the_options() {
    let run = console::test(["prompt:select"])
        .expects_choice("Role?", "Admin", ["Member", "Owner"])
        .run()
        .await;

    run.assert_failed()
        .assert_errors_contain("Admin")
        .assert_errors_contain("`Member`")
        .assert_errors_contain("`Owner`");
    assert!(!run.output().contains("index"));

    let empty = console::test(["prompt:select"])
        .expects_question("Role?", "")
        .run()
        .await;
    empty
        .assert_failed()
        .assert_errors_contain("`Member`, `Owner`");
}

#[tokio::test]
async fn an_empty_answer_takes_the_default() {
    let run = console::test(["prompt:select-default"])
        .expects_question("Role?", "")
        .run()
        .await;
    run.assert_successful().assert_output_contains("index 1");

    let named = console::test(["prompt:default"])
        .expects_question("Name?", "")
        .run()
        .await;
    named.assert_successful().assert_output_contains("name Ada");

    let typed = console::test(["prompt:default"])
        .expects_question("Name?", "Grace")
        .run()
        .await;
    typed
        .assert_successful()
        .assert_output_contains("name Grace");
}

#[tokio::test]
async fn expects_choice_fails_a_menu_that_offers_other_options() {
    let run = console::test(["prompt:select"])
        .expects_choice("Role?", "Owner", ["Member", "Owner", "Admin"])
        .run()
        .await;

    run.assert_failed()
        .assert_errors_contain("`Member`, `Owner`, `Admin`")
        .assert_errors_contain("offers `Member`, `Owner`");
    assert_eq!(
        run.unasked_questions(),
        ["Role?"],
        "the answer was not used"
    );

    let not_a_menu = console::test(["prompt:default"])
        .expects_choice("Name?", "Ada", ["Ada"])
        .run()
        .await;
    not_a_menu
        .assert_failed()
        .assert_errors_contain("offers no options");
}

#[tokio::test]
async fn select_keyed_answers_the_key() {
    for answer in ["Professional", "pro"] {
        let run = console::test(["prompt:keyed"])
            .expects_choice("Plan?", answer, ["Basic", "Professional"])
            .run()
            .await;
        run.assert_successful().assert_output_contains("plan pro");
    }

    let default = console::test(["prompt:keyed"])
        .expects_question("Plan?", "")
        .run()
        .await;
    default
        .assert_successful()
        .assert_output_contains("plan basic");

    let wrong = console::test(["prompt:keyed"])
        .expects_question("Plan?", "gold")
        .run()
        .await;
    wrong
        .assert_failed()
        .assert_errors_contain("`Basic`, `Professional`");
}

#[tokio::test]
async fn multiselect_answers_the_chosen_indexes() {
    let run = console::test(["prompt:multi"])
        .expects_choice("Roles?", "Member,Owner", ["Member", "Owner"])
        .run()
        .await;
    run.assert_successful()
        .assert_output_contains("indexes [0, 1]");

    let spaced = console::test(["prompt:multi"])
        .expects_question("Roles?", "Owner , Member,Owner")
        .run()
        .await;
    spaced
        .assert_successful()
        .assert_output_contains("indexes [0, 1]");

    let defaults = console::test(["prompt:multi"])
        .expects_question("Roles?", "")
        .run()
        .await;
    defaults
        .assert_successful()
        .assert_output_contains("indexes [1]");

    let wrong = console::test(["prompt:multi"])
        .expects_question("Roles?", "Member,Admin")
        .run()
        .await;
    wrong
        .assert_failed()
        .assert_errors_contain("`Admin`")
        .assert_errors_contain("`Member`, `Owner`");
}

#[tokio::test]
async fn progress_maps_every_item_and_writes_the_count_it_finished_at() {
    let run = console::test(["prompt:progress"]).run().await;

    run.assert_successful();
    assert_eq!(run.output(), "Sync 3/3\ncalls 3 results [0, 2, 4]\n");
    assert!(!run.output().contains('\u{1b}'));
}

#[tokio::test]
async fn a_bar_keeps_its_label_and_hint_and_stops_at_the_total() {
    let run = console::test(["prompt:bar"]).run().await;
    run.assert_successful();
    assert_eq!(run.output(), "done 4 of 4\nUploading 4/4 (b.txt)\n");

    let quiet = console::test(["prompt:bar", "-q"]).run().await;
    quiet.assert_successful();
    assert_eq!(quiet.output(), "", "a quiet run draws no bar");
}

#[tokio::test]
async fn a_form_answers_every_value_by_name() {
    let run = console::test(["prompt:form"])
        .expects_question("Name?", "Ada")
        .expects_question("Token?", "t0k")
        .expects_question("Admin?", "yes")
        .expects_choice("Plan?", "Professional", ["Basic", "Professional"])
        .run()
        .await;

    run.assert_successful().assert_every_question_was_asked();
    let answers = FORM_SEEN
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .expect("the form was submitted");
    assert_eq!(answers.text("name"), Some("Ada"));
    assert_eq!(answers.text("token"), Some("t0k"));
    assert_eq!(answers.confirmed("admin"), Some(true));
    assert_eq!(answers.selected("plan"), Some(1));
    assert_eq!(answers.text("missing"), None);
    assert!(!run.output().contains("t0k"), "the secret is not captured");
    assert!(
        !format!("{answers:?}").contains("t0k"),
        "the secret is not in the answers' Debug output"
    );
}

#[tokio::test]
async fn a_form_with_two_prompts_of_one_name_asks_nothing() {
    let run = console::test(["prompt:form-twice"]).run().await;

    run.assert_failed().assert_errors_contain("`name`");
    assert_eq!(run.output(), "", "nothing was asked");
}

/// Set on the child that runs `prompt:stdin` in [`prompts_child`].
const PROMPTS_CHILD: &str = "SUPRNOVA_INFRA_PROMPTS_CHILD";

#[derive(Parser, Command, Debug)]
#[console(name = "prompt:stdin", description = "Asks from the standard input")]
struct AsksFromTheStandardInput {}

#[async_trait]
impl TypedCommand for AsksFromTheStandardInput {
    async fn run(self) -> Result<(), FrameworkError> {
        let role = console::select("Role?", &["Member", "Owner"], None)?;
        let password = console::secret("Password?")?;
        let roles = console::multiselect("Roles?", &["Member", "Owner"], &[])?;
        console::progress("Sync", 0..3, |n, _bar| n);
        console::line(format!(
            "role {role} password {} chars roles {roles:?}",
            password.len()
        ));
        Ok(())
    }
}

/// The child half of
/// [`prompts_read_one_line_each_from_a_standard_input_that_is_no_terminal`]:
/// the console binary's dispatch, with the real standard streams.
#[test]
fn prompts_child() {
    if std::env::var_os(PROMPTS_CHILD).is_none() {
        return;
    }
    let outcome = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(console::dispatch_argv(vec![
            "console".to_owned(),
            "prompt:stdin".to_owned(),
        ]));
    if outcome.is_err() {
        std::process::exit(1);
    }
}

#[test]
fn prompts_read_one_line_each_from_a_standard_input_that_is_no_terminal() {
    let run = |input: &str| -> Output {
        let mut child = Process::new(std::env::current_exe().expect("the test binary"))
            .args([
                "--exact",
                "laravel_infra_gaps::prompts_child",
                "--nocapture",
            ])
            .env(PROMPTS_CHILD, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the child");
        std::io::Write::write_all(
            child.stdin.as_mut().expect("the child's input"),
            input.as_bytes(),
        )
        .expect("write the answers");
        drop(child.stdin.take());
        child.wait_with_output().expect("the child ends")
    };

    let output = run("Owner\nhunter2\nMember,Owner\n");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("role 1 password 7 chars roles [0, 1]"),
        "{stdout}"
    );
    assert!(stdout.contains("Sync 3/3\n"), "{stdout}");
    assert!(!stdout.contains("hunter2"), "{stdout}");
    assert!(
        !stdout.contains('\u{1b}'),
        "a pipe gets no escape sequence: {stdout}"
    );

    let refused = run("Admin\n");
    assert!(!refused.status.success());
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("`Member`, `Owner`"),
        "the error names the options: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// PAR-142: `down`
// ---------------------------------------------------------------------------

/// The arguments of the `down` [`down_child`] runs, as JSON.
const DOWN_CHILD: &str = "SUPRNOVA_INFRA_DOWN_CHILD";
/// The base path [`down_child`] installs.
const DOWN_BASE: &str = "SUPRNOVA_INFRA_DOWN_BASE";
/// What [`down_child`] prints before the number of `MaintenanceModeEnabled`
/// events the run dispatched.
const EVENTS: &str = "maintenance-mode-enabled-events=";

/// Run `app down <args>` through `Application::run_with_args` in a child
/// process, with `base` as the application's base path.
fn down(base: &Path, args: &[&str]) -> Output {
    Process::new(std::env::current_exe().expect("the test binary"))
        .args(["--exact", "laravel_infra_gaps::down_child", "--nocapture"])
        .env(
            DOWN_CHILD,
            serde_json::to_string(args).expect("the arguments as JSON"),
        )
        .env(DOWN_BASE, base)
        .env("APP_ENV", "testing")
        .env("APP_URL", "https://example.test")
        .env_remove("MAINTENANCE_DRIVER")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run the child")
}

/// The child half of [`down`]: `down` under the events fake, then the
/// number of `MaintenanceModeEnabled` events it dispatched.
#[test]
fn down_child() {
    let Ok(args) = std::env::var(DOWN_CHILD) else {
        return;
    };
    let args: Vec<String> = serde_json::from_str(&args).expect("the arguments");
    let base = std::env::var(DOWN_BASE).expect("the base path");
    suprnova::boot::load_env().expect("load the configuration");
    let events = suprnova::EventFacade::fake();
    let outcome = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(async move {
            suprnova::Application::new()
                .bootstrap(move || async move {
                    suprnova::set_base_path(base);
                })
                .run_with_args(
                    ["app".to_owned(), "down".to_owned()]
                        .into_iter()
                        .chain(args),
                )
                .await
        });
    let enabled = suprnova::events::dispatched_count::<suprnova::MaintenanceModeEnabled>(|_| true);
    drop(events);
    println!("{EVENTS}{enabled}");
    if let Err(error) = outcome {
        eprintln!("{}", error.message());
        std::process::exit(1);
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The response a request gets from `MaintenanceMiddleware` over the down
/// file `down` wrote under `base`.
async fn request_while_down(base: &Path) -> suprnova::testing::TestResponse {
    let driver = Arc::new(suprnova::FileMaintenanceMode::with_path(
        base.join("storage/framework/down"),
    ));
    let router = suprnova::Router::new().get("/", |_request: suprnova::Request| async {
        suprnova::http::text("home")
    });
    suprnova::testing::TestClient::new(
        router,
        suprnova::MiddlewareRegistry::new()
            .append(suprnova::MaintenanceMiddleware::with_driver(driver)),
    )
    .get("/")
    .send()
    .await
}

#[test]
fn down_dispatches_the_event_and_prints_the_bypass_address() {
    let base = tempfile::tempdir().expect("a base path");

    let output = down(base.path(), &["--secret", "abc"]);

    let printed = stdout(&output);
    assert!(output.status.success(), "{printed}{}", stderr(&output));
    assert!(
        printed.contains("Application is now in maintenance mode.\n"),
        "{printed}"
    );
    assert!(
        printed.contains("You may bypass maintenance mode via [https://example.test/abc].\n"),
        "{printed}"
    );
    assert!(printed.contains(&format!("{EVENTS}1")), "{printed}");
    assert!(base.path().join("storage/framework/down").is_file());
}

#[test]
fn a_second_down_updates_the_options() {
    let base = tempfile::tempdir().expect("a base path");
    let first = down(base.path(), &[]);
    assert!(first.status.success(), "{}", stderr(&first));

    let second = down(base.path(), &["--retry", "30"]);

    let printed = stdout(&second);
    assert!(second.status.success(), "{printed}{}", stderr(&second));
    assert!(
        printed.contains("Maintenance mode options updated.\n"),
        "{printed}"
    );
    assert!(
        !printed.contains("Application is now in maintenance mode."),
        "{printed}"
    );
    assert!(
        printed.contains(&format!("{EVENTS}1")),
        "an update dispatches the event too: {printed}"
    );
    assert!(
        !printed.contains("bypass"),
        "no secret, no bypass line: {printed}"
    );
}

#[tokio::test]
async fn retry_takes_a_date_and_sends_it_as_retry_after() {
    for (retry, header) in [
        (
            "Sat, 01 Jan 2033 00:00:00 GMT",
            "Sat, 01 Jan 2033 00:00:00 GMT",
        ),
        ("2033-01-01T00:00:00Z", "Sat, 01 Jan 2033 00:00:00 GMT"),
        ("120", "120"),
    ] {
        let base = tempfile::tempdir().expect("a base path");
        let output = down(base.path(), &["--retry", retry]);
        assert!(
            output.status.success(),
            "{retry}: {}{}",
            stdout(&output),
            stderr(&output)
        );

        let response = request_while_down(base.path()).await;
        response.assert_status(503);
        assert_eq!(response.header("Retry-After"), Some(header), "{retry}");
    }
}

#[test]
fn retry_refuses_what_is_neither_seconds_nor_a_date() {
    let base = tempfile::tempdir().expect("a base path");

    let output = down(base.path(), &["--retry", "soon"]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("soon"), "{}", stderr(&output));
    assert!(!base.path().join("storage/framework/down").exists());
}

#[tokio::test]
async fn render_serves_the_rendered_template() {
    let base = tempfile::tempdir().expect("a base path");
    let views = base.path().join("resources/views/errors");
    std::fs::create_dir_all(&views).expect("the views directory");
    std::fs::write(
        views.join("503.html"),
        "<p>Back in {{ retry_after }} seconds</p>",
    )
    .expect("the template");

    let output = down(
        base.path(),
        &["--render", "errors/503.html", "--retry", "60"],
    );
    assert!(
        output.status.success(),
        "{}{}",
        stdout(&output),
        stderr(&output)
    );

    let response = request_while_down(base.path()).await;
    response.assert_status(503);
    assert_eq!(response.body_text(), "<p>Back in 60 seconds</p>");
}

#[test]
fn render_with_a_missing_template_fails_before_maintenance_mode_starts() {
    let base = tempfile::tempdir().expect("a base path");

    let output = down(base.path(), &["--render", "errors/missing.html"]);

    assert!(!output.status.success());
    let errors = stderr(&output);
    assert!(
        errors.contains("Failed to enter maintenance mode: ") && errors.trim_end().ends_with('.'),
        "{errors}"
    );
    assert!(errors.contains("errors/missing.html"), "{errors}");
    assert!(!base.path().join("storage/framework/down").exists());
    assert!(
        stdout(&output).contains(&format!("{EVENTS}0")),
        "{}",
        stdout(&output)
    );
}

#[test]
fn a_down_that_cannot_record_maintenance_mode_prints_laravels_failure_line() {
    let base = tempfile::tempdir().expect("a base path");
    std::fs::write(base.path().join("storage"), b"").expect("a file where storage goes");

    let output = down(base.path(), &[]);

    assert!(!output.status.success());
    let errors = stderr(&output);
    assert!(
        errors.contains("Failed to enter maintenance mode: ") && errors.trim_end().ends_with('.'),
        "{errors}"
    );
    assert!(
        stdout(&output).contains(&format!("{EVENTS}0")),
        "{}",
        stdout(&output)
    );
}
