use magnetar::default_migration::DefaultMigrationBindings;
use magnetar::migration::{MigrationEngine, MigrationRunner, ShapeConfirmation, SourceShape};
use sea_orm::Database;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let source_shape = value_after(&arguments, "--source-shape")
        .ok_or("--source-shape torii|suprnova-web|suprnova-api|magnetar is required")?;
    let selected = SourceShape::parse_cli(source_shape)?;
    let source_url = value_after(&arguments, "--database-url")
        .map(str::to_owned)
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .ok_or("--database-url or DATABASE_URL is required")?;
    let app_url = value_after(&arguments, "--app-database-url")
        .map(str::to_owned)
        .or_else(|| std::env::var("MAGNETAR_APP_DATABASE_URL").ok())
        .ok_or("--app-database-url or MAGNETAR_APP_DATABASE_URL is required")?;

    let source = Database::connect(&source_url).await?;
    let app = Database::connect(&app_url).await?;
    magnetar::default_schema::migrate(&app).await?;
    let runner = MigrationEngine::new(source, DefaultMigrationBindings::new(app));
    let detected = runner.detect_shape().await?;
    let confirmation = ShapeConfirmation {
        detected,
        operator_selected: selected,
    };

    if arguments.iter().any(|argument| argument == "--abort") {
        runner.abort().await?;
    } else if arguments.iter().any(|argument| argument == "--restore") {
        println!("{:#?}", runner.restore().await?);
    } else {
        let plan = runner.dry_run(confirmation).await?;
        println!("{plan:#?}");
        if arguments.iter().any(|argument| argument == "--apply") {
            println!("{:#?}", runner.apply(&plan).await?);
        }
    }
    Ok(())
}

fn value_after<'a>(arguments: &'a [String], flag: &str) -> Option<&'a str> {
    arguments
        .iter()
        .position(|argument| argument == flag)
        .and_then(|index| arguments.get(index + 1))
        .map(String::as_str)
}
