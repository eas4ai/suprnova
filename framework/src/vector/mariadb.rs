//! Phase 9B - MariaDB vector driver via direct `sqlx`.
//!
//! Talks to MariaDB over its MySQL-compatible wire protocol. A thin adapter
//! that satisfies [`VectorDriver`] while preserving the framework's `String`
//! IDs and `serde_json::Value` payload contract.
//!
//! # MariaDB version requirement
//!
//! `VECTOR(N)` and `VEC_DISTANCE_*` builtins land in **MariaDB 11.7+**. The
//! driver runs a `SELECT VERSION()` on first use and rejects anything older
//! with [`FrameworkError::internal`]. The result is cached in a
//! [`tokio::sync::OnceCell`] - *definitive* outcomes (verified ≥ 11.7, or
//! a parsed pre-11.7 / non-MariaDB version string) stick across calls.
//! Transient query failures (lazy-connect first-dial timeout, brief
//! network blip, restart-induced auth gap) are NOT cached, so the next
//! call gets a fresh shot once the underlying issue clears.
//!
//! # Schema convention
//!
//! Each store maps to one table with three columns:
//!
//! ```sql
//! CREATE TABLE `<store>` (
//!     id        VARBINARY(254) NOT NULL PRIMARY KEY,
//!     embedding VECTOR(<N>)    NOT NULL,
//!     metadata  JSON           NULL,
//!     VECTOR INDEX (embedding) DISTANCE=<euclidean|cosine>
//! ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
//! ```
//!
//! MariaDB refuses a vector index on a table whose primary key is longer
//! than 256 bytes, counting the two length bytes, so the id is at most
//! 254 bytes. A `utf8mb4` `VARCHAR(255)` key would be 1020 bytes and the
//! server rejects the `CREATE TABLE`. `VARBINARY` also compares ids byte
//! for byte, the way every other driver keys them: the table's default
//! collation would treat `Doc` and `doc` as one row.
//!
//! Use [`MariaDbVectorDriver::ensure_table_sql`] to emit this string -
//! paste it into your migration. **The driver does not auto-create
//! tables**: schema is a migration concern, not a runtime side-effect.
//!
//! The index's `DISTANCE=` clause must match the function the driver
//! uses at query time (set via [`MariaDbVectorDriver::with_distance`]) -
//! mismatched pairs silently fall back to a full table scan per the
//! MariaDB docs. The recommended path is
//! [`MariaDbVectorDriver::ensure_table_sql_for`], which reads
//! `self.distance` so the migration SQL and the query function come
//! from the same source. The static
//! [`MariaDbVectorDriver::ensure_table_sql`] is retained for migration
//! generators that haven't built a driver yet, but the caller is
//! responsible for passing the same distance to both ends.
//!
//! # ID mapping - there is none
//!
//! The id column holds any UTF-8 string up to 254 bytes. [`VectorItem::id`]
//! passes through unchanged; similarity hits round-trip the same string in
//! [`VectorMatch::id`]. No reserved payload keys, no derived UUIDs.
//!
//! # Store-name validation
//!
//! Store names interpolate directly into `CREATE`/`INSERT`/`SELECT`
//! statements (sqlx does not parameterize identifiers). The driver
//! validates names through [`MariaDbVectorDriver::validate_store_name`]
//! at every entry point - empty / too long / non-`[A-Za-z_][A-Za-z0-9_]*`
//! names are rejected with [`FrameworkError::param`]. All emitted SQL
//! backtick-quotes the validated name for defense in depth.
//!
//! # Score normalization
//!
//! MariaDB returns *distance* (lower = closer). The trait contract is
//! *score* (higher = more similar). Conversions match the source-verified
//! ranges:
//!
//! | Metric    | MariaDB returns       | We expose as `score`         |
//! | --------- | --------------------- | ---------------------------- |
//! | Cosine    | `[0, 2]` (`1 - cos`)  | `1.0 - d / 2.0` → `[0, 1]`   |
//! | Euclidean | `[0, ∞)` L2 norm      | `1.0 / (1.0 + d)` → `(0, 1]` |
//!
//! See [`MariaDbVectorDriver::score_from_distance`].
//!
//! # Trapdoor
//!
//! When you outgrow the trait surface - additional columns on the same
//! table, raw `VEC_TOTEXT` reads, batched joins - drop down to
//! [`MariaDbVectorDriver::pool`] for the underlying `sqlx::MySqlPool`.
//! The [`MariaDbVectorDriver::embedding_to_vec_text`],
//! [`MariaDbVectorDriver::score_from_distance`] and
//! [`MariaDbVectorDriver::ensure_table_sql`] pure-fn helpers stay
//! consistent with the framework's encoding when mixing direct and
//! trait-routed calls.

use super::driver::{VectorDriver, VectorItem, VectorMatch};
use crate::FrameworkError;
use async_trait::async_trait;
use sqlx::Row;
use sqlx::mysql::{MySqlPool, MySqlPoolOptions};
use std::collections::HashSet;
use std::fmt::Write;
use std::sync::Arc;
use tokio::sync::{OnceCell, RwLock};

/// IN-list size beyond which `delete` splits the call into multiple
/// `DELETE ... WHERE id IN (...)` statements wrapped in one
/// transaction. Keeps any single statement well below MariaDB's
/// `max_allowed_packet` even when deleting millions of ids at once.
const DELETE_BATCH_SIZE: usize = 1000;

