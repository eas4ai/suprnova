//! Console commands: the index and generation commands resolve declared indexes before
//! invoking the generation coordinator; the baseline command measures relevance and latency.

use super::{
    CollectionEngine, DatabaseSearchEngine, QdrantSearchEngine, SearchConfig, SearchEmbedder,
    baseline::{Baseline, BaselineFloors, Calibration, Fixture, fixture_directory},
    generations,
    registry::SearchRegistry,
};
use crate::{DB, DatabaseConfig, FrameworkError};
use std::path::PathBuf;
use suprnova_macros::command;

fn parse_args(
    command: &str,
    args: &[String],
    required: bool,
    model: bool,
    allow_prune: bool,
) -> Result<(Option<&'static str>, bool), FrameworkError> {
    let kind = if model { "model or index" } else { "index" };
    let mut name = None;
    let mut prune = false;
    for argument in args {
        if allow_prune && argument == "--prune" {
            if prune {
                return Err(FrameworkError::bad_request(format!(
                    "{command}: --prune supplied twice"
                )));
            }
            prune = true;
        } else if argument.starts_with('-') {
            return Err(FrameworkError::bad_request(format!(
                "{command}: unknown option '{argument}'"
            )));
        } else if argument.trim().is_empty() || name.replace(argument.as_str()).is_some() {
            return Err(FrameworkError::bad_request(format!(
                "{command}: requires one {kind} name"
            )));
        }
    }
    let Some(name) = name else {
        if required {
            return Err(FrameworkError::bad_request(format!(
                "{command}: requires a {kind} name"
            )));
        }
        return Ok((None, prune));
    };
    let entry = if model {
        SearchRegistry::by_model_or_index(name)
    } else {
        SearchRegistry::by_index(name)
    }
    .map_err(|error| FrameworkError::bad_request(format!("{command}: {error}")))?;
    Ok((Some(entry.index), prune))
}

#[command(
    name = "search:import",
    description = "Import a searchable model or index"
)]
async fn import(args: Vec<String>) -> Result<(), FrameworkError> {
    let (index, _) = parse_args("search:import", &args, true, true, false)?;
    if let Some(index) = index {
        generations::import_index(index).await?;
    }
    Ok(())
}

#[command(
    name = "search:flush",
    description = "Remove all documents from a model or index"
)]
async fn flush(args: Vec<String>) -> Result<(), FrameworkError> {
    let (index, _) = parse_args("search:flush", &args, true, true, false)?;
    if let Some(index) = index {
        generations::flush_index(index).await?;
    }
    Ok(())
}

#[command(name = "search:index", description = "Create a declared search index")]
async fn create(args: Vec<String>) -> Result<(), FrameworkError> {
    let (index, _) = parse_args("search:index", &args, true, false, false)?;
    if let Some(index) = index {
        generations::create_index(index).await?;
    }
    Ok(())
}

#[command(
    name = "search:delete-index",
    description = "Delete a declared search index"
)]
async fn delete(args: Vec<String>) -> Result<(), FrameworkError> {
    let (index, _) = parse_args("search:delete-index", &args, true, false, false)?;
    if let Some(index) = index {
        generations::delete_index(index).await?;
    }
    Ok(())
}

#[command(
    name = "search:sync-index-settings",
    description = "Apply code-declared search settings"
)]
async fn sync(args: Vec<String>) -> Result<(), FrameworkError> {
    let (index, _) = parse_args("search:sync-index-settings", &args, false, false, false)?;
    let indexes = match index {
        Some(index) => vec![index],
        None => SearchRegistry::indexes()?,
    };
    for index in indexes {
        generations::sync_settings(index).await?;
    }
    Ok(())
}

#[command(
    name = "search:reindex",
    description = "Build and publish a complete search generation"
)]
async fn reindex(args: Vec<String>) -> Result<(), FrameworkError> {
    let (index, _) = parse_args("search:reindex", &args, true, true, false)?;
    if let Some(index) = index {
        generations::reindex(index).await?;
    }
    Ok(())
}

