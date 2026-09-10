use std::fmt::Write as _;

use console::{Style, Term, style};

// ─── Brand colors ───────────────────────────────────────────
// Suprnova's palette: warm explosion (yellow/orange core) with cool edges (cyan/blue)
const BANNER: &str = r#"
  ▄▄▄▄▄                                           
 ██▀▀▀▀█▄                                         
 ▀██▄  ▄▀             ▄    ▄                      
   ▀██▄▄  ██ ██ ████▄ ████▄████▄ ▄███▄▀█▄ ██▀▄▀▀█▄
 ▄   ▀██▄ ██ ██ ██ ██ ██   ██ ██ ██ ██ ██▄██ ▄█▀██
 ▀██████▀▄▀██▀█▄████▀▄█▀  ▄██ ▀█▄▀███▀  ▀█▀ ▄▀█▄██
                ██                                
                ▀                                 
"#;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Print the Suprnova banner with version
pub fn banner() {
    let term = Term::stdout();
    let _ = term.clear_line();
    print!("{}", banner_text());
}

/// Render the Suprnova banner with version.
fn banner_text() -> String {
    let mut out = String::new();

    for (i, line) in BANNER.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let styled = match i {
            1 => style(line).color256(220).bold(),     // bright yellow
            2 => style(line).color256(214).bold(),     // yellow-orange
            3 => style(line).color256(214).bold(),     // orange
            4 => style(line).color256(208).bold(),     // deeper orange
            5 => style(line).color256(203).bold(),     // red-orange
            6 => style(line).color256(197).bold(),     // red
            7 | 8 => style(line).color256(161).bold(), // deep magenta tail
            _ => style(line).cyan(),
        };
        let _ = writeln!(out, "{styled}");
    }
    let _ = writeln!(
        out,
        "  {} {}",
        style("A Rust web framework that doesn't gatekeep.").dim(),
        style(format!("v{VERSION}")).dim().italic(),
    );
    let _ = writeln!(out);
    out
}

/// Section header - used to group related output
pub fn header(text: &str) {
    println!();
    println!("  {}", style(text).cyan().bold().underlined());
    println!();
}

/// Success: ✓ message
pub fn success(msg: &str) {
    println!("  {} {}", style("✓").green().bold(), msg);
}

/// Error: ✗ message  
pub fn error(msg: &str) {
    eprintln!("  {} {}", style("✗").red().bold(), msg);
}

/// Warning: ⚠ message
pub fn warning(msg: &str) {
    eprintln!("  {} {}", style("⚠").yellow().bold(), msg);
}

/// Info: → message
pub fn info(msg: &str) {
    println!("  {} {}", style("→").cyan(), msg);
}

/// Step indicator: [n/total] message
pub fn step(current: usize, total: usize, msg: &str) {
    println!(
        "  {} {}",
        style(format!("[{}/{}]", current, total)).dim().bold(),
        msg,
    );
}

/// Dimmed hint text
pub fn hint(msg: &str) {
    println!("  {}", style(msg).dim());
}

/// A labeled value line:  label ··· value
pub fn label_value(label: &str, value: &str) {
    let dots = ".".repeat(36_usize.saturating_sub(label.len()));
    println!(
        "  {} {} {}",
        style(label).bold(),
        style(dots).dim(),
        style(value).cyan(),
    );
}

/// Print a boxed summary panel
pub fn panel(title: &str, lines: &[&str]) {
    let max_len = lines
        .iter()
        .map(|l| console::measure_text_width(l))
        .max()
        .unwrap_or(40);
    let width = max_len.max(console::measure_text_width(title) + 4) + 4;

    let border = Style::new().dim();

    // Top border
    println!(
        "  {}{}{}",
        border.apply_to("╭─"),
        border.apply_to("─".repeat(width)),
        border.apply_to("─╮"),
    );

    // Title
    let title_pad = width - console::measure_text_width(title);
    println!(
        "  {}  {}{}{}",
        border.apply_to("│"),
        style(title).bold().cyan(),
        " ".repeat(title_pad),
        border.apply_to("│"),
    );

    // Separator
    println!(
        "  {}{}{}",
        border.apply_to("├─"),
        border.apply_to("─".repeat(width)),
        border.apply_to("─┤"),
    );

    // Content lines
    for line in lines {
        let pad = width - console::measure_text_width(line);
        println!(
            "  {}  {}{}{}",
            border.apply_to("│"),
            line,
            " ".repeat(pad),
            border.apply_to("│"),
        );
    }

    // Bottom border
    println!(
        "  {}{}{}",
        border.apply_to("╰─"),
        border.apply_to("─".repeat(width)),
        border.apply_to("─╯"),
    );
}