/// Row count per multi-row `INSERT ... VALUES (...), (...), ...`
/// statement in `upsert`. 500 rows × 3 placeholders each = 1500
/// placeholders per statement, comfortably under MySQL's 65535
/// placeholder cap and below the per-packet ceiling for any
/// reasonable `max_allowed_packet` config. All chunks run inside one
/// transaction so the whole call is atomic.
const UPSERT_BATCH_SIZE: usize = 500;

/// Build the `VALUES (?, VEC_FROMTEXT(?), ?), (...) ...` clause used
/// by the batched upsert. Extracted for pure-fn testability - the
/// template is entirely framework-controlled (no user input), so the
/// only failure mode is row_count mismatch with caller binds, which
/// the unit tests pin.
fn upsert_values_clause(row_count: usize) -> String {
    std::iter::repeat_n("(?, VEC_FROMTEXT(?), ?)", row_count)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Distance metric for the MariaDB vector index. Picks both the
/// `DISTANCE=` clause emitted by [`MariaDbVectorDriver::ensure_table_sql`]
/// and the `VEC_DISTANCE_*` function used in `similar`.
///
/// Default: `Cosine` (matches the in-process Memory driver and most
/// embedding model conventions). MariaDB's own default is `Euclidean`,
/// which the framework overrides to keep the user-facing contract
/// consistent across drivers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MariaDbDistance {
    /// Cosine distance - `1 - cos(θ)`, range `[0, 2]`.
    #[default]
    Cosine,
    /// Euclidean (L2) distance, range `[0, ∞)`.
    Euclidean,
}

impl MariaDbDistance {
    /// The literal that follows `DISTANCE=` in the index definition.
    /// Lowercase per MariaDB's own examples.
    pub fn index_clause(self) -> &'static str {
        match self {
            MariaDbDistance::Cosine => "cosine",
            MariaDbDistance::Euclidean => "euclidean",
        }
    }

    /// The `VEC_DISTANCE_*` function used in `SELECT` queries. Must
    /// match the index's `DISTANCE=` clause or MariaDB falls back to a
    /// full table scan.
    pub fn fn_name(self) -> &'static str {
        match self {
            MariaDbDistance::Cosine => "VEC_DISTANCE_COSINE",
            MariaDbDistance::Euclidean => "VEC_DISTANCE_EUCLIDEAN",
        }
    }
}

/// MariaDB-backed [`VectorDriver`].
pub struct MariaDbVectorDriver {
    pool: Arc<MySqlPool>,
    distance: MariaDbDistance,
    /// Cached `SELECT VERSION()` result. Populated lazily on first use:
    /// `Ok(())` once verified ≥ 11.7; `Err(msg)` once a pre-11.7 server
    /// (or non-MariaDB) is *definitively* detected (server responded
    /// with a string we could parse). Definitive results stick - we
    /// don't keep hammering the server.
    ///
    /// Transient *query* failures (connection refused, timeout,
    /// auth blip during a lazy-connect first dial) are NOT cached;
    /// they bubble out of [`ensure_version`] and let the next call
    /// re-attempt. Implemented via `get_or_try_init`.
    ///
    /// [`ensure_version`]: Self::ensure_version
    version_check: Arc<OnceCell<Result<(), String>>>,
    /// Memo of store names whose `VECTOR INDEX ... DISTANCE=<clause>`
    /// has been verified to match `self.distance`. Populated by
    /// [`ensure_store_distance`] on the first `similar` call against
    /// each store, so the `SHOW CREATE TABLE` round-trip only fires
    /// once per (driver, store) pair.
    ///
    /// [`ensure_store_distance`]: Self::ensure_store_distance
    verified_stores: Arc<RwLock<HashSet<String>>>,
}

