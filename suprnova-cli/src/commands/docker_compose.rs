//! docker:compose command - Generate docker-compose.yml for local development

use dialoguer::{Confirm, theme::ColorfulTheme};
use std::fs;
use std::path::Path;

use crate::commands::cargo_meta;
use crate::templates;
use crate::ui;

/// Let callers handle generation failures while preserving the original service selection.
pub fn run(with_mailpit: bool, with_minio: bool) -> std::io::Result<()> {
    run_with_rustrak(with_mailpit, with_minio, false)
}

/// Return filesystem failures so only the CLI dispatcher decides when to exit.
pub fn run_with_rustrak(
    with_mailpit: bool,
    with_minio: bool,
    with_rustrak: bool,
) -> std::io::Result<()> {
    if !Path::new("Cargo.toml").exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Cargo.toml not found; run docker:compose from a Suprnova project root directory",
        ));
    }

    let project_name = get_project_name();
    let compose_path = Path::new("docker-compose.yml");

    if compose_path.exists() {
        ui::warning("docker-compose.yml already exists");
        ui::hint("Remove or rename the existing docker-compose.yml to generate a new one.");
        return Ok(());
    }

    let (include_mailpit, include_minio, include_rustrak) =
        prompt_for_services(with_mailpit, with_minio, with_rustrak);

    let generated = if include_rustrak {
        templates::docker_compose_template_with_rustrak(
            &project_name,
            include_mailpit,
            include_minio,
            true,
        )
    } else {
        templates::docker_compose_template(&project_name, include_mailpit, include_minio)
    };
    crate::secure_fs::write_private(compose_path, &generated.yaml).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("Failed to write docker-compose.yml: {error}"),
        )
    })?;
    ui::success("Created docker-compose.yml");

    update_gitignore();

    print_instructions(&generated, include_mailpit, include_minio, include_rustrak);
    Ok(())
}

fn get_project_name() -> String {
    cargo_meta::package_name_from_path(Path::new("Cargo.toml")).unwrap_or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_else(|| "suprnova_app".to_string())
    })
}

fn prompt_for_services(
    with_mailpit: bool,
    with_minio: bool,
    with_rustrak: bool,
) -> (bool, bool, bool) {
    if with_mailpit || with_minio || with_rustrak {
        return (with_mailpit, with_minio, with_rustrak);
    }

    ui::br();
    ui::header("Optional Services");
    ui::hint("PostgreSQL and Redis are included by default.");
    ui::br();

    let include_mailpit = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Include Mailpit (email testing)?")
        .default(false)
        .interact()
        .unwrap_or(false);

    let include_minio = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Include MinIO (S3-compatible storage)?")
        .default(false)
        .interact()
        .unwrap_or(false);

    let include_rustrak = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Include Rustrak (error reporting)?")
        .default(false)
        .interact()
        .unwrap_or(false);

    ui::br();

    (include_mailpit, include_minio, include_rustrak)
}

fn update_gitignore() {
    let gitignore_path = Path::new(".gitignore");
    if !gitignore_path.exists() {
        return;
    }

    let content = match fs::read_to_string(gitignore_path) {
        Ok(c) => c,
        Err(_) => return,
    };

    if content.contains("docker-compose.override.yml") {
        return;
    }

    let new_content = format!(
        "{}\n# Local Docker overrides\ndocker-compose.override.yml\n",
        content.trim_end()
    );

    if fs::write(gitignore_path, new_content).is_ok() {
        ui::success("Updated .gitignore");
    }
}

fn print_instructions(
    generated: &templates::GeneratedCompose,
    has_mailpit: bool,
    has_minio: bool,
    has_rustrak: bool,
) {
    ui::br();

    // Every port is published on 127.0.0.1 by the template, so naming
    // localhost here is accurate rather than optimistic.
    let mut services = vec![
        "PostgreSQL ···· 127.0.0.1:5432".to_string(),
        "Redis ········· 127.0.0.1:6379".to_string(),
    ];
    if has_mailpit {
        services.push("Mailpit SMTP ·· 127.0.0.1:1025".to_string());
        services.push("Mailpit UI ···· http://127.0.0.1:8025".to_string());
    }
    if has_minio {
        services.push("MinIO API ····· 127.0.0.1:9000".to_string());
        services.push("MinIO Console · http://127.0.0.1:9001".to_string());
    }
    if has_rustrak {
        services.push("Rustrak ······· http://127.0.0.1:8080".to_string());
    }
    let service_refs: Vec<&str> = services.iter().map(String::as_str).collect();
    ui::panel("Services", &service_refs);

    ui::br();
    ui::hint("Start:");
    if has_rustrak {
        ui::hint("Set RUSTRAK_CREATE_SUPERUSER to email:password for the first admin.");
        ui::hint("Keep the generated session key in docker-compose.yml across restarts.");
    }
    ui::command("docker compose up -d");
    ui::br();
    ui::hint("Update your .env:");
    // The password is generated per project and written only into
    // docker-compose.yml, so this line is the one place the operator can
    // read it without going and grepping the file.
    ui::command(&format!(
        "DATABASE_URL=postgres://suprnova:{}@127.0.0.1:5432/suprnova_db",
        generated.db_password
    ));
    if let Some(minio_password) = &generated.minio_password {
        ui::br();
        ui::hint("MinIO root credentials (also in docker-compose.yml):");
        ui::command(&format!("suprnova / {minio_password}"));
    }
    if has_rustrak {
        ui::br();
        ui::hint("Create a project in Rustrak and copy its DSN into REPORTING_DSN.");
        ui::hint(
            "Live uses REPORTING_PUBLIC_DSN. Inertia requires both REPORTING_PUBLIC_DSN and VITE_REPORTING_DSN.",
        );
    }
    ui::br();
    ui::hint("Services are published on 127.0.0.1 only. To reach them from");
    ui::hint("another host, set DB_HOST_BIND / REDIS_HOST_BIND - and put");
    ui::hint("them behind something that authenticates first.");
    ui::br();
}
