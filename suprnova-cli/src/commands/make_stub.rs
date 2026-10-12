//! The artisan generators that write one file from a stub: `make:factory`,
//! `make:seeder`, `make:cast`, `make:channel`, `make:class`, `make:enum`,
//! `make:event`, `make:listener`, `make:job`, `make:job-middleware`,
//! `make:mail`, `make:notification`, `make:observer`, `make:policy`,
//! `make:provider`, `make:resource`, `make:rule`, `make:scope`,
//! `make:test`, `make:trait` and `make:interface`.
//!
//! Each takes a name that must be a Rust identifier and writes
//! `src/<directory>/<snake>.rs` (`make:class` writes `src/<snake>.rs`, and
//! `make:test` writes `tests/<snake>.rs`). It declares the module on the
//! way, as Rust needs and Laravel's autoloader does not: `pub mod
//! <directory>;` in `src/lib.rs` and `pub mod <snake>;` in the directory's
//! module file, which it creates when the project has none. `make:test`
//! declares nothing, since every file under `tests/` is a crate of its own.
//! An existing file is kept unless `--force` is given, and the run writes
//! every file or none.

use console::style;
use std::path::{Path, PathBuf};

use crate::commands::generator::{self, WriteSet};
use crate::commands::live_make::Naming;
use crate::commands::live_tool::display_text;
use crate::templates::{self, stubs};
use crate::ui;

/// Longest name a generator takes; a longer one is refused, as `live:make`
/// refuses one.
const MAX_NAME_LEN: usize = 64;

/// The kind of file a generator writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `make:factory`
    Factory,
    /// `make:seeder`
    Seeder,
    /// `make:cast`
    Cast,
    /// `make:channel`
    Channel,
    /// `make:class`
    Class,
    /// `make:enum`
    Enum,
    /// `make:event`
    Event,
    /// `make:listener`
    Listener,
    /// `make:job`
    Job,
    /// `make:job-middleware`
    JobMiddleware,
    /// `make:mail`
    Mail,
    /// `make:notification`
    Notification,
    /// `make:observer`
    Observer,
    /// `make:policy`
    Policy,
    /// `make:provider`
    Provider,
    /// `make:resource`
    Resource,
    /// `make:rule`
    Rule,
    /// `make:scope`
    Scope,
    /// `make:test`
    Test,
    /// `make:trait`
    Trait,
    /// `make:interface`
    Interface,
}

/// Where a model-bound stub finds its model when `--model` is not given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelDefault {
    /// The kind binds no model.
    None,
    /// The name without the kind's suffix: `PostFactory` builds `Post`.
    FromName,
    /// `User`, the model every new project has: a scope's name says
    /// nothing about the model it scopes.
    User,
}