/// Print a command example in the "next steps" style
pub fn command(cmd: &str) {
    println!("    {}", style(format!("$ {}", cmd)).cyan());
}

/// Newline shorthand
pub fn br() {
    println!();
}

/// The command list the help screen renders, grouped the way it groups them.
///
/// Each entry's first whitespace-separated token is the subcommand name;
/// anything after it is the argument sketch the screen prints. `main`'s
/// tests read this table back and require it to name every subcommand clap
/// defines, so a command added without a line here fails the suite.
pub(crate) const HELP_SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "CREATE",
        &[
            ("new [name]", "Create a new Suprnova project"),
            ("serve", "Start dev servers (backend + frontend)"),
            ("dev:tls", "Trust portless CA + register HTTPS dev URL"),
        ],
    ),
    (
        "GENERATE",
        &[
            ("make:controller <name>", "Scaffold a new controller"),
            ("make:action <name>", "Scaffold a new action"),
            ("make:middleware <name>", "Scaffold a new middleware"),
            ("make:migration <name>", "Scaffold a new migration"),
            ("make:inertia <name>", "Scaffold an Inertia page"),
            ("make:error <name>", "Scaffold a domain error"),
            ("make:task <name>", "Scaffold a scheduled task"),
            ("make:command <name>", "Scaffold a console command"),
        ],
    ),
    (
        "LIVE",
        &[
            ("live:make <name>", "Scaffold a Live component and view"),
            ("live:check", "Check Live views with the integrated checker"),
            ("live:inspect", "Report safe Live runtime state"),
            (
                "live:assets --out <dir>",
                "Publish the reviewed Live runtime artifacts",
            ),
        ],
    ),
    (
        "DATABASE",
        &[
            ("migrate", "Run pending migrations"),
            ("migrate:status", "Show migration status"),
            ("migrate:rollback", "Rollback last migration(s)"),
            ("migrate:fresh", "Drop all tables & re-migrate"),
            ("db:sync", "Sync schema → entity files"),
        ],
    ),
    (
        "SCHEDULE",
        &[
            ("schedule:run", "Run due tasks once"),
            ("schedule:work", "Start scheduler daemon"),
            ("schedule:list", "List registered tasks"),
        ],
    ),
    (
        "WORKFLOW",
        &[
            ("workflow:work", "Start workflow worker"),
            ("workflow:install", "Install workflow migrations"),
        ],
    ),
    (
        "SSR",
        &[
            ("ssr:start", "Launch Inertia SSR worker (foreground)"),
            ("ssr:check", "Verify SSR worker is reachable"),
        ],
    ),
    (
        "DEPLOY",
        &[
            ("docker:init", "Generate production Dockerfile"),
            ("docker:compose", "Generate docker-compose.yml"),
        ],
    ),
    (
        "SECURITY",
        &[("key:generate", "Mint a fresh APP_KEY (AES-256, base64)")],
    ),
    (
        "OTHER",
        &[
            ("generate-types", "Generate TS types from Rust structs"),
            ("web:run", "Run web server (production)"),
        ],
    ),
];

/// Custom help output - replaces clap's generated top-level help.
///
/// `main` also hands [`help_text`] to clap as the top-level command's help,
/// so a bare `suprnova`, `suprnova -h` and `suprnova --help` all render this
/// same screen while every subcommand keeps clap's own generated help.
pub fn print_help() {
    let term = Term::stdout();
    let _ = term.clear_line();
    print!("{}", help_text());
}

/// Render the top-level help screen.
pub fn help_text() -> String {
    let mut out = banner_text();

    let _ = writeln!(out, "  {}", style("USAGE:").bold().underlined());
    let _ = writeln!(out, "    suprnova {}", style("<command> [options]").dim());
    let _ = writeln!(out);

    for (section, commands) in HELP_SECTIONS {
        let _ = writeln!(out, "  {}", style(section).bold().underlined());
        for (cmd, desc) in *commands {
            let pad = 30_usize.saturating_sub(cmd.len());
            let _ = writeln!(
                out,
                "    {}{}{}",
                style(cmd).cyan(),
                " ".repeat(pad),
                style(desc).dim(),
            );
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(
        out,
        "  {}",
        style("Run 'suprnova <command> --help' for details on a specific command.").dim()
    );
    let _ = writeln!(out);

    out
}