impl MariaDbVectorDriver {
    /// Wrap an already-built `MySqlPool`. Use this when you need
    /// custom pool options (max_connections, acquire_timeout, etc.).
    pub fn from_pool(pool: MySqlPool) -> Self {
        Self {
            pool: Arc::new(pool),
            distance: MariaDbDistance::default(),
            version_check: Arc::new(OnceCell::new()),
            verified_stores: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// Build a lazy-connect pool from a MySQL/MariaDB URL. The pool
    /// validates URL syntax but does NOT open a connection until the
    /// first query - so `Vector::register(...)` at bootstrap is safe
    /// even before the database is reachable.
    ///
    /// First operation absorbs the connection cost; if the URL points
    /// nowhere, that operation surfaces the failure, not registration.
    ///
    /// The URL may use the `mysql://` or the `mariadb://` scheme.
    pub fn from_url(url: &str) -> Result<Self, FrameworkError> {
        let pool = MySqlPoolOptions::new()
            .connect_lazy(crate::database::config::driver_url(url).as_ref())
            // The URL carries the password, so the error names the fault
            // and not the URL.
            .map_err(|e| FrameworkError::internal(format!("mariadb pool init: {e}")))?;
        Ok(Self::from_pool(pool))
    }

    /// Construct from the environment:
    ///
    /// - `MARIADB_URL` - the URL of the MariaDB the vectors are in
    /// - `DATABASE_URL` - read when `MARIADB_URL` is not set, and taken
    ///   when it names a MariaDB or a MySQL database
    ///
    /// The fallback is for the application whose one database is a
    /// MariaDB: its rows and its vectors are in one engine, and it has
    /// one URL to set. A `DATABASE_URL` of another engine is not taken.
    /// The driver would accept it and fail on the first query, with an
    /// error that says nothing about a variable.
    ///
    /// A URL with the scheme `mysql` is taken, because a MariaDB is
    /// written that way as well. When it is the URL of a MySQL server,
    /// the driver fails on its first query: MySQL has no vector
    /// functions.
    ///
    /// # Errors
    ///
    /// When neither variable gives a URL of a MariaDB or a MySQL
    /// database, and when the pool cannot be built from the URL. The
    /// error names the variables and never the URL, which carries the
    /// password.
    pub fn from_env() -> Result<Self, FrameworkError> {
        Self::from_variables(|name| std::env::var(name).ok())
    }

    /// [`Self::from_env`] with the variables looked up by `variable`.
    fn from_variables(variable: impl Fn(&str) -> Option<String>) -> Result<Self, FrameworkError> {
        let set = |name: &str| {
            variable(name)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        if let Some(url) = set("MARIADB_URL") {
            return Self::from_url(&url);
        }
        match set("DATABASE_URL") {
            Some(url) if names_mariadb(&url) => Self::from_url(&url),
            Some(_) => Err(FrameworkError::param(
                "MARIADB_URL is not set, and DATABASE_URL names a database that is no MariaDB \
                 and no MySQL; set MARIADB_URL for the MariaDB vector driver",
            )),
            None => Err(FrameworkError::param(
                "MARIADB_URL is not set; the MariaDB vector driver needs it, or a DATABASE_URL \
                 that names a MariaDB",
            )),
        }
    }

    /// Set the distance metric. Default is [`MariaDbDistance::Cosine`].
    pub fn with_distance(mut self, d: MariaDbDistance) -> Self {
        self.distance = d;
        self
    }

    /// Borrow the underlying `sqlx::MySqlPool`. Use this for raw
    /// queries that the trait surface doesn't cover.
    pub fn pool(&self) -> &MySqlPool {
        &self.pool
    }

    /// The driver's configured distance metric.
    pub fn distance(&self) -> MariaDbDistance {
        self.distance
    }

    /// Validate a store name against the MariaDB identifier rules the
    /// driver requires. Accepts `[A-Za-z_][A-Za-z0-9_]*` of length ≤ 64
    /// (the InnoDB table-name limit is wider, but we constrain further
    /// for safety + predictability across MySQL/MariaDB variants).
    ///
    /// Returns the validated `&str` unchanged so callers can chain.
    pub fn validate_store_name(name: &str) -> Result<&str, FrameworkError> {
        if name.is_empty() {
            return Err(FrameworkError::param(
                "mariadb vector store name cannot be empty",
            ));
        }
        if name.len() > 64 {
            return Err(FrameworkError::param(format!(
                "mariadb vector store name '{name}' exceeds 64 characters"
            )));
        }
        let mut chars = name.chars();
        let first = chars.next().expect("non-empty checked above");
        if !(first.is_ascii_alphabetic() || first == '_') {
            return Err(FrameworkError::param(format!(
                "mariadb vector store name '{name}' must start with a letter or '_'"
            )));
        }
        for c in chars {
            if !(c.is_ascii_alphanumeric() || c == '_') {
                return Err(FrameworkError::param(format!(
                    "mariadb vector store name '{name}' contains invalid character '{c}' \
                     (allowed: A-Z a-z 0-9 _)"
                )));
            }
        }
        Ok(name)
    }

    /// Emit the `CREATE TABLE IF NOT EXISTS` SQL for a store *using
    /// this driver's configured distance*. The recommended call when
    /// the driver is already constructed - guarantees the index's
    /// `DISTANCE=` clause matches the function `similar` will use,
    /// because both read from `self.distance`.
    ///
    /// Errors if `table` fails [`validate_store_name`] or `dim` is 0.
    ///
    /// [`validate_store_name`]: Self::validate_store_name
    pub fn ensure_table_sql_for(&self, table: &str, dim: usize) -> Result<String, FrameworkError> {
        Self::ensure_table_sql(table, dim, self.distance)
    }

    /// Static form of [`ensure_table_sql_for`] - emits the
    /// `CREATE TABLE IF NOT EXISTS` SQL when you don't have a driver
    /// in scope (CLI migration generators, build scripts). The
    /// **caller is responsible** for passing the same `MariaDbDistance`
    /// value they'll later use via [`with_distance`] - MariaDB silently
    /// falls back to a full table scan when the function used at query
    /// time doesn't match the index's `DISTANCE=` clause. Prefer
    /// [`ensure_table_sql_for`] in code paths where a driver is
    /// already constructed.
    ///
    /// Errors if `table` fails [`validate_store_name`] or `dim` is 0.
    ///
    /// [`ensure_table_sql_for`]: Self::ensure_table_sql_for
    /// [`with_distance`]: Self::with_distance
    /// [`validate_store_name`]: Self::validate_store_name
    pub fn ensure_table_sql(
        table: &str,
        dim: usize,
        distance: MariaDbDistance,
    ) -> Result<String, FrameworkError> {
        Self::validate_store_name(table)?;
        if dim == 0 {
            return Err(FrameworkError::param(
                "mariadb vector dim must be greater than 0",
            ));
        }
        Ok(format!(
            "CREATE TABLE IF NOT EXISTS `{table}` (\n  \
             id VARBINARY(254) NOT NULL PRIMARY KEY,\n  \
             embedding VECTOR({dim}) NOT NULL,\n  \
             metadata JSON NULL,\n  \
             VECTOR INDEX (embedding) DISTANCE={dist}\n\
             ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
            dist = distance.index_clause()
        ))
    }

    /// Format an embedding as the JSON-array text form
    /// `VEC_FROMTEXT` accepts: `[1,2,3.5,-0.7]`.
    ///
    /// Errors if the embedding is empty or contains any non-finite
    /// value (`NaN`, `±Infinity`) - JSON does not represent those
    /// and MariaDB would reject them anyway. Finite floats serialize
    /// via `f32`'s `Display` impl, which produces the shortest
    /// round-trippable JSON-number representation.
    pub fn embedding_to_vec_text(v: &[f32]) -> Result<String, FrameworkError> {
        if v.is_empty() {
            return Err(FrameworkError::param(
                "mariadb vector embedding cannot be empty",
            ));
        }
        let mut s = String::with_capacity(v.len() * 8);
        s.push('[');
        for (i, f) in v.iter().enumerate() {
            if !f.is_finite() {
                return Err(FrameworkError::param(format!(
                    "mariadb vector embedding contains non-finite value at index {i}: {f}"
                )));
            }
            if i > 0 {
                s.push(',');
            }
            write!(&mut s, "{f}").expect("write to String never fails");
        }
        s.push(']');
        Ok(s)
    }

    /// Convert a MariaDB raw distance to a uniform `[0, 1]` similarity
    /// score where higher = more similar. See the module docs for the
    /// per-metric formula.
    ///
    /// Negative inputs (which shouldn't happen - distances are
    /// non-negative - but float arithmetic can dip below zero) are
    /// clamped to 0 before conversion.
    pub fn score_from_distance(distance: MariaDbDistance, raw: f32) -> f32 {
        let d = raw.max(0.0);
        match distance {
            MariaDbDistance::Cosine => (1.0 - d / 2.0).clamp(0.0, 1.0),
            MariaDbDistance::Euclidean => (1.0 / (1.0 + d)).clamp(0.0, 1.0),
        }
    }

    /// Run the `SELECT VERSION()` check at most once per driver per
    /// successful execution path. The cell stores the *definitive*
    /// result - `Ok(())` once verified ≥ 11.7, `Err(msg)` once a
    /// pre-11.7 server (or non-MariaDB) is identified - and subsequent
    /// calls short-circuit through it.
    ///
    /// **Transient failures are not cached.** If the `VERSION()` query
    /// itself fails (connection refused, timeout, auth blip during a
    /// lazy-connect first dial), the error bubbles out without touching
    /// the cell, so the next call gets a fresh attempt once the
    /// underlying issue clears. This is implemented via
    /// `get_or_try_init`, which only writes to the cell when its init
    /// closure returns `Ok`.
    async fn ensure_version(&self) -> Result<(), FrameworkError> {
        let pool = Arc::clone(&self.pool);
        let cached: &Result<(), String> = self
            .version_check
            .get_or_try_init(|| async move {
                let row: (String,) = sqlx::query_as("SELECT VERSION()")
                    .fetch_one(&*pool)
                    .await
                    .map_err(|e| {
                        // Transient query failure - bubble out so the
                        // next call retries. Not cached.
                        FrameworkError::internal(format!("mariadb VERSION() query failed: {e}"))
                    })?;
                // Server responded - the version check itself is
                // deterministic and its result IS cached, sticky.
                Ok::<_, FrameworkError>(check_mariadb_117(&row.0))
            })
            .await?;
        match cached {
            Ok(()) => Ok(()),
            Err(msg) => Err(FrameworkError::internal(msg.clone())),
        }
    }

    /// Verify the distance of the table's vector index matches
    /// `self.distance`. MariaDB does not error when a `VEC_DISTANCE_*`
    /// function disagrees with the index's distance - it silently falls
    /// back to a full table scan, leaving users with mysteriously slow
    /// queries. This check catches the mismatch at the framework
    /// boundary so the error surfaces clearly instead of as a
    /// production perf cliff.
    ///
    /// One `SHOW CREATE TABLE` runs per (driver, store) pair on first
    /// `similar` call; the result is cached in `verified_stores` so
    /// every subsequent call is zero-cost. Other methods (`upsert`,
    /// `delete`, `count`) do not engage the vector index and so do not
    /// trigger the check.
    ///
    /// The caller must have already validated `store` via
    /// [`validate_store_name`] - the name is interpolated into the
    /// emitted SQL.
    ///
    /// [`validate_store_name`]: Self::validate_store_name
    async fn ensure_store_distance(&self, store: &str) -> Result<(), FrameworkError> {
        if self.verified_stores.read().await.contains(store) {
            return Ok(());
        }

        let sql = format!("SHOW CREATE TABLE `{store}`");
        let row: (String, String) = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .fetch_one(&*self.pool)
            .await
            .map_err(|e| {
                FrameworkError::internal(format!(
                    "mariadb: SHOW CREATE TABLE for store '{store}' failed: {e}"
                ))
            })?;
        let ddl = &row.1;

        let table_distance = match extract_vector_index_distance(ddl) {
            Some(d) => d,
            None => {
                return Err(FrameworkError::internal(format!(
                    "mariadb vector store '{store}' has no VECTOR INDEX - run \
                     ensure_table_sql_for to emit the canonical CREATE TABLE, \
                     or add `VECTOR INDEX (embedding) DISTANCE={}` to the \
                     table definition.",
                    self.distance.index_clause()
                )));
            }
        };

        let expected = self.distance.index_clause();
        if table_distance != expected {
            return Err(FrameworkError::internal(format!(
                "mariadb vector store '{store}' has VECTOR INDEX DISTANCE={table_distance} \
                 but the driver is configured with_distance({:?}). Mismatched distances \
                 silently fall back to a full table scan - rebuild the table with \
                 DISTANCE={expected} or reconfigure the driver.",
                self.distance
            )));
        }

        self.verified_stores.write().await.insert(store.to_string());
        Ok(())
    }
}

/// Extract the distance of the vector index from `SHOW CREATE TABLE`
/// output. Returns the value in lower case when the index names one;
/// `"euclidean"` when it names none (MariaDB's default); `None` when the
/// table has no vector index at all.
///
/// The server does not print the DDL it was given. MariaDB 11.7 and later
/// print the index as ``VECTOR KEY `name` (`col`)`` and its options as
/// backtick-quoted names, `` `distance`=cosine `` in 11.7 and
/// `` `DISTANCE`=cosine `` in 12. The parse takes the line that starts
/// with `VECTOR KEY` or `VECTOR INDEX` (the second is the DDL
/// [`MariaDbVectorDriver::ensure_table_sql`] writes), drops the backticks,
/// ignores case and reads the `distance=` option. A column line cannot
/// match: the server quotes every column name, so such a line starts with
/// a backtick.
fn extract_vector_index_distance(ddl: &str) -> Option<String> {
    for line in ddl.lines() {
        let line = line.trim().to_ascii_lowercase();
        let is_vector_index = ["vector key", "vector index"].iter().any(|keyword| {
            line.strip_prefix(keyword)
                .is_some_and(|rest| rest.starts_with([' ', '(', '`']))
        });
        if !is_vector_index {
            continue;
        }
        let options = line.replace('`', "").replace(" =", "=").replace("= ", "=");
        let distance = options.split_whitespace().find_map(|token| {
            token
                .strip_prefix("distance=")
                .map(|value| value.trim_end_matches([',', ')']).to_owned())
        });
        return Some(distance.unwrap_or_else(|| "euclidean".to_owned()));
    }
    None
}

/// Parse a MariaDB `VERSION()` string and confirm it advertises ≥ 11.7.
///
/// MariaDB embeds a legacy `5.5.5-` prefix in many connector contexts
/// (so MySQL clients see a "fake" 5.5.5 first), with the real version
/// after - e.g. `"5.5.5-11.7.2-MariaDB-1:11.7.2+maria~ubu2404"`. We
/// strip that prefix when present, then parse `MAJOR.MINOR.*` from the
/// remainder. Anything without "MariaDB" anywhere in the string is
/// rejected to keep the driver from running against a regular MySQL.
fn check_mariadb_117(version: &str) -> Result<(), String> {
    if !version.contains("MariaDB") {
        return Err(format!(
            "mariadb vector driver requires MariaDB 11.7+; \
             VERSION() returned '{version}' (no 'MariaDB' marker - \
             is this a MySQL server?)"
        ));
    }
    let stripped = version.strip_prefix("5.5.5-").unwrap_or(version);
    let version_part = stripped.split('-').next().unwrap_or("");
    let mut parts = version_part.split('.');
    let major: u32 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(|| {
        format!("mariadb vector driver: couldn't parse major version from '{version}'")
    })?;
    let minor: u32 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(|| {
        format!("mariadb vector driver: couldn't parse minor version from '{version}'")
    })?;
    if major > 11 || (major == 11 && minor >= 7) {
        Ok(())
    } else {
        Err(format!(
            "mariadb vector driver requires MariaDB 11.7+; found {major}.{minor} in '{version}'"
        ))
    }
}

/// Validate that a [`VectorItem`]'s metadata is either a JSON object
/// or `null` - the only shapes the schema's `JSON` column accepts in a
/// way that round-trips through the driver. Other JSON kinds (arrays,
/// primitives) are rejected with `FrameworkError::param` for parity
/// with the Qdrant and Pinecone drivers.
fn validate_metadata(item: &VectorItem) -> Result<(), FrameworkError> {
    match &item.metadata {
        serde_json::Value::Object(_) | serde_json::Value::Null => Ok(()),
        other => Err(FrameworkError::param(format!(
            "mariadb vector item '{}' metadata must be a JSON object or null, got: {}",
            item.id, other
        ))),
    }
}

#[async_trait]
impl VectorDriver for MariaDbVectorDriver {
    async fn upsert(&self, store: &str, items: Vec<VectorItem>) -> Result<(), FrameworkError> {
        if items.is_empty() {
            return Ok(());
        }
        let table = MariaDbVectorDriver::validate_store_name(store)?;
        for item in &items {
            validate_metadata(item)?;
        }
        self.ensure_version().await?;

        // Batched multi-row INSERT - one statement per chunk of
        // `UPSERT_BATCH_SIZE` rows, all wrapped in a single transaction.
        // Cuts round-trips by ~500x vs per-row INSERTs on bulk loads
        // (1536-dim embedding corpora hitting 100k+ rows). Any per-batch
        // failure rolls back every preceding batch - the whole call
        // remains atomic.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| FrameworkError::internal(format!("mariadb upsert begin tx: {e}")))?;

        let mut iter = items.into_iter();
        loop {
            let chunk: Vec<VectorItem> = iter.by_ref().take(UPSERT_BATCH_SIZE).collect();
            if chunk.is_empty() {
                break;
            }
            let row_count = chunk.len();

            let sql = format!(
                "INSERT INTO `{table}` (id, embedding, metadata) VALUES {values} \
                 ON DUPLICATE KEY UPDATE \
                 embedding = VALUES(embedding), \
                 metadata = VALUES(metadata)",
                values = upsert_values_clause(row_count)
            );

            let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
            for item in chunk {
                let vec_text = MariaDbVectorDriver::embedding_to_vec_text(&item.embedding)?;
                let metadata = match item.metadata {
                    serde_json::Value::Null => None,
                    v => Some(v),
                };
                q = q.bind(item.id).bind(vec_text).bind(metadata);
            }

            q.execute(&mut *tx).await.map_err(|e| {
                FrameworkError::internal(format!(
                    "mariadb upsert chunk ({row_count} rows) failed for store '{table}': {e}"
                ))
            })?;
        }

        tx.commit()
            .await
            .map_err(|e| FrameworkError::internal(format!("mariadb upsert commit: {e}")))?;
        Ok(())
    }

    async fn similar(
        &self,
        store: &str,
        query: Vec<f32>,
        k: usize,
    ) -> Result<Vec<VectorMatch>, FrameworkError> {
        if k == 0 || query.is_empty() {
            return Ok(Vec::new());
        }
        if query.iter().all(|&f| f == 0.0) {
            return Err(FrameworkError::param(
                "mariadb vector::similar query is zero-vector",
            ));
        }
        let table = MariaDbVectorDriver::validate_store_name(store)?;
        self.ensure_version().await?;
        self.ensure_store_distance(table).await?;

        let vec_text = MariaDbVectorDriver::embedding_to_vec_text(&query)?;
        let sql = format!(
            "SELECT id, metadata, {fn_name}(embedding, VEC_FROMTEXT(?)) AS dist \
             FROM `{table}` \
             ORDER BY dist ASC \
             LIMIT ?",
            fn_name = self.distance.fn_name()
        );

        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&vec_text)
            .bind(k as u64)
            .fetch_all(&*self.pool)
            .await
            .map_err(|e| {
                FrameworkError::internal(format!("mariadb similar failed for store '{table}': {e}"))
            })?;

        let mut matches = Vec::with_capacity(rows.len());
        for row in rows {
            // The id column is `VARBINARY`, which the driver hands back as
            // bytes; a table created with a text id reads the same way.
            let id: Vec<u8> = row.try_get("id").map_err(|e| {
                FrameworkError::internal(format!("mariadb similar: decode id column: {e}"))
            })?;
            let id = String::from_utf8(id).map_err(|e| {
                FrameworkError::internal(format!("mariadb similar: id is not UTF-8: {e}"))
            })?;
            let metadata: Option<serde_json::Value> = row.try_get("metadata").map_err(|e| {
                FrameworkError::internal(format!(
                    "mariadb similar: decode metadata column for id '{id}': {e}"
                ))
            })?;
            // `dist` comes back as f32 from MariaDB's vector functions.
            let dist: f32 = row.try_get("dist").map_err(|e| {
                FrameworkError::internal(format!(
                    "mariadb similar: decode dist column for id '{id}': {e}"
                ))
            })?;
            matches.push(VectorMatch {
                id,
                score: MariaDbVectorDriver::score_from_distance(self.distance, dist),
                metadata: metadata.unwrap_or(serde_json::Value::Null),
            });
        }
        Ok(matches)
    }

    async fn delete(&self, store: &str, ids: Vec<String>) -> Result<(), FrameworkError> {
        if ids.is_empty() {
            return Ok(());
        }
        let table = MariaDbVectorDriver::validate_store_name(store)?;
        self.ensure_version().await?;

        // Chunk the IN-list so a single DELETE statement never exceeds
        // MariaDB's `max_allowed_packet` (default 64 MiB; reasonable
        // deployments tune it down to 16 MiB). 1000 placeholders per
        // batch keeps the serialized statement comfortably under any
        // sensible packet ceiling - even worst-case 255-byte VARCHAR
        // ids leave ~250 KiB of overhead. All batches run inside one
        // transaction so the call is atomic: any per-batch failure
        // rolls back every preceding batch.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| FrameworkError::internal(format!("mariadb delete begin tx: {e}")))?;

        for chunk in ids.chunks(DELETE_BATCH_SIZE) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!("DELETE FROM `{table}` WHERE id IN ({placeholders})");
            let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
            for id in chunk {
                q = q.bind(id);
            }
            q.execute(&mut *tx).await.map_err(|e| {
                FrameworkError::internal(format!("mariadb delete failed for store '{table}': {e}"))
            })?;
        }

        tx.commit()
            .await
            .map_err(|e| FrameworkError::internal(format!("mariadb delete commit: {e}")))?;
        Ok(())
    }

    async fn count(&self, store: &str) -> Result<usize, FrameworkError> {
        let table = MariaDbVectorDriver::validate_store_name(store)?;
        self.ensure_version().await?;
        let sql = format!("SELECT COUNT(*) AS n FROM `{table}`");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql))
            .fetch_one(&*self.pool)
            .await
            .map_err(|e| {
                FrameworkError::internal(format!("mariadb count failed for store '{table}': {e}"))
            })?;
        let n: i64 = row.try_get("n").map_err(|e| {
            FrameworkError::internal(format!("mariadb count: decode COUNT column: {e}"))
        })?;
        Ok(n.max(0) as usize)
    }
}