impl Kind {
    /// What the kind is called in messages.
    fn noun(self) -> &'static str {
        match self {
            Kind::Factory => "factory",
            Kind::Seeder => "seeder",
            Kind::Cast => "cast",
            Kind::Channel => "channel",
            Kind::Class => "class",
            Kind::Enum => "enum",
            Kind::Event => "event",
            Kind::Listener => "listener",
            Kind::Job => "job",
            Kind::JobMiddleware => "job middleware",
            Kind::Mail => "mailable",
            Kind::Notification => "notification",
            Kind::Observer => "observer",
            Kind::Policy => "policy",
            Kind::Provider => "provider",
            Kind::Resource => "resource",
            Kind::Rule => "rule",
            Kind::Scope => "scope",
            Kind::Test => "test",
            Kind::Trait => "trait",
            Kind::Interface => "interface",
        }
    }

    /// The module directories under `src/` the file goes in, outermost
    /// first, each with the doc comment of the module file a run creates
    /// for it. Empty for `make:class`, whose file is a module of the crate
    /// root, and for `make:test`, whose file is under `tests/`.
    fn dirs(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Kind::Factory => &[("factories", "//! Model factories, for tests and seeders.")],
            Kind::Seeder => &[("seeders", "//! Database seeders, which `db:seed` runs.")],
            Kind::Cast => &[("casts", "//! Custom attribute casts.")],
            Kind::Channel => &[(
                "broadcasting",
                "//! Broadcast channels and who may join them.",
            )],
            Kind::Enum => &[("enums", "//! The application's enums.")],
            Kind::Event => &[("events", "//! Application events.")],
            Kind::Listener => &[("listeners", "//! Event listeners.")],
            Kind::Job => &[("jobs", "//! Queued jobs.")],
            Kind::JobMiddleware => &[
                ("jobs", "//! Queued jobs."),
                ("middleware", "//! Job middleware."),
            ],
            Kind::Mail => &[("mail", "//! Mailables.")],
            Kind::Notification => &[("notifications", "//! Notifications.")],
            Kind::Observer => &[("observers", "//! Model observers.")],
            Kind::Policy => &[("policies", "//! Authorization policies.")],
            Kind::Provider => &[(
                "providers",
                "//! Service registrations the bootstrap calls.",
            )],
            Kind::Resource => &[("resources", "//! JSON:API resources.")],
            Kind::Rule => &[("rules", "//! Custom validation rules.")],
            Kind::Scope => &[("scopes", "//! Global query scopes.")],
            Kind::Trait => &[("concerns", "//! Traits with behaviour types share.")],
            Kind::Interface => &[(
                "contracts",
                "//! Traits that name a contract for implementors to fulfil.",
            )],
            Kind::Class | Kind::Test => &[],
        }
    }

    /// The suffix a type of this kind ends with, appended when the name
    /// lacks it, as `make:middleware` appends `Middleware`.
    fn suffix(self) -> Option<&'static str> {
        match self {
            Kind::Factory => Some("Factory"),
            Kind::Seeder => Some("Seeder"),
            Kind::Observer => Some("Observer"),
            Kind::Policy => Some("Policy"),
            Kind::Resource => Some("Resource"),
            Kind::Scope => Some("Scope"),
            _ => None,
        }
    }

    /// The type names the stub brings into scope. A generated type, model
    /// or event of the same name would clash with the import.
    fn imports(self) -> &'static [&'static str] {
        match self {
            Kind::Factory => &["Factory"],
            Kind::Seeder => &["FrameworkError", "Seeder"],
            Kind::Cast => &["Cast", "FrameworkError"],
            Kind::Channel => &[
                "Channel",
                "ChannelParams",
                "PrivateChannel",
                "Request",
                "Value",
            ],
            Kind::Event => &["Event"],
            Kind::Listener => &["Event", "FrameworkError", "Listener"],
            Kind::Job => &["Deserialize", "FrameworkError", "Job", "Serialize"],
            Kind::JobMiddleware => &[
                "Envelope",
                "FrameworkError",
                "JobMiddleware",
                "JobMiddlewareNext",
                "JobOutcome",
            ],
            Kind::Mail => &["Deserialize", "Mailable", "Serialize"],
            Kind::Notification => &[
                "Deserialize",
                "Notification",
                "NotificationMailable",
                "Serialize",
            ],
            Kind::Observer => &["FrameworkError", "Observer"],
            Kind::Resource => &["Data", "Validate"],
            Kind::Rule => &["Rule", "ValidationMessage"],
            Kind::Scope => &["Builder", "GlobalScope"],
            Kind::Class
            | Kind::Enum
            | Kind::Policy
            | Kind::Provider
            | Kind::Test
            | Kind::Trait
            | Kind::Interface => &[],
        }
    }

    fn model_default(self) -> ModelDefault {
        match self {
            Kind::Factory | Kind::Observer | Kind::Policy => ModelDefault::FromName,
            Kind::Scope => ModelDefault::User,
            _ => ModelDefault::None,
        }
    }
}

/// One run, as the command line gave it.
pub struct Request {
    kind: Kind,
    name: String,
    force: bool,
    model: Option<String>,
    event: Option<String>,
}

impl Request {
    /// A run of `kind`'s generator for `name`; `force` overwrites an
    /// existing file.
    pub fn new(kind: Kind, name: String, force: bool) -> Self {
        Self {
            kind,
            name,
            force,
            model: None,
            event: None,
        }
    }

    /// The model `--model` names, for `make:factory`, `make:observer`,
    /// `make:policy` and `make:scope`.
    pub fn with_model(mut self, model: Option<String>) -> Self {
        self.model = model;
        self
    }

    /// The event `--event` names, for `make:listener`.
    pub fn with_event(mut self, event: Option<String>) -> Self {
        self.event = event;
        self
    }
}

/// Run one generator. A refused name or a failed write prints the reason
/// and exits 1.
pub fn run(request: Request) {
    if let Err(e) = run_inner(&request) {
        ui::error(&e);
        std::process::exit(1);
    }
}

/// The spellings of one name: the type (`OrderShipped`), the module and
/// file stem (`order_shipped`) and the words (`Order Shipped`).
#[derive(Debug, PartialEq, Eq)]
struct Names {
    type_name: String,
    stem: String,
    title: String,
}

impl Names {
    /// The spellings of `type_name`, which [`identifier`] has checked.
    fn of(type_name: String) -> Result<Self, String> {
        let naming = Naming::parse(&type_name)
            .ok_or_else(|| format!("'{type_name}' is longer than {MAX_NAME_LEN} characters"))?;
        let title = naming
            .snake
            .split('_')
            .map(capitalized)
            .collect::<Vec<_>>()
            .join(" ");
        Ok(Self {
            type_name,
            stem: naming.snake,
            title,
        })
    }
}

