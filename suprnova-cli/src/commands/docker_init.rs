//! docker:init command - Generate production-ready Dockerfile

use std::fs;
use std::path::Path;

use crate::commands::cargo_meta;
use crate::templates;
use crate::ui;

pub fn run() {
    if !Path::new("Cargo.toml").exists() {
        ui::error("Cargo.toml not found");
        ui::hint("Make sure you're in a Suprnova project root directory.");
        std::process::exit(1);
    }

    let package_name = get_package_name();

    let dockerfile_path = Path::new("Dockerfile");
    let dockerignore_path = Path::new(".dockerignore");

    if dockerfile_path.exists() {
        ui::warning("Dockerfile already exists");
        ui::hint("Remove or rename the existing Dockerfile to generate a new one.");
        std::process::exit(0);
    }

    // Emit the Dockerfile that matches the scaffold this project came
    // from. Through v0.7.2 there was only one, and it was the full-stack
    // one — so on an API project the very first instruction,
    // `COPY frontend/package.json`, failed outright and `suprnova new
    // --api` + `docker:init` + `docker build` could not succeed.
    let manifest = fs::read_to_string("Cargo.toml").unwrap_or_default();
    let kind = cargo_meta::detect_project_kind(&manifest);

    let dockerfile_content = match kind {
        cargo_meta::ProjectKind::Api => templates::api_dockerfile_template(&package_name),
        cargo_meta::ProjectKind::FullStack => templates::dockerfile_template(&package_name),
    };
    if let Err(e) = fs::write(dockerfile_path, dockerfile_content) {
        ui::error(&format!("Failed to write Dockerfile: {}", e));
        std::process::exit(1);
    }
    match kind {
        cargo_meta::ProjectKind::Api => {
            ui::success("Created Dockerfile (API project — no frontend stage)")
        }
        cargo_meta::ProjectKind::FullStack => ui::success("Created Dockerfile"),
    }

    if !dockerignore_path.exists() {
        let dockerignore_content = templates::dockerignore_template();
        if let Err(e) = fs::write(dockerignore_path, dockerignore_content) {
            ui::error(&format!("Failed to write .dockerignore: {}", e));
            std::process::exit(1);
        }
        ui::success("Created .dockerignore");
    }

    ui::br();
    ui::panel(
        "Docker",
        &[
            &format!("docker build -t {} .", package_name),
            // 8765, matching SERVER_PORT and EXPOSE in the generated
            // Dockerfile. This printed 8080 through v0.7.2, so following
            // the instruction verbatim published a port nothing was
            // listening on and the container looked dead.
            &format!(
                "docker run -p 8765:8765 --env-file .env.production {}",
                package_name
            ),
        ],
    );
    ui::br();
    ui::hint("Create a .env.production file with your production environment variables.");
    ui::br();
}

fn get_package_name() -> String {
    cargo_meta::package_name_from_path(Path::new("Cargo.toml")).unwrap_or_else(|| "app".to_string())
}