/// Whether `url` is the URL of a MariaDB or a MySQL database, by its
/// scheme. The two are one wire protocol, and the driver takes both.
fn names_mariadb(url: &str) -> bool {
    let scheme = url.split_once("://").map(|(scheme, _)| scheme);
    matches!(
        scheme.map(str::to_ascii_lowercase).as_deref(),
        Some("mariadb" | "mysql")
    )
}

#[cfg(test)]
mod from_env_tests {
    use super::{MariaDbVectorDriver, names_mariadb};

    fn variables(set: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            set.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    // The pool connects on its first query, so building it needs no
    // MariaDB. It needs a runtime.
    #[tokio::test]
    async fn the_url_of_the_vectors_is_read_first() {
        let driver = MariaDbVectorDriver::from_variables(variables(&[
            ("MARIADB_URL", "mariadb://app:secret@127.0.0.1:3306/vectors"),
            ("DATABASE_URL", "postgres://app:secret@127.0.0.1/app"),
        ]));
        assert!(driver.is_ok(), "{:?}", driver.err().map(|e| e.to_string()));
    }

    #[tokio::test]
    async fn the_database_of_the_application_is_taken_when_it_is_a_mariadb() {
        for url in [
            "mariadb://app:secret@127.0.0.1:3306/app",
            "mysql://app:secret@127.0.0.1:3306/app",
            "MySQL://app:secret@127.0.0.1:3306/app",
        ] {
            let lookup = move |name: &str| (name == "DATABASE_URL").then(|| url.to_owned());
            let driver = MariaDbVectorDriver::from_variables(lookup);
            assert!(
                driver.is_ok(),
                "{url}: {:?}",
                driver.err().map(|e| e.to_string())
            );
        }
    }

    #[tokio::test]
    async fn the_database_of_another_engine_is_not_taken() {
        for url in [
            "postgres://app:secret@127.0.0.1/app",
            "sqlite://database.db",
            "/var/lib/app/database.db",
        ] {
            let lookup = move |name: &str| (name == "DATABASE_URL").then(|| url.to_owned());
            let error = MariaDbVectorDriver::from_variables(lookup)
                .err()
                .expect("no MariaDB is named");
            let message = error.to_string();
            assert!(message.contains("MARIADB_URL"), "{message}");
            assert!(
                !message.contains("secret") && !message.contains(url),
                "the error must not show the URL: {message}"
            );
        }
    }

    #[tokio::test]
    async fn with_no_url_at_all_the_error_names_the_variables() {
        for set in [variables(&[]), variables(&[("MARIADB_URL", "  ")])] {
            let error = MariaDbVectorDriver::from_variables(set)
                .err()
                .expect("there is no URL");
            let message = error.to_string();
            assert!(
                message.contains("MARIADB_URL") && message.contains("DATABASE_URL"),
                "{message}"
            );
        }
    }

    #[test]
    fn a_scheme_is_what_names_the_engine() {
        assert!(names_mariadb("mariadb://host/db"));
        assert!(names_mariadb("mysql://host/db"));
        assert!(!names_mariadb("postgres://host/mysql"));
        assert!(!names_mariadb("mysql"));
        assert!(!names_mariadb(""));
    }
}

#[cfg(test)]
mod upsert_clause_tests {
    use super::upsert_values_clause;