/// Check that `raw` is a Rust identifier that a module can be named after,
/// and return it as a type name. `role` says what the name is for in the
/// message a bad one gets.
///
/// A name in snake case is read as words (`order_shipped` is
/// `OrderShipped`); any other keeps its spelling with the first letter
/// upper-cased, so `HTTPLog` stays `HTTPLog`.
fn identifier(raw: &str, role: &str) -> Result<String, String> {
    let shown = display_text(raw);
    if raw.is_empty() {
        return Err(format!("a {role} name is required"));
    }
    let is_identifier = raw.starts_with(|c: char| c.is_ascii_alphabetic())
        && raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !is_identifier {
        return Err(format!(
            "'{shown}' is not a valid {role} name: it must be a Rust identifier of ASCII \
             letters, digits and `_`, starting with a letter"
        ));
    }
    if raw.len() > MAX_NAME_LEN {
        return Err(format!(
            "'{shown}' is not a valid {role} name: it is longer than {MAX_NAME_LEN} characters"
        ));
    }
    if raw.contains("__") || raw.ends_with('_') {
        return Err(format!(
            "'{shown}' is not a valid {role} name: `_` must stand between two words"
        ));
    }
    // What `Naming` refuses beyond the checks above is a module name that
    // is a Rust keyword: `Type` would be the module `type`.
    let naming = Naming::parse(raw).ok_or_else(|| {
        format!("'{shown}' is not a valid {role} name: its module name is a Rust keyword")
    })?;
    Ok(if raw.contains('_') {
        naming.pascal
    } else {
        capitalized(raw)
    })
}

/// `word` with its first letter upper-cased.
fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// The plural of the last word of a kebab-case name, by the common English
/// rules: `user` is `users`, `category` is `categories`, `box` is `boxes`.
fn pluralized(kebab: &str) -> String {
    let (head, last) = match kebab.rsplit_once('-') {
        Some((head, last)) => (format!("{head}-"), last),
        None => (String::new(), kebab),
    };
    let ends_with_consonant_y = last.len() > 1
        && last.ends_with('y')
        && !last[..last.len() - 1].ends_with(['a', 'e', 'i', 'o', 'u']);
    let plural = if ends_with_consonant_y {
        format!("{}ies", &last[..last.len() - 1])
    } else if ["s", "x", "z", "ch", "sh"]
        .iter()
        .any(|end| last.ends_with(end))
    {
        format!("{last}es")
    } else {
        format!("{last}s")
    };
    head + &plural
}

/// A model or event the stub imports from the application: its type and
/// the module under `src/models` or `src/events` that holds it.
#[derive(Debug)]
struct Imported {
    type_name: String,
    module: String,
}

impl Imported {
    fn parse(raw: &str, role: &str) -> Result<Self, String> {
        let names = Names::of(identifier(raw, role)?)?;
        Ok(Self {
            module: names.stem,
            type_name: names.type_name,
        })
    }

    /// Warn when `src/<dir>/<module>.rs` is not there: the stub names
    /// the type at that path, and the project does not compile until it
    /// exists.
    fn warn_when_missing(&self, dir: &str, noun: &str, flag: &str) {
        let flat = Path::new("src")
            .join(dir)
            .join(format!("{}.rs", self.module));
        let nested = Path::new("src").join(dir).join(&self.module).join("mod.rs");
        if !flat.is_file() && !nested.is_file() {
            ui::warning(&format!(
                "{} not found: the {noun} names `crate::{dir}::{}::{}`; write it, or name \
                 another with {flag}",
                flat.display(),
                self.module,
                self.type_name
            ));
        }
    }
}

/// Everything a stub is rendered with, and where it goes.
#[derive(Debug)]
struct Plan {
    kind: Kind,
    names: Names,
    /// The path from the crate root to the module (`jobs::middleware::x`).
    module_path: String,
    file: PathBuf,
    model: Option<Imported>,
    event: Option<Imported>,
}

