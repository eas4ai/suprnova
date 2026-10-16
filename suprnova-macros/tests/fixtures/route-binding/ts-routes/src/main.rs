//! BIND-015: drives the TypeScript route generator of `suprnova
//! generate-types` over the project directory it is given, and prints what
//! it read: each named route's parameters and form request, and whether the
//! generated TypeScript still holds a binding field.

use std::path::Path;

use suprnova_cli::commands::generate_routes::{generate_typescript, scan_routes};

fn main() {
    let project = std::env::args()
        .nth(1)
        .expect("the project directory is the first argument");
    let routes = match scan_routes(Path::new(&project)) {
        Ok(routes) => routes,
        Err(error) => {
            eprintln!("scan failed: {error}");
            std::process::exit(1);
        }
    };
    for route in &routes {
        let params: Vec<&str> = route
            .definition
            .path_params
            .iter()
            .map(|param| param.name.as_str())
            .collect();
        println!(
            "route {} params {} request {}",
            route.definition.name.as_deref().unwrap_or("-"),
            params.join(","),
            route
                .request_struct
                .as_ref()
                .map(|form| form.name.as_str())
                .unwrap_or("-")
        );
    }
    println!(
        "typescript-holds-a-field {}",
        generate_typescript(&routes).contains("post:slug")
    );
}