#[command(
    name = "search:status",
    description = "Report search generations and optionally prune tombstones"
)]
async fn status(args: Vec<String>) -> Result<(), FrameworkError> {
    let (index, prune) = parse_args("search:status", &args, false, false, true)?;
    let indexes = match index {
        Some(index) => vec![index],
        None => SearchRegistry::indexes()?,
    };
    if indexes.is_empty() && prune {
        generations::prune_tombstones().await?;
    }
    for index in indexes {
        generations::status(index, prune).await?;
    }
    Ok(())
}

#[suprnova_macros::command(
    name = "search:baseline",
    description = "Calibrate rival relevance floors or run warm engine baselines"
)]
async fn search_baseline(args: Vec<String>) -> Result<(), FrameworkError> {
    let (action, engines) = arguments(&args)?;
    let directory = fixture_directory();
    if action == "calibrate" {
        let services = Calibration::from_env()?;
        let tolerance = std::env::var("SEARCH_BASELINE_TOLERANCE").map_or(Ok(0.05), |s| {
            s.parse::<f64>()
                .map_err(|_| FrameworkError::param("SEARCH_BASELINE_TOLERANCE: requires a number"))
        })?;
        let fixture = Fixture::load(&directory)?;
        let floors = BaselineFloors::load(directory.join("baseline.json"))?;
        let calibrated = services.run(&fixture, &floors, tolerance).await?;
        calibrated.write(directory.join("baseline.json"))?;
        crate::console::line(
            "Calibrated framework/tests/search/fixtures/baseline.json - review and commit the replacement floors.",
        );
        return Ok(());
    }
    let baseline = Baseline::load(directory)?;
    let config = SearchConfig::current()?;
    let embedder = if config.embedder == SearchEmbedder::None {
        None
    } else {
        super::boot::boot().await?;
        Some(SearchRegistry::embedder_named(
            "search_baseline",
            config.embedder.as_str(),
        )?)
    };
    let allow = std::env::var("SEARCH_BASELINE_ALLOW_UNCALIBRATED").as_deref() == Ok("1");
    let mut failures = Vec::new();
    for engine in engines {
        let result = if engine == "collection" {
            baseline
                .run(&engine, &CollectionEngine::new(), embedder.as_deref())
                .await
        } else {
            run_persistent(&baseline, &engine, embedder.as_deref()).await
        };
        let record = result?;
        let output = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../target/search-baseline/{engine}.json"));
        record.write(output)?;
        baseline.print(&record)?;
        if let Err(error) = baseline.assert_record(&record, allow) {
            failures.push(error.to_string());
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(FrameworkError::param(failures.join("; ")))
    }
}

fn arguments(args: &[String]) -> Result<(String, Vec<String>), FrameworkError> {
    let Some(action) = args
        .first()
        .filter(|a| matches!(a.as_str(), "run" | "calibrate"))
    else {
        return Err(FrameworkError::param(
            "search:baseline requires calibrate or run [--engine collection|sqlite|postgres|mysql|qdrant]",
        ));
    };
    if action == "calibrate" {
        if args.len() != 1 {
            return Err(FrameworkError::param(
                "search:baseline calibrate takes no arguments",
            ));
        }
        return Ok((action.clone(), vec![]));
    }
    let engine = match &args[1..] {
        [] => None,
        [flag, value] if flag == "--engine" => Some(value.as_str()),
        [arg] => arg.strip_prefix("--engine="),
        _ => {
            return Err(FrameworkError::param(
                "search:baseline run accepts one --engine option",
            ));
        }
    };
    if !args[1..].is_empty() && engine.is_none() {
        return Err(FrameworkError::param(
            "search:baseline run: invalid --engine option",
        ));
    }
    let engines = match engine {
        None => vec!["collection".into(), "sqlite".into()],
        Some(engine)
            if matches!(
                engine,
                "collection" | "sqlite" | "postgres" | "mysql" | "qdrant"
            ) =>
        {
            vec![engine.into()]
        }
        Some(_) => {
            return Err(FrameworkError::param(
                "search:baseline --engine must be collection, sqlite, postgres, mysql or qdrant",
            ));
        }
    };
    Ok((action.clone(), engines))
}
async fn run_persistent(
    baseline: &Baseline,
    engine: &str,
    embedder: Option<&dyn super::Embedder>,
) -> Result<super::baseline::BaselineRecord, FrameworkError> {
    let variable = match engine {
        "sqlite" | "qdrant" => None,
        "postgres" => Some("PG_TEST_URL"),
        "mysql" => Some("MYSQL_TEST_URL"),
        _ => {
            return Err(FrameworkError::param(
                "search baseline: invalid persistent engine",
            ));
        }
    };
    let url = variable
        .map(crate::config::try_env_required::<String>)
        .transpose()?
        .unwrap_or_else(|| "sqlite::memory:".into());
    if url.trim().is_empty() {
        return Err(FrameworkError::param(format!(
            "{}: requires a disposable baseline database URL",
            variable.unwrap_or("DATABASE_URL")
        )));
    }
    let name = format!("baseline_{}", uuid::Uuid::new_v4().simple());
    DB::register_named(
        &name,
        DatabaseConfig::builder()
            .url(url)
            .min_connections(1)
            .max_connections(1)
            .idle_timeout(0)
            .max_lifetime(0)
            .logging(false)
            .build(),
    )
    .await
    .map_err(|_| {
        FrameworkError::database(format!(
            "search baseline {engine}: cannot connect to the disposable database"
        ))
    })?;
    DB::with_default_connection(name, async {
        let connection = DB::connection()?;
        let backend = connection.get_database_backend();
        if matches!(
            (engine, backend),
            (
                "sqlite",
                sea_orm::DbBackend::Postgres | sea_orm::DbBackend::MySql
            ) | (
                "postgres",
                sea_orm::DbBackend::Sqlite | sea_orm::DbBackend::MySql
            ) | (
                "mysql",
                sea_orm::DbBackend::Sqlite | sea_orm::DbBackend::Postgres
            )
        ) {
            return Err(FrameworkError::param(format!(
                "search baseline {engine}: database URL selects the wrong engine"
            )));
        }
        let manager = sea_orm_migration::SchemaManager::new(connection.inner());
        for migration in super::migrations() {
            migration.up(&manager).await.map_err(|e| {
                FrameworkError::database(format!("search baseline migrations: {e}"))
            })?;
        }
        if engine == "qdrant" {
            let driver = QdrantSearchEngine::from_env()?;
            baseline.run(engine, &driver, embedder).await
        } else {
            baseline
                .run(engine, &DatabaseSearchEngine::new(), embedder)
                .await
        }
    })
    .await
}

#[command(
    name = "search:analytics",
    description = "Report top queries, missing queries and click positions [--index name] [--days days]"
)]
async fn analytics(args: Vec<String>) -> Result<(), FrameworkError> {
    let mut index = None;
    let mut days = None;
    let mut arguments = args.iter();
    while let Some(argument) = arguments.next() {
        let (name, value) = match argument.split_once('=') {
            Some(pair) => pair,
            None => (
                argument.as_str(),
                arguments
                    .next()
                    .ok_or_else(|| FrameworkError::param(format!("{argument} requires a value")))?
                    .as_str(),
            ),
        };
        match name {
            "--index" if index.is_none() => index = Some(value),
            "--days" if days.is_none() => {
                days = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| FrameworkError::param("analytics --days must be positive"))?,
                )
            }
            _ => {
                return Err(FrameworkError::param(format!(
                    "search:analytics unknown or repeated option '{name}'"
                )));
            }
        }
    }
    let report = super::analytics::report(index, days.unwrap_or(30)).await?;
    crate::console::line(
        serde_json::to_string(&report).map_err(|error| {
            FrameworkError::internal(format!("search analytics report: {error}"))
        })?,
    );
    Ok(())
}

#[command(
    name = "search:analytics-prune",
    description = "Prune expired search analytics and removal tombstones"
)]
async fn analytics_prune(args: Vec<String>) -> Result<(), FrameworkError> {
    if !args.is_empty() {
        return Err(FrameworkError::param(
            "search:analytics-prune accepts no parameters",
        ));
    }
    crate::console::line(format!(
        "search:analytics-prune - removed {} rows",
        super::analytics::prune().await?
    ));
    Ok(())
}