impl Plan {
    fn new(request: &Request) -> Result<Self, String> {
        let kind = request.kind;
        let noun = kind.noun();
        if request.model.is_some() && kind.model_default() == ModelDefault::None {
            return Err(format!("make:{} takes no --model", command_suffix(kind)));
        }
        if request.event.is_some() && kind != Kind::Listener {
            return Err(format!("make:{} takes no --event", command_suffix(kind)));
        }

        let written = identifier(&request.name, noun)?;
        let (type_name, base) = match kind.suffix() {
            Some(suffix) => match written.strip_suffix(suffix) {
                Some("") => {
                    return Err(format!(
                        "'{written}' names no {noun}: put a name before `{suffix}`, as in \
                         `Post{suffix}`"
                    ));
                }
                Some(base) => (written.clone(), base.to_string()),
                None => (format!("{written}{suffix}"), written.clone()),
            },
            None => (written.clone(), written.clone()),
        };
        let names = Names::of(type_name)?;

        let model = match (request.model.as_deref(), kind.model_default()) {
            (_, ModelDefault::None) => None,
            (Some(model), _) => Some(Imported::parse(model, "model")?),
            (None, ModelDefault::FromName) => Some(Imported::parse(&base, "model")?),
            (None, ModelDefault::User) => Some(Imported::parse("User", "model")?),
        };
        let event = match request.event.as_deref() {
            Some(event) => Some(Imported::parse(event, "event")?),
            None => None,
        };

        let imports = kind.imports();
        let clashes = std::iter::once(names.type_name.as_str())
            .chain(model.iter().map(|model| model.type_name.as_str()))
            .chain(event.iter().map(|event| event.type_name.as_str()))
            .find(|name| imports.contains(name));
        if let Some(clash) = clashes {
            return Err(format!(
                "'{clash}' cannot be used here: the {noun} file imports a `{clash}` of its \
                 own; choose another name"
            ));
        }

        let modules: Vec<&str> = kind.dirs().iter().map(|(module, _)| *module).collect();
        let (file, twin) = if kind == Kind::Test {
            (
                Path::new("tests").join(format!("{}.rs", names.stem)),
                Path::new("tests").join(&names.stem).join("main.rs"),
            )
        } else {
            if kind == Kind::Class && ["lib", "main"].contains(&names.stem.as_str()) {
                return Err(format!(
                    "'{}' would write src/{}.rs, the crate's own root; choose another name",
                    names.type_name, names.stem
                ));
            }
            let dir = modules
                .iter()
                .fold(PathBuf::from("src"), |dir, module| dir.join(module));
            (
                dir.join(format!("{}.rs", names.stem)),
                dir.join(&names.stem).join("mod.rs"),
            )
        };
        // A directory module of the same name: a second file for it would
        // make the module ambiguous, which rustc refuses, and its own file
        // declares modules a stub would lose.
        if twin.exists() {
            return Err(format!(
                "{} already holds a `{}` {}; choose another name",
                twin.display(),
                names.stem,
                if kind == Kind::Test { "test" } else { "module" }
            ));
        }

        let module_path = modules
            .iter()
            .copied()
            .chain([names.stem.as_str()])
            .collect::<Vec<_>>()
            .join("::");
        Ok(Self {
            kind,
            names,
            module_path,
            file,
            model,
            event,
        })
    }

    /// The stub, rendered with this run's names.
    fn render(&self) -> String {
        let names = &self.names;
        let model_type = self.model.as_ref().map_or("", |model| &model.type_name);
        let model_module = self.model_module();
        // A policy of `User` takes the user twice, so the model parameter
        // is `model` there, as Laravel's policy stub names it.
        let model_param = if self.kind == Kind::Policy && model_module == "user" {
            "model"
        } else {
            model_module
        };
        let model_kebab = model_module.replace('_', "-");
        let model_import = if model_module == "user" || model_type.is_empty() {
            String::new()
        } else {
            format!("\nuse crate::models::{model_module}::{model_type};")
        };
        let event_type = self.event.as_ref().map_or("", |event| &event.type_name);
        let event_module = self.event.as_ref().map_or("", |event| &event.module);
        let channel = self.channel_name();
        let resource_type = self.resource_type();

        let stub = match self.kind {
            Kind::Factory => stubs::FACTORY,
            Kind::Seeder => stubs::SEEDER,
            Kind::Cast => stubs::CAST,
            Kind::Channel => stubs::CHANNEL,
            Kind::Class => stubs::CLASS,
            Kind::Enum => stubs::ENUM,
            Kind::Event => stubs::EVENT,
            Kind::Listener if self.event.is_some() => stubs::LISTENER_EVENT,
            Kind::Listener => stubs::LISTENER_ANY,
            Kind::Job => stubs::JOB,
            Kind::JobMiddleware => stubs::JOB_MIDDLEWARE,
            Kind::Mail => stubs::MAIL,
            Kind::Notification => stubs::NOTIFICATION,
            Kind::Observer => stubs::OBSERVER,
            Kind::Policy => stubs::POLICY,
            Kind::Provider => stubs::PROVIDER,
            Kind::Resource => stubs::RESOURCE,
            Kind::Rule => stubs::RULE,
            Kind::Scope => stubs::SCOPE,
            Kind::Test => stubs::TEST,
            Kind::Trait => stubs::TRAIT,
            Kind::Interface => stubs::INTERFACE,
        };
        templates::render_placeholders(
            stub,
            &[
                ("{name}", &names.type_name),
                ("{stem}", &names.stem),
                ("{title}", &names.title),
                ("{module}", &self.module_path),
                ("{model}", model_type),
                ("{model_module}", model_module),
                ("{model_param}", model_param),
                ("{model_kebab}", &model_kebab),
                ("{model_import}", &model_import),
                ("{event}", event_type),
                ("{event_module}", event_module),
                ("{channel}", &channel),
                ("{resource_type}", &resource_type),
            ],
        )
    }