    #[test]
    fn single_row() {
        assert_eq!(upsert_values_clause(1), "(?, VEC_FROMTEXT(?), ?)");
    }

    #[test]
    fn three_rows() {
        assert_eq!(
            upsert_values_clause(3),
            "(?, VEC_FROMTEXT(?), ?), (?, VEC_FROMTEXT(?), ?), (?, VEC_FROMTEXT(?), ?)"
        );
    }

    #[test]
    fn zero_rows_produces_empty_string() {
        // Caller is expected to short-circuit on empty input - but the
        // helper itself is total over usize.
        assert_eq!(upsert_values_clause(0), "");
    }

    #[test]
    fn placeholder_count_is_three_per_row() {
        // Each row contributes 3 binds; pin it so a future refactor
        // doesn't silently change the bind count.
        let clause = upsert_values_clause(100);
        let placeholders = clause.matches('?').count();
        assert_eq!(placeholders, 300);
    }
}

#[cfg(test)]
mod distance_extract_tests {
    use super::extract_vector_index_distance;

    #[test]
    fn explicit_cosine() {
        let ddl = "CREATE TABLE `t` (\n  id INT,\n  embedding VECTOR(3) NOT NULL,\n  \
                   VECTOR INDEX (embedding) DISTANCE=cosine\n) ENGINE=InnoDB";
        assert_eq!(
            extract_vector_index_distance(ddl).as_deref(),
            Some("cosine")
        );
    }