    /// The module the model is in, under `src/models`.
    fn model_module(&self) -> &str {
        self.model
            .as_ref()
            .map_or("", |model| model.module.as_str())
    }

    /// The name a client subscribes to a channel by: the type's words
    /// without `Channel`, joined by dots, so `OrderUpdates` is
    /// `order.updates` and `ChatChannel` is `chat`.
    fn channel_name(&self) -> String {
        let base = self
            .names
            .type_name
            .strip_suffix("Channel")
            .unwrap_or(&self.names.type_name);
        Naming::parse(base)
            .map(|naming| naming.snake)
            .unwrap_or_else(|| self.names.stem.clone())
            .replace('_', ".")
    }

    /// The JSON:API type of a resource: the plural of its words without
    /// `Resource`, in kebab case, so `UserProfileResource` is
    /// `user-profiles`.
    fn resource_type(&self) -> String {
        let base = self
            .names
            .type_name
            .strip_suffix("Resource")
            .unwrap_or(&self.names.type_name);
        Naming::parse(base)
            .map(|naming| pluralized(&naming.kebab))
            .unwrap_or_else(|| self.names.stem.clone())
    }
}

/// `factory` for `make:factory`: the command name without `make:`.
fn command_suffix(kind: Kind) -> &'static str {
    match kind {
        Kind::JobMiddleware => "job-middleware",
        Kind::Mail => "mail",
        _ => kind.noun(),
    }
}

fn run_inner(request: &Request) -> Result<(), String> {
    let plan = Plan::new(request)?;
    let kind = plan.kind;

    if plan.file.exists() && !request.force {
        // Laravel's generators report an existing file and exit 0, as
        // `make:view` and `make:inertia` do.
        ui::warning(&format!(
            "{} already exists; nothing was written (pass --force to overwrite)",
            plan.file.display()
        ));
        return Ok(());
    }
    if let Some(model) = &plan.model {
        model.warn_when_missing("models", kind.noun(), "--model");
    }
    if let Some(event) = &plan.event {
        event.warn_when_missing("events", kind.noun(), "--event");
    }

    let mut files = WriteSet::default();
    files.put(plan.file.clone(), plan.render())?;
    if kind != Kind::Test {
        declare_modules(&mut files, kind.dirs(), &plan.names.stem)?;
    }
    generator::report(&files.apply()?);
    print_next_steps(&plan);
    Ok(())
}

/// Plan the declarations that reach the new module: `pub mod <first>;` in
/// `src/lib.rs`, then one `pub mod` in the module file of each directory on
/// the way, creating a module file the project lacks. A declaration that
/// is already there is left alone.
fn declare_modules(
    files: &mut WriteSet,
    dirs: &[(&'static str, &'static str)],
    stem: &str,
) -> Result<(), String> {
    let chain: Vec<&str> = dirs
        .iter()
        .map(|(module, _)| *module)
        .chain([stem])
        .collect();
    let first = chain[0];
    let lib_rs = PathBuf::from("src/lib.rs");
    if lib_rs.is_file() {
        files.edit(lib_rs, |current| {
            let content = current?;
            (!generator::declares_module(content, first))
                .then(|| generator::insert_declaration(content, &format!("pub mod {first};")))
        })?;
    } else {
        ui::warning(&format!(
            "src/lib.rs not found: declare `pub mod {first};` in your crate root"
        ));
    }

    let mut parent = PathBuf::from("src");
    for (index, (module, header)) in dirs.iter().enumerate() {
        let child = chain[index + 1];
        let declaration = format!("pub mod {child};");
        files.edit(
            generator::module_file(&parent, module),
            |current| match current {
                Some(content) if generator::declares_module(content, child) => None,
                Some(content) => Some(generator::insert_declaration(content, &declaration)),
                None => Some(format!("{header}\n\n{declaration}\n")),
            },
        )?;
        parent = parent.join(module);
    }
    Ok(())
}