    #[test]
    fn explicit_euclidean() {
        let ddl = "...\n  VECTOR INDEX (embedding) DISTANCE=euclidean\n";
        assert_eq!(
            extract_vector_index_distance(ddl).as_deref(),
            Some("euclidean")
        );
    }

    #[test]
    fn with_m_parameter_before_distance() {
        let ddl = "...\n  VECTOR INDEX (embedding) M=8 DISTANCE=cosine\n";
        assert_eq!(
            extract_vector_index_distance(ddl).as_deref(),
            Some("cosine")
        );
    }

    #[test]
    fn omitted_clause_defaults_to_euclidean() {
        // MariaDB's own default when DISTANCE= isn't specified.
        let ddl = "...\n  VECTOR INDEX (embedding)\n";
        assert_eq!(
            extract_vector_index_distance(ddl).as_deref(),
            Some("euclidean")
        );
    }

    #[test]
    fn omitted_clause_with_m_defaults_to_euclidean() {
        let ddl = "...\n  VECTOR INDEX (embedding) M=16\n";
        assert_eq!(
            extract_vector_index_distance(ddl).as_deref(),
            Some("euclidean")
        );
    }

    #[test]
    fn no_vector_index_returns_none() {
        let ddl = "CREATE TABLE `t` (\n  id INT,\n  PRIMARY KEY (id),\n  \
                   INDEX (other_col)\n) ENGINE=InnoDB";
        assert_eq!(extract_vector_index_distance(ddl).as_deref(), None);
    }