/// Tell the developer what the new file is for and where it gets wired in.
fn print_next_steps(plan: &Plan) {
    let kind = plan.kind;
    let name = &plan.names.type_name;
    let path = format!("crate::{}::{name}", plan.module_path);
    let model = plan
        .model
        .as_ref()
        .map_or("", |model| model.type_name.as_str());

    ui::br();
    ui::info(&format!(
        "{} {} created",
        capitalized(kind.noun()),
        style(name).cyan().bold()
    ));
    ui::br();
    let (hint, commands): (&str, Vec<String>) = match kind {
        Kind::Factory => (
            "Build models with it:",
            vec![format!("{name}::new().count(10).create_many().await?")],
        ),
        Kind::Seeder => (
            "Register it in src/bootstrap.rs, then run it:",
            vec![
                format!("suprnova::seed::register::<{path}>();"),
                format!("suprnova db:seed {name}"),
            ],
        ),
        Kind::Cast => (
            "Name it on a model field:",
            vec![format!("#[model(casts = {{ field = {path} }})]")],
        ),
        Kind::Channel => (
            "Register it in the channel registry the bootstrap binds:",
            vec![format!("registry.register({path});")],
        ),
        Kind::Class | Kind::Enum => ("Use it:", vec![format!("use {path};")]),
        Kind::Event => (
            "Dispatch it:",
            vec![format!("EventFacade::dispatch({path} {{}}).await?;")],
        ),
        Kind::Listener => {
            let event = plan
                .event
                .as_ref()
                .map_or("YourEvent", |event| event.type_name.as_str());
            (
                "Register it in src/bootstrap.rs:",
                vec![format!(
                    "EventFacade::listen::<{event}, _>(Arc::new({path})).await;"
                )],
            )
        }
        Kind::Job => (
            "Register it at boot, then push it:",
            vec![
                format!("suprnova::queue::worker::register_job::<{path}>();"),
                format!("Queue::push({name} {{}}).await?;"),
            ],
        ),
        Kind::JobMiddleware => (
            "Return it from a job's middleware():",
            vec![format!("vec![Arc::new({path})]")],
        ),
        Kind::Mail => (
            "Send it:",
            vec![format!("Mail::to(address).send({name} {{}}).await?;")],
        ),
        Kind::Notification => (
            "Register its mail renderer at boot, then send it:",
            vec![
                format!("suprnova::register_mail_renderer::<{path}>()?;"),
                format!("Notify::send(&user, &{name} {{}}).await?;"),
            ],
        ),
        Kind::Observer => (
            "Nothing to register: #[suprnova::observer] installs it when the application boots.",
            Vec::new(),
        ),
        Kind::Policy => (
            "Each method is a gate; check one in a handler:",
            vec![format!(
                "#[authorize(\"update-{}\", {model})]",
                plan.model_module().replace('_', "-")
            )],
        ),
        Kind::Provider => (
            "Call it from register in src/bootstrap.rs:",
            vec![format!("crate::{}::register().await;", plan.module_path)],
        ),
        Kind::Resource => (
            "Render it from a handler:",
            vec!["Resource::single(resource).render().await".to_string()],
        ),
        Kind::Rule => (
            "Use it in a validate! row:",
            vec![format!("field => Required, {name};")],
        ),
        Kind::Scope => (
            "Register it at boot:",
            vec![format!("ScopeRegistry::register::<{model}, _>({path});")],
        ),
        Kind::Test => (
            "Run it:",
            vec![format!("cargo test --test {}", plan.names.stem)],
        ),
        Kind::Trait => (
            "Implement it:",
            vec![format!("impl {path} for YourType {{}}")],
        ),
        Kind::Interface => (
            "Implement it, then bind an implementor in the container:",
            vec![format!("App::bind::<dyn {path}>(Arc::new(implementor));")],
        ),
    };
    ui::hint(hint);
    for command in &commands {
        ui::command(command);
    }
    ui::br();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(kind: Kind, name: &str) -> Result<Plan, String> {
        Plan::new(&Request::new(kind, name.to_string(), false))
    }

    #[test]
    fn a_name_keeps_its_spelling_or_is_read_as_words() {
        assert_eq!(identifier("OrderShipped", "event").unwrap(), "OrderShipped");
        assert_eq!(identifier("orderShipped", "event").unwrap(), "OrderShipped");
        assert_eq!(
            identifier("order_shipped", "event").unwrap(),
            "OrderShipped"
        );
        assert_eq!(identifier("HTTPLog", "event").unwrap(), "HTTPLog");
        let names = Names::of("HTTPLog".to_string()).unwrap();
        assert_eq!(names.stem, "http_log");
        assert_eq!(names.title, "Http Log");
    }

    #[test]
    fn a_name_that_is_not_an_identifier_is_refused_with_the_reason() {
        for (bad, reason) in [
            ("", "is required"),
            ("1st", "Rust identifier"),
            ("_hidden", "Rust identifier"),
            ("order-shipped", "Rust identifier"),
            ("Admin/Users", "Rust identifier"),
            ("Ünicode", "Rust identifier"),
            ("a__b", "between two words"),
            ("trailing_", "between two words"),
            ("Type", "keyword"),
        ] {
            let error = identifier(bad, "event").expect_err(bad);
            assert!(error.contains(reason), "{bad:?}: {error}");
        }
        let long = "A".repeat(MAX_NAME_LEN + 1);
        assert!(
            identifier(&long, "event")
                .unwrap_err()
                .contains("longer than")
        );
    }

    #[test]
    fn a_suffix_is_appended_once_and_names_the_model() {
        let factory = plan(Kind::Factory, "Post").unwrap();
        assert_eq!(factory.names.type_name, "PostFactory");
        assert_eq!(factory.file, Path::new("src/factories/post_factory.rs"));
        assert_eq!(factory.model.as_ref().unwrap().type_name, "Post");
        let same = plan(Kind::Factory, "PostFactory").unwrap();
        assert_eq!(same.names.type_name, "PostFactory");
        assert_eq!(same.model.as_ref().unwrap().module, "post");
        assert!(
            plan(Kind::Factory, "Factory")
                .unwrap_err()
                .contains("names no factory")
        );
        let scope = plan(Kind::Scope, "Active").unwrap();
        assert_eq!(scope.names.type_name, "ActiveScope");
        assert_eq!(scope.model.as_ref().unwrap().type_name, "User");
    }

    #[test]
    fn every_kind_writes_where_the_manual_says() {
        for (kind, name, file, module_path) in [
            (
                Kind::Seeder,
                "Users",
                "src/seeders/users_seeder.rs",
                "seeders::users_seeder",
            ),
            (
                Kind::Cast,
                "AsMoney",
                "src/casts/as_money.rs",
                "casts::as_money",
            ),
            (
                Kind::Channel,
                "OrderUpdates",
                "src/broadcasting/order_updates.rs",
                "broadcasting::order_updates",
            ),
            (Kind::Class, "Invoice", "src/invoice.rs", "invoice"),
            (Kind::Enum, "Status", "src/enums/status.rs", "enums::status"),
            (
                Kind::Event,
                "OrderShipped",
                "src/events/order_shipped.rs",
                "events::order_shipped",
            ),
            (
                Kind::Listener,
                "SendReceipt",
                "src/listeners/send_receipt.rs",
                "listeners::send_receipt",
            ),
            (
                Kind::Job,
                "ProcessPodcast",
                "src/jobs/process_podcast.rs",
                "jobs::process_podcast",
            ),
            (
                Kind::JobMiddleware,
                "RateLimitedByUser",
                "src/jobs/middleware/rate_limited_by_user.rs",
                "jobs::middleware::rate_limited_by_user",
            ),
            (
                Kind::Mail,
                "Welcome",
                "src/mail/welcome.rs",
                "mail::welcome",
            ),
            (
                Kind::Notification,
                "InvoicePaid",
                "src/notifications/invoice_paid.rs",
                "notifications::invoice_paid",
            ),
            (
                Kind::Observer,
                "User",
                "src/observers/user_observer.rs",
                "observers::user_observer",
            ),
            (
                Kind::Policy,
                "Note",
                "src/policies/note_policy.rs",
                "policies::note_policy",
            ),
            (
                Kind::Provider,
                "PaymentServiceProvider",
                "src/providers/payment_service_provider.rs",
                "providers::payment_service_provider",
            ),
            (
                Kind::Resource,
                "User",
                "src/resources/user_resource.rs",
                "resources::user_resource",
            ),
            (
                Kind::Rule,
                "Uppercase",
                "src/rules/uppercase.rs",
                "rules::uppercase",
            ),
            (Kind::Test, "UserTest", "tests/user_test.rs", "user_test"),
            (
                Kind::Trait,
                "HasUuid",
                "src/concerns/has_uuid.rs",
                "concerns::has_uuid",
            ),
            (
                Kind::Interface,
                "PaymentGateway",
                "src/contracts/payment_gateway.rs",
                "contracts::payment_gateway",
            ),
        ] {
            let plan = plan(kind, name).unwrap();
            assert_eq!(plan.file, Path::new(file), "{kind:?}");
            assert_eq!(plan.module_path, module_path, "{kind:?}");
        }
    }

    #[test]
    fn a_name_that_clashes_with_an_import_is_refused() {
        for (kind, name) in [
            (Kind::Rule, "Rule"),
            (Kind::Job, "Job"),
            (Kind::Event, "Event"),
            (Kind::Channel, "Request"),
            (Kind::Cast, "Cast"),
        ] {
            let error = plan(kind, name).err().unwrap_or_default();
            assert!(error.contains("imports a"), "{kind:?} {name}: {error}");
        }
        let scope = Plan::new(
            &Request::new(Kind::Scope, "Active".to_string(), false)
                .with_model(Some("Builder".to_string())),
        );
        assert!(scope.err().unwrap_or_default().contains("imports a"));
    }

    #[test]
    fn options_belong_to_their_commands() {
        let model =
            Request::new(Kind::Job, "Ship".to_string(), false).with_model(Some("User".to_string()));
        assert!(
            Plan::new(&model)
                .err()
                .unwrap_or_default()
                .contains("no --model")
        );
        let event = Request::new(Kind::Job, "Ship".to_string(), false)
            .with_event(Some("Shipped".to_string()));
        assert!(
            Plan::new(&event)
                .err()
                .unwrap_or_default()
                .contains("no --event")
        );
        let class = plan(Kind::Class, "Lib").err().unwrap_or_default();
        assert!(class.contains("crate's own root"), "{class}");
    }

    #[test]
    fn channels_and_resources_are_named_from_the_type() {
        assert_eq!(
            plan(Kind::Channel, "OrderUpdates").unwrap().channel_name(),
            "order.updates"
        );
        assert_eq!(
            plan(Kind::Channel, "ChatChannel").unwrap().channel_name(),
            "chat"
        );
        assert_eq!(
            plan(Kind::Channel, "AdminAlertsChannel")
                .unwrap()
                .channel_name(),
            "admin.alerts"
        );
        assert_eq!(
            plan(Kind::Resource, "User").unwrap().resource_type(),
            "users"
        );
        assert_eq!(
            plan(Kind::Resource, "UserProfileResource")
                .unwrap()
                .resource_type(),
            "user-profiles"
        );
        assert_eq!(pluralized("category"), "categories");
        assert_eq!(pluralized("key"), "keys");
        assert_eq!(pluralized("box"), "boxes");
        assert_eq!(pluralized("status"), "statuses");
        assert_eq!(pluralized("search-match"), "search-matches");
    }

    #[test]
    fn a_policy_of_user_takes_the_user_once() {
        let source = plan(Kind::Policy, "User").unwrap().render();
        assert_eq!(
            source.matches("use crate::models::user::User;").count(),
            1,
            "{source}"
        );
        assert!(source.contains("#[policy(User, User)]"), "{source}");
        assert!(
            source.contains("fn update(_user: &User, _model: &User) -> bool"),
            "{source}"
        );

        let note = plan(Kind::Policy, "Note").unwrap().render();
        assert!(note.contains("use crate::models::note::Note;"), "{note}");
        assert!(
            note.contains("fn update(_user: &User, _note: &Note) -> bool"),
            "{note}"
        );
        assert!(note.contains("`update-note`"), "{note}");
    }

    #[test]
    fn no_placeholder_is_left_in_any_stub() {
        let mut requests: Vec<Request> = [
            (Kind::Factory, "User"),
            (Kind::Seeder, "Users"),
            (Kind::Cast, "AsMoney"),
            (Kind::Channel, "OrderUpdates"),
            (Kind::Class, "Invoice"),
            (Kind::Enum, "Status"),
            (Kind::Event, "OrderShipped"),
            (Kind::Listener, "AuditEvents"),
            (Kind::Job, "ProcessPodcast"),
            (Kind::JobMiddleware, "RateLimitedByUser"),
            (Kind::Mail, "Welcome"),
            (Kind::Notification, "InvoicePaid"),
            (Kind::Observer, "User"),
            (Kind::Policy, "Note"),
            (Kind::Provider, "PaymentServiceProvider"),
            (Kind::Resource, "User"),
            (Kind::Rule, "Uppercase"),
            (Kind::Scope, "Active"),
            (Kind::Test, "UserTest"),
            (Kind::Trait, "HasUuid"),
            (Kind::Interface, "PaymentGateway"),
        ]
        .into_iter()
        .map(|(kind, name)| Request::new(kind, name.to_string(), false))
        .collect();
        requests.push(
            Request::new(Kind::Listener, "SendReceipt".to_string(), false)
                .with_event(Some("OrderShipped".to_string())),
        );
        for request in &requests {
            let source = Plan::new(request).unwrap().render();
            for key in [
                "{name}",
                "{stem}",
                "{title}",
                "{module}",
                "{model",
                "{event",
                "{channel}",
                "{resource_type}",
            ] {
                assert!(
                    !source.contains(key),
                    "{:?} left {key}:\n{source}",
                    request.kind
                );
            }
        }
    }
}