    #[test]
    fn matches_canonical_ensure_table_sql_output() {
        // Pin parity with our own emitter - if ensure_table_sql ever
        // changes its line format, this test fails first.
        let sql = super::MariaDbVectorDriver::ensure_table_sql(
            "documents",
            128,
            super::MariaDbDistance::Cosine,
        )
        .unwrap();
        assert_eq!(
            extract_vector_index_distance(&sql).as_deref(),
            Some("cosine")
        );

        let sql = super::MariaDbVectorDriver::ensure_table_sql(
            "documents",
            128,
            super::MariaDbDistance::Euclidean,
        )
        .unwrap();
        assert_eq!(
            extract_vector_index_distance(&sql).as_deref(),
            Some("euclidean")
        );
    }

    /// `SHOW CREATE TABLE` from MariaDB 12.3.1 for the table
    /// `ensure_table_sql` creates, verbatim: the server prints the index as
    /// `VECTOR KEY` and its options as backtick-quoted names.
    const MARIADB_12_COSINE: &str = "CREATE TABLE `v1` (\n  \
        `id` varbinary(254) NOT NULL,\n  \
        `embedding` vector(3) NOT NULL,\n  \
        `metadata` longtext CHARACTER SET utf8mb4 COLLATE utf8mb4_bin DEFAULT NULL CHECK (json_valid(`metadata`)),\n  \
        PRIMARY KEY (`id`),\n  \
        VECTOR KEY `embedding` (`embedding`) `DISTANCE`=cosine\n\
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_uca1400_ai_ci";

    #[test]
    fn real_server_output_names_cosine() {
        assert_eq!(
            extract_vector_index_distance(MARIADB_12_COSINE).as_deref(),
            Some("cosine")
        );
    }

    #[test]
    fn real_server_output_names_euclidean() {
        let ddl = "CREATE TABLE `v2` (\n  `id` varbinary(254) NOT NULL,\n  \
                   `embedding` vector(3) NOT NULL,\n  PRIMARY KEY (`id`),\n  \
                   VECTOR KEY `embedding` (`embedding`) `DISTANCE`=euclidean\n\
                   ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_uca1400_ai_ci";
        assert_eq!(
            extract_vector_index_distance(ddl).as_deref(),
            Some("euclidean")
        );
    }

    #[test]
    fn real_server_output_without_distance_defaults_to_euclidean() {
        let ddl = "CREATE TABLE `v3` (\n  `id` varbinary(254) NOT NULL,\n  \
                   `embedding` vector(3) NOT NULL,\n  PRIMARY KEY (`id`),\n  \
                   VECTOR KEY `embedding` (`embedding`)\n\
                   ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_uca1400_ai_ci";
        assert_eq!(
            extract_vector_index_distance(ddl).as_deref(),
            Some("euclidean")
        );
    }

    #[test]
    fn real_server_output_with_a_named_key_and_m() {
        let ddl = "CREATE TABLE `v4` (\n  `id` varbinary(254) NOT NULL,\n  \
                   `embedding` vector(3) NOT NULL,\n  PRIMARY KEY (`id`),\n  \
                   VECTOR KEY `idx_e` (`embedding`) `M`=8 `DISTANCE`=cosine\n\
                   ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_uca1400_ai_ci";
        assert_eq!(
            extract_vector_index_distance(ddl).as_deref(),
            Some("cosine")
        );
    }

    /// MariaDB 11.7's own test results (`mysql-test/main/vector.result`)
    /// print the option name in lower case.
    #[test]
    fn mariadb_11_7_lower_case_option() {
        let ddl = "  VECTOR KEY `v` (`v`) `distance`=cosine\n";
        assert_eq!(
            extract_vector_index_distance(ddl).as_deref(),
            Some("cosine")
        );
    }

    /// A column or key whose name merely contains the words is not the
    /// vector index.
    #[test]
    fn a_column_named_like_the_keyword_is_not_the_index() {
        let ddl = "CREATE TABLE `t` (\n  `vector key` int,\n  KEY `vector index` (`x`)\n)";
        assert_eq!(extract_vector_index_distance(ddl).as_deref(), None);
    }
}

#[cfg(test)]
mod version_check_tests {
    use super::check_mariadb_117;

    #[test]
    fn accepts_11_7_exact() {
        assert!(check_mariadb_117("11.7.2-MariaDB-1:11.7.2+maria~ubu2404").is_ok());
    }

    #[test]
    fn accepts_11_8() {
        assert!(check_mariadb_117("11.8.0-MariaDB").is_ok());
    }

    #[test]
    fn accepts_12_0() {
        assert!(check_mariadb_117("12.0.1-MariaDB").is_ok());
    }

    #[test]
    fn accepts_legacy_5_5_5_prefix_with_11_7() {
        assert!(check_mariadb_117("5.5.5-11.7.2-MariaDB-1:11.7.2+maria~ubu2404").is_ok());
    }

    #[test]
    fn rejects_11_6() {
        let err = check_mariadb_117("11.6.2-MariaDB").unwrap_err();
        assert!(
            err.contains("11.7+"),
            "error message should reference 11.7+: {err}"
        );
        assert!(
            err.contains("11.6"),
            "error message should echo the found version: {err}"
        );
    }

    #[test]
    fn rejects_10_11() {
        assert!(check_mariadb_117("10.11.6-MariaDB").is_err());
    }

    #[test]
    fn rejects_mysql_8() {
        let err = check_mariadb_117("8.0.36").unwrap_err();
        assert!(
            err.contains("no 'MariaDB' marker"),
            "should call out missing MariaDB marker: {err}"
        );
    }

    #[test]
    fn rejects_unparseable_version() {
        assert!(check_mariadb_117("MariaDB-bogus").is_err());
    }
}
