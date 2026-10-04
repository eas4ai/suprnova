//! Validated application configuration for Suprnova Live.

use std::error::Error;
use std::fmt;

use crate::FrameworkError;
use crate::render_cache::providers::redis::REDACTED_URL;

/// Where the Live instance ledger keeps its records.
///
/// The instance ledger is revision authority: it decides which mounted
/// component instance may accept the next action, and at most one accepted
/// outcome exists per base revision. A deployment of more than one node has
/// to keep that authority somewhere every node reads, which is what the two
/// distributed drivers are for. [`Self::Memory`] is the default and remains
/// correct for exactly one process.
///
/// The bounds a ledger runs under (instance lifetime, retained outcomes,
/// capacity) are the same constants in every driver: what changes is where
/// the records live, never what the state machine over them permits.
#[derive(Clone, Eq, PartialEq)]
pub enum LedgerDriver {
    /// One process, its own records. Records are lost with the process,
    /// which for a single node is the same thing as the process's own
    /// instances being gone.
    Memory,
    /// Records in the two Live record tables the RenderCache tier migration
    /// creates, shared by every node pointed at the same database, and
    /// coupled to the host transaction when one is open.
    Database,
    /// Records in Redis, shared by every node pointed at the same instance
    /// and prefix. Redis cannot join a host transaction, so a claim there is
    /// made before the transaction it belongs to commits - which spec 05
    /// permits, and which is the difference an operator is choosing between
    /// this driver and [`Self::Database`].
    Redis {
        /// Where the records are kept. It can carry a password, so it is
        /// never printed: see this type's [`fmt::Debug`] implementation.
        url: String,
        /// The literal string every key this deployment writes begins with.
        prefix: String,
    },
}

/// Manual, not derived: the [`LedgerDriver::Redis`] endpoint can carry a
/// password, and a driver reaches boot errors and logs.
impl fmt::Debug for LedgerDriver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Memory => formatter.write_str("Memory"),
            Self::Database => formatter.write_str("Database"),
            Self::Redis { url: _, prefix } => formatter
                .debug_struct("Redis")
                .field("url", &REDACTED_URL)
                .field("prefix", prefix)
                .finish(),
        }
    }
}

impl LedgerDriver {
    /// Reads `LIVE_LEDGER_DRIVER` (`memory` default, `database`, `redis`),
    /// with `LIVE_REDIS_URL` (falling back to `REDIS_URL`, then to
    /// `redis://127.0.0.1:6379`) and `LIVE_REDIS_PREFIX` for the Redis
    /// driver.
    ///
    /// The prefix defaults to this crate's own Live key namespace, spelled
    /// out in the `DEFAULT_LIVE_REDIS_PREFIX` constant beside this function
    /// (crate-private, so this is a plain code span rather than a link, and
    /// the literal is not repeated here: the rendered public Live
    /// documentation must not carry an internal crate path, and this
    /// namespace is that path plus a colon).
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] when `LIVE_LEDGER_DRIVER` names a driver
    /// this build does not have. The message names the variable and never
    /// repeats the rejected value: an environment value can carry a secret.
    pub fn from_env() -> Result<Self, FrameworkError> {
        Self::from_source(&|name| std::env::var(name).ok())
    }

    /// [`Self::from_env`] over any reader, so the parser can be proven
    /// against fixed pairs rather than against the process environment.
    fn from_source(read: &dyn Fn(&str) -> Option<String>) -> Result<Self, FrameworkError> {
        let non_empty = |name: &str| read(name).filter(|value| !value.is_empty());
        match non_empty("LIVE_LEDGER_DRIVER").as_deref() {
            None | Some("memory") => Ok(Self::Memory),
            Some("database") => Ok(Self::Database),
            Some("redis") => Ok(Self::Redis {
                url: non_empty("LIVE_REDIS_URL")
                    .or_else(|| non_empty("REDIS_URL"))
                    .unwrap_or_else(|| DEFAULT_LIVE_REDIS_URL.to_owned()),
                prefix: non_empty("LIVE_REDIS_PREFIX")
                    .unwrap_or_else(|| DEFAULT_LIVE_REDIS_PREFIX.to_owned()),
            }),
            Some(_) => Err(FrameworkError::internal(
                "LiveRuntime: LIVE_LEDGER_DRIVER is set to a value that is not one of memory, \
                 database, or redis. The rejected value is not repeated here: an environment \
                 value can carry a secret. Set LIVE_LEDGER_DRIVER to one of the accepted \
                 values, or unset it to run the in-process ledger.",
            )),
        }
    }
}

/// Where the Redis ledger driver connects when neither `LIVE_REDIS_URL` nor
/// `REDIS_URL` names an endpoint.
const DEFAULT_LIVE_REDIS_URL: &str = "redis://127.0.0.1:6379";
/// The key namespace the Redis ledger driver writes under when
/// `LIVE_REDIS_PREFIX` does not name one.
const DEFAULT_LIVE_REDIS_PREFIX: &str = "suprnova_live:";

/// Every limit Live runs under, validated once at startup.
///
/// The server's configuration is the single source of each limit. The
/// bootstrap writes the page limits into the inert configuration element and
/// the browser runtime reads them from there, so the browser never applies a
/// limit tighter than the one configured here. Each limit is read from a
/// `LIVE_*` key in the application's `.env` file ([`Self::from_env`]) or set
/// in code through [`LiveConfigBuilder`]; the defaults are sized for large
/// modern pages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveConfig {
    max_request_bytes: usize,
    max_response_bytes: usize,
    max_html_bytes: usize,
    max_json_depth: usize,
    max_json_entries: usize,
    max_request_items: usize,
    max_response_items: usize,
    max_context_lifetime_ms: u64,
    request_timeout_ms: u32,
    max_queued_per_island: u8,
    max_parallel_per_island: u8,
    morph: LiveMorphLimits,
    async_max_payload_bytes: usize,
    async_max_buffer_bytes: usize,
    async_max_queued_events: usize,
    async_max_replay_events: usize,
    max_redirect_bytes: usize,
    upload: LiveUploadLimits,
}

/// The limits on file uploads, which the server enforces and the browser
/// reads from the configuration element.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveUploadLimits {
    chunk_bytes: usize,
    max_active: usize,
    max_file_bytes: u64,
    max_pending_files: usize,
    max_pending_bytes: u64,
    max_storage_bytes: u64,
}

impl LiveUploadLimits {
    /// Bytes in one chunk request (`LIVE_UPLOAD_CHUNK_BYTES`). The server
    /// holds a chunk in memory while it checks and stores it.
    #[must_use]
    pub const fn chunk_bytes(self) -> usize {
        self.chunk_bytes
    }

    /// Transfers running at once (`LIVE_UPLOAD_MAX_ACTIVE`).
    #[must_use]
    pub const fn max_active(self) -> usize {
        self.max_active
    }

    /// Bytes in one uploaded file (`LIVE_UPLOAD_MAX_FILE_BYTES`).
    #[must_use]
    pub const fn max_file_bytes(self) -> u64 {
        self.max_file_bytes
    }

    /// Files one visitor may have selected and not yet finished
    /// (`LIVE_UPLOAD_MAX_PENDING_FILES`).
    #[must_use]
    pub const fn max_pending_files(self) -> usize {
        self.max_pending_files
    }

    /// Bytes across one visitor's unfinished files
    /// (`LIVE_UPLOAD_MAX_PENDING_BYTES`).
    #[must_use]
    pub const fn max_pending_bytes(self) -> u64 {
        self.max_pending_bytes
    }

    /// Bytes the temporary upload store may hold across every visitor
    /// (`LIVE_UPLOAD_MAX_STORAGE_BYTES`).
    #[must_use]
    pub const fn max_storage_bytes(self) -> u64 {
        self.max_storage_bytes
    }
}

/// The browser morph's limits on one rendered island.
///
/// The server never morphs; it validates these values and delivers them to
/// the browser in the configuration element.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveMorphLimits {
    max_nodes: u32,
    max_depth: u32,
    max_keys: u32,
    max_attributes: u32,
    max_attributes_per_element: u32,
    deadline_ms: u32,
}

impl LiveMorphLimits {
    /// Nodes in one rendered island (`LIVE_MORPH_MAX_NODES`).
    #[must_use]
    pub const fn max_nodes(self) -> u32 {
        self.max_nodes
    }

    /// Element nesting in one rendered island (`LIVE_MORPH_MAX_DEPTH`).
    #[must_use]
    pub const fn max_depth(self) -> u32 {
        self.max_depth
    }

    /// Keyed elements in one rendered island (`LIVE_MORPH_MAX_KEYS`).
    #[must_use]
    pub const fn max_keys(self) -> u32 {
        self.max_keys
    }

    /// Attributes across one rendered island (`LIVE_MORPH_MAX_ATTRIBUTES`).
    #[must_use]
    pub const fn max_attributes(self) -> u32 {
        self.max_attributes
    }

    /// Attributes on one rendered element
    /// (`LIVE_MORPH_MAX_ATTRIBUTES_PER_ELEMENT`).
    #[must_use]
    pub const fn max_attributes_per_element(self) -> u32 {
        self.max_attributes_per_element
    }

    /// Milliseconds one morph may run before it is abandoned
    /// (`LIVE_MORPH_DEADLINE_MS`); `0` means no deadline.
    #[must_use]
    pub const fn deadline_ms(self) -> u32 {
        self.deadline_ms
    }
}

const MIB: usize = 1024 * 1024;

/// The ceiling on every byte setting. A Live message is held in memory whole
/// while it is parsed, so the ceiling keeps one message to one bounded
/// allocation; 1 GiB is far past any page.
const HARD_MAX_CONTROL_BYTES: usize = 1024 * MIB;
/// Sized for a large modern page: a data table of tens of thousands of rows
/// renders in a few megabytes.
const DEFAULT_CONTROL_BYTES: usize = 16 * MIB;
/// JSON nesting. The canonical parser walks containers recursively, so this
/// is its stack guard; 32 levels is far past real component state.
const DEFAULT_JSON_DEPTH: usize = 32;
const HARD_MAX_JSON_DEPTH: usize = suprnova_live::limits::HARD_MAX_DEPTH;
/// Array elements plus object members in one request. A parsed entry costs
/// memory beyond its bytes, so this bounds what a request of many tiny
/// entries can amplify into; a million covers a large table's state.
const DEFAULT_JSON_ENTRIES: usize = 1_000_000;
const HARD_MAX_JSON_ENTRIES: usize = suprnova_live::limits::HARD_MAX_ENTRIES;
/// Items in one protocol collection. Each is a unit of server work: a model
/// proposal, an operation, an action argument, a validation entry, an event,
/// an effect, a rendered child. A large editable grid stays well inside it.
const DEFAULT_ITEMS: usize = 65_536;
const HARD_MAX_ITEMS: usize = suprnova_live::limits::HARD_MAX_COLLECTION_ITEMS;
/// The engine's ceiling on one trusted request context's validity window.
const HARD_MAX_CONTEXT_LIFETIME_MS: u64 = 300_000;
const DEFAULT_CONTEXT_LIFETIME_MS: u64 = 30_000;
/// How long the browser waits for one Live response, body included. A 16 MiB
/// response over a 5 Mbit/s mobile link takes about 27 seconds.
const DEFAULT_REQUEST_TIMEOUT_MS: u32 = 60_000;
/// `setTimeout`'s largest delay; a longer one fires at once.
const HARD_MAX_TIMER_MS: u32 = 2_147_483_647;
/// Requests one island queues and runs at once. The browser scheduler's
/// per-island structures are built for at most these.
const DEFAULT_QUEUED_PER_ISLAND: u8 = 8;
const HARD_MAX_QUEUED_PER_ISLAND: u8 = 64;
const DEFAULT_PARALLEL_PER_ISLAND: u8 = 1;
const HARD_MAX_PARALLEL_PER_ISLAND: u8 = 8;
/// Nodes, keyed elements and attributes in one rendered island. The morph
/// visits each of them, so they bound one update's work; the defaults admit
/// tables of hundreds of thousands of rows.
const DEFAULT_MORPH_MAX_NODES: u32 = 1_000_000;
const DEFAULT_MORPH_MAX_KEYS: u32 = 1_000_000;
const DEFAULT_MORPH_MAX_ATTRIBUTES: u32 = 10_000_000;
/// Attribute reconciliation looks each name up in the element's list, so one
/// element's cost grows with the square of its attribute count.
const DEFAULT_MORPH_MAX_ATTRIBUTES_PER_ELEMENT: u32 = 4_096;
const HARD_MAX_MORPH_COUNT: u32 = 1 << 30;
/// Element nesting in one rendered island. The morph library walks the tree
/// recursively, so this guards the browser's call stack; browsers' own HTML
/// parsers stop nesting at 512.
const DEFAULT_MORPH_MAX_DEPTH: u32 = 512;
const HARD_MAX_MORPH_DEPTH: u32 = 4_096;
/// No deadline by default: the morph runs synchronously, so a deadline can
/// only abandon work that already held the main thread and changed part of
/// the island.
const DEFAULT_MORPH_DEADLINE_MS: u32 = 0;
/// One asynchronous event payload, and what one open document's delivery
/// queue may hold in server memory while its browser catches up.
const DEFAULT_ASYNC_MAX_PAYLOAD_BYTES: usize = MIB;
const DEFAULT_ASYNC_MAX_BUFFER_BYTES: usize = 16 * MIB;
const HARD_MAX_ASYNC_PAYLOAD_BYTES: usize = suprnova_live::async_updates::MAX_ASYNC_PAYLOAD_BYTES;
const HARD_MAX_ASYNC_BUFFER_BYTES: usize = suprnova_live::async_updates::MAX_ASYNC_BUFFER_BYTES;
/// Events one open document may hold before its browser applies them, and
/// events one reconnect replay may carry. Each queued event is server memory
/// already bounded in bytes by `LIVE_ASYNC_MAX_BUFFER_BYTES`; the count bounds
/// the per-event bookkeeping, and the engine's ceiling is its own.
const DEFAULT_ASYNC_MAX_QUEUED_EVENTS: usize = 4_096;
const DEFAULT_ASYNC_MAX_REPLAY_EVENTS: usize = 4_096;
const HARD_MAX_ASYNC_EVENTS: usize = suprnova_live::async_updates::MAX_ASYNC_BUFFER_EVENTS;
/// One redirect or history URL a Live response carries. Browsers refuse a
/// URL past 2 MiB, so a longer one could never be followed.
const DEFAULT_MAX_REDIRECT_BYTES: usize = 64 * 1024;
const HARD_MAX_REDIRECT_BYTES: usize = 2 * MIB;
/// Upload limits. A file is stored in chunks, each held in server memory
/// while it is checked, so the chunk size and the transfers running at once
/// bound that memory; the file and pending sizes bound disk.
const GIB: u64 = 1024 * 1024 * 1024;
const TIB: u64 = 1024 * GIB;
const DEFAULT_UPLOAD_CHUNK_BYTES: usize = 8 * MIB;
const HARD_MAX_UPLOAD_CHUNK_BYTES: usize = 64 * MIB;
const DEFAULT_UPLOAD_MAX_ACTIVE: usize = 8;
const HARD_MAX_UPLOAD_ACTIVE: usize = 1_024;
/// Chunk bytes times transfers running at once: the chunk memory one process
/// may hold for uploads.
const HARD_MAX_UPLOAD_IN_FLIGHT_BYTES: u64 = GIB;
const DEFAULT_UPLOAD_MAX_FILE_BYTES: u64 = GIB;
const HARD_MAX_UPLOAD_FILE_BYTES: u64 = TIB;
const DEFAULT_UPLOAD_MAX_PENDING_FILES: usize = 1_024;
const HARD_MAX_UPLOAD_PENDING_FILES: usize = 100_000;
const DEFAULT_UPLOAD_MAX_PENDING_BYTES: u64 = 4 * GIB;
const HARD_MAX_UPLOAD_PENDING_BYTES: u64 = 16 * TIB;
const DEFAULT_UPLOAD_MAX_STORAGE_BYTES: u64 = 16 * GIB;
const HARD_MAX_UPLOAD_STORAGE_BYTES: u64 = 64 * TIB;
/// The chunks one file may take: the engine retains a record per chunk and
/// a retry outcome per chunk plus six, under its ceiling of 100,000.
const HARD_MAX_UPLOAD_CHUNKS_PER_FILE: u64 = 99_994;
/// The chunk records retained per file when the file size does not need
/// more; the engine's reference profile.
const MIN_UPLOAD_CHUNKS_PER_FILE: u64 = 4_096;
/// What one chunk copies besides its own bytes while it is buffered.
const UPLOAD_CHUNK_COPY_BYTES: u64 = 64 * 1024;

/// A configured Live limit that a request, response, render or event went
/// over, with everything a developer needs to change it: the limit, the
/// measured value, the configured value and the configuration key.
///
/// It holds only sizes and fixed names, never content, so it is safe to log.
/// The browser runtime prints the same sentence for the limits it applies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveLimitExceeded {
    label: &'static str,
    key: &'static str,
    unit: &'static str,
    measured: u64,
    configured: u64,
    at_least: bool,
}

impl LiveLimitExceeded {
    /// One request body over `LIVE_MAX_REQUEST_BYTES`.
    #[must_use]
    pub const fn request_bytes(measured: u64, configured: u64, at_least: bool) -> Self {
        Self::new(
            "request size",
            "LIVE_MAX_REQUEST_BYTES",
            "bytes",
            measured,
            configured,
            at_least,
        )
    }

    /// One response body over `LIVE_MAX_RESPONSE_BYTES`.
    #[must_use]
    pub const fn response_bytes(measured: u64, configured: u64, at_least: bool) -> Self {
        Self::new(
            "response size",
            "LIVE_MAX_RESPONSE_BYTES",
            "bytes",
            measured,
            configured,
            at_least,
        )
    }

    /// One island render over `LIVE_MAX_HTML_BYTES`.
    #[must_use]
    pub const fn html_bytes(measured: u64, configured: u64, at_least: bool) -> Self {
        Self::new(
            "island HTML size",
            "LIVE_MAX_HTML_BYTES",
            "bytes",
            measured,
            configured,
            at_least,
        )
    }

    /// One asynchronous payload over `LIVE_ASYNC_MAX_PAYLOAD_BYTES`.
    #[must_use]
    pub const fn async_payload_bytes(measured: u64, configured: u64) -> Self {
        Self::new(
            "async payload size",
            "LIVE_ASYNC_MAX_PAYLOAD_BYTES",
            "bytes",
            measured,
            configured,
            false,
        )
    }

    /// One redirect or history URL over `LIVE_MAX_REDIRECT_BYTES`.
    #[must_use]
    pub const fn redirect_bytes(measured: u64, configured: u64) -> Self {
        Self::new(
            "redirect URL size",
            "LIVE_MAX_REDIRECT_BYTES",
            "bytes",
            measured,
            configured,
            false,
        )
    }

    /// One uploaded file over `LIVE_UPLOAD_MAX_FILE_BYTES`.
    #[must_use]
    pub const fn upload_file_bytes(measured: u64, configured: u64) -> Self {
        Self::new(
            "upload file size",
            "LIVE_UPLOAD_MAX_FILE_BYTES",
            "bytes",
            measured,
            configured,
            false,
        )
    }

    /// One upload chunk over `LIVE_UPLOAD_CHUNK_BYTES`.
    #[must_use]
    pub const fn upload_chunk_bytes(measured: u64, configured: u64, at_least: bool) -> Self {
        Self::new(
            "upload chunk size",
            "LIVE_UPLOAD_CHUNK_BYTES",
            "bytes",
            measured,
            configured,
            at_least,
        )
    }

    const fn new(
        label: &'static str,
        key: &'static str,
        unit: &'static str,
        measured: u64,
        configured: u64,
        at_least: bool,
    ) -> Self {
        Self {
            label,
            key,
            unit,
            measured,
            configured,
            at_least,
        }
    }

    /// The configuration key to change, such as `LIVE_MAX_HTML_BYTES`.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.key
    }

    /// The size measured; with [`Self::at_least`] the real size is larger.
    #[must_use]
    pub const fn measured(self) -> u64 {
        self.measured
    }

    /// The configured limit the value went over.
    #[must_use]
    pub const fn configured(self) -> u64 {
        self.configured
    }

    /// Whether measuring stopped at the limit, so the value is larger still.
    #[must_use]
    pub const fn at_least(self) -> bool {
        self.at_least
    }
}

impl fmt::Display for LiveLimitExceeded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Suprnova Live {} limit exceeded: measured {}{} {}, configured {} {}. Raise {} in the \
             application's .env file to allow it.",
            self.label,
            if self.at_least { "at least " } else { "" },
            self.measured,
            self.unit,
            self.configured,
            self.unit,
            self.key
        )
    }
}

impl Error for LiveLimitExceeded {}

/// One `LIVE_*` limit key: its name, the unit its value is counted in, and
/// how to read its value from a built configuration.
pub(crate) struct LiveLimitKey {
    /// The `.env` key, such as `LIVE_MAX_HTML_BYTES`.
    pub(crate) key: &'static str,
    /// The unit its value is counted in, such as `bytes`.
    pub(crate) unit: &'static str,
    /// Reads its value from a built configuration.
    pub(crate) value: fn(LiveConfig) -> u64,
}

/// Every `LIVE_*` limit key, in the order the manual lists them. The tooling
/// report and the manual check both walk this table, so a key added here is
/// reported and must be documented.
pub(crate) const LIVE_LIMIT_KEYS: &[LiveLimitKey] = &[
    LiveLimitKey {
        key: "LIVE_MAX_REQUEST_BYTES",
        unit: "bytes",
        value: |config| config.max_request_bytes as u64,
    },
    LiveLimitKey {
        key: "LIVE_MAX_RESPONSE_BYTES",
        unit: "bytes",
        value: |config| config.max_response_bytes as u64,
    },
    LiveLimitKey {
        key: "LIVE_MAX_HTML_BYTES",
        unit: "bytes",
        value: |config| config.max_html_bytes as u64,
    },
    LiveLimitKey {
        key: "LIVE_MAX_JSON_DEPTH",
        unit: "levels",
        value: |config| config.max_json_depth as u64,
    },
    LiveLimitKey {
        key: "LIVE_MAX_JSON_ENTRIES",
        unit: "entries",
        value: |config| config.max_json_entries as u64,
    },
    LiveLimitKey {
        key: "LIVE_MAX_REQUEST_ITEMS",
        unit: "items",
        value: |config| config.max_request_items as u64,
    },
    LiveLimitKey {
        key: "LIVE_MAX_RESPONSE_ITEMS",
        unit: "items",
        value: |config| config.max_response_items as u64,
    },
    LiveLimitKey {
        key: "LIVE_MAX_CONTEXT_LIFETIME_MS",
        unit: "ms",
        value: |config| config.max_context_lifetime_ms,
    },
    LiveLimitKey {
        key: "LIVE_REQUEST_TIMEOUT_MS",
        unit: "ms",
        value: |config| u64::from(config.request_timeout_ms),
    },
    LiveLimitKey {
        key: "LIVE_MAX_QUEUED_PER_ISLAND",
        unit: "requests",
        value: |config| u64::from(config.max_queued_per_island),
    },
    LiveLimitKey {
        key: "LIVE_MAX_PARALLEL_PER_ISLAND",
        unit: "requests",
        value: |config| u64::from(config.max_parallel_per_island),
    },
    LiveLimitKey {
        key: "LIVE_MORPH_MAX_NODES",
        unit: "nodes",
        value: |config| u64::from(config.morph.max_nodes),
    },
    LiveLimitKey {
        key: "LIVE_MORPH_MAX_DEPTH",
        unit: "levels",
        value: |config| u64::from(config.morph.max_depth),
    },
    LiveLimitKey {
        key: "LIVE_MORPH_MAX_KEYS",
        unit: "keyed elements",
        value: |config| u64::from(config.morph.max_keys),
    },
    LiveLimitKey {
        key: "LIVE_MORPH_MAX_ATTRIBUTES",
        unit: "attributes",
        value: |config| u64::from(config.morph.max_attributes),
    },
    LiveLimitKey {
        key: "LIVE_MORPH_MAX_ATTRIBUTES_PER_ELEMENT",
        unit: "attributes",
        value: |config| u64::from(config.morph.max_attributes_per_element),
    },
    LiveLimitKey {
        key: "LIVE_MORPH_DEADLINE_MS",
        unit: "ms",
        value: |config| u64::from(config.morph.deadline_ms),
    },
    LiveLimitKey {
        key: "LIVE_ASYNC_MAX_PAYLOAD_BYTES",
        unit: "bytes",
        value: |config| config.async_max_payload_bytes as u64,
    },
    LiveLimitKey {
        key: "LIVE_ASYNC_MAX_BUFFER_BYTES",
        unit: "bytes",
        value: |config| config.async_max_buffer_bytes as u64,
    },
    LiveLimitKey {
        key: "LIVE_ASYNC_MAX_QUEUED_EVENTS",
        unit: "events",
        value: |config| config.async_max_queued_events as u64,
    },
    LiveLimitKey {
        key: "LIVE_ASYNC_MAX_REPLAY_EVENTS",
        unit: "events",
        value: |config| config.async_max_replay_events as u64,
    },
    LiveLimitKey {
        key: "LIVE_MAX_REDIRECT_BYTES",
        unit: "bytes",
        value: |config| config.max_redirect_bytes as u64,
    },
    LiveLimitKey {
        key: "LIVE_UPLOAD_CHUNK_BYTES",
        unit: "bytes",
        value: |config| config.upload.chunk_bytes as u64,
    },
    LiveLimitKey {
        key: "LIVE_UPLOAD_MAX_ACTIVE",
        unit: "transfers",
        value: |config| config.upload.max_active as u64,
    },
    LiveLimitKey {
        key: "LIVE_UPLOAD_MAX_FILE_BYTES",
        unit: "bytes",
        value: |config| config.upload.max_file_bytes,
    },
    LiveLimitKey {
        key: "LIVE_UPLOAD_MAX_PENDING_FILES",
        unit: "files",
        value: |config| config.upload.max_pending_files as u64,
    },
    LiveLimitKey {
        key: "LIVE_UPLOAD_MAX_PENDING_BYTES",
        unit: "bytes",
        value: |config| config.upload.max_pending_bytes,
    },
    LiveLimitKey {
        key: "LIVE_UPLOAD_MAX_STORAGE_BYTES",
        unit: "bytes",
        value: |config| config.upload.max_storage_bytes,
    },
];

impl LiveConfig {
    /// Starts a builder at the defaults.
    #[must_use]
    pub const fn builder() -> LiveConfigBuilder {
        LiveConfigBuilder::new()
    }

    /// Returns the defaults, sized for large modern pages.
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            max_request_bytes: DEFAULT_CONTROL_BYTES,
            max_response_bytes: DEFAULT_CONTROL_BYTES,
            max_html_bytes: DEFAULT_CONTROL_BYTES,
            max_json_depth: DEFAULT_JSON_DEPTH,
            max_json_entries: DEFAULT_JSON_ENTRIES,
            max_request_items: DEFAULT_ITEMS,
            max_response_items: DEFAULT_ITEMS,
            max_context_lifetime_ms: DEFAULT_CONTEXT_LIFETIME_MS,
            request_timeout_ms: DEFAULT_REQUEST_TIMEOUT_MS,
            max_queued_per_island: DEFAULT_QUEUED_PER_ISLAND,
            max_parallel_per_island: DEFAULT_PARALLEL_PER_ISLAND,
            morph: LiveMorphLimits {
                max_nodes: DEFAULT_MORPH_MAX_NODES,
                max_depth: DEFAULT_MORPH_MAX_DEPTH,
                max_keys: DEFAULT_MORPH_MAX_KEYS,
                max_attributes: DEFAULT_MORPH_MAX_ATTRIBUTES,
                max_attributes_per_element: DEFAULT_MORPH_MAX_ATTRIBUTES_PER_ELEMENT,
                deadline_ms: DEFAULT_MORPH_DEADLINE_MS,
            },
            async_max_payload_bytes: DEFAULT_ASYNC_MAX_PAYLOAD_BYTES,
            async_max_buffer_bytes: DEFAULT_ASYNC_MAX_BUFFER_BYTES,
            async_max_queued_events: DEFAULT_ASYNC_MAX_QUEUED_EVENTS,
            async_max_replay_events: DEFAULT_ASYNC_MAX_REPLAY_EVENTS,
            max_redirect_bytes: DEFAULT_MAX_REDIRECT_BYTES,
            upload: LiveUploadLimits {
                chunk_bytes: DEFAULT_UPLOAD_CHUNK_BYTES,
                max_active: DEFAULT_UPLOAD_MAX_ACTIVE,
                max_file_bytes: DEFAULT_UPLOAD_MAX_FILE_BYTES,
                max_pending_files: DEFAULT_UPLOAD_MAX_PENDING_FILES,
                max_pending_bytes: DEFAULT_UPLOAD_MAX_PENDING_BYTES,
                max_storage_bytes: DEFAULT_UPLOAD_MAX_STORAGE_BYTES,
            },
        }
    }

    /// Reads every `LIVE_*` limit key from the environment, which
    /// `Config::init` has loaded from the application's `.env` file, over the
    /// defaults.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] when a key is not a whole number or a value
    /// breaks its rule; the message names the key, the value and the rule.
    pub fn from_env() -> Result<Self, FrameworkError> {
        Self::from_source(&|name| std::env::var(name).ok())
    }

    /// [`Self::from_env`] over any reader, so the parser can be proven
    /// against fixed pairs rather than against the process environment.
    fn from_source(read: &dyn Fn(&str) -> Option<String>) -> Result<Self, FrameworkError> {
        let builder = LiveConfigBuilder::from_source(read)?;
        builder.build().map_err(|error| {
            let value = read(error.key()).filter(|value| !value.trim().is_empty());
            let set = value.map_or_else(
                || format!("{} (its default)", error.key()),
                |value| format!("{}={}", error.key(), value.trim()),
            );
            FrameworkError::internal(format!(
                "Live configuration rejected: {set} {}. Change it in the application's .env \
                 file.",
                error.rule()
            ))
        })
    }

    /// The configuration the runtime runs under: the one bound in the
    /// container when the application built one in code, otherwise the one
    /// the environment describes.
    pub(crate) fn resolve() -> Result<Self, FrameworkError> {
        crate::App::resolve::<Self>().or_else(|_| Self::from_env())
    }

    /// One complete Live request body (`LIVE_MAX_REQUEST_BYTES`). The signed
    /// snapshot travels in every request, so this is also the snapshot limit.
    #[must_use]
    pub const fn max_request_bytes(self) -> usize {
        self.max_request_bytes
    }

    /// One complete Live response body (`LIVE_MAX_RESPONSE_BYTES`).
    #[must_use]
    pub const fn max_response_bytes(self) -> usize {
        self.max_response_bytes
    }

    /// One island render's HTML, apart from the response around it
    /// (`LIVE_MAX_HTML_BYTES`).
    #[must_use]
    pub const fn max_html_bytes(self) -> usize {
        self.max_html_bytes
    }

    /// Container nesting in Live JSON (`LIVE_MAX_JSON_DEPTH`).
    #[must_use]
    pub const fn max_json_depth(self) -> usize {
        self.max_json_depth
    }

    /// Array elements plus object members in one request
    /// (`LIVE_MAX_JSON_ENTRIES`).
    #[must_use]
    pub const fn max_json_entries(self) -> usize {
        self.max_json_entries
    }

    /// Items in one request collection: model proposals, operations, action
    /// arguments (`LIVE_MAX_REQUEST_ITEMS`).
    #[must_use]
    pub const fn max_request_items(self) -> usize {
        self.max_request_items
    }

    /// Items in one response collection: validation entries, events,
    /// effects, extensions, and the assets, mounts and children of one render
    /// (`LIVE_MAX_RESPONSE_ITEMS`).
    #[must_use]
    pub const fn max_response_items(self) -> usize {
        self.max_response_items
    }

    /// The validity window of one trusted request context
    /// (`LIVE_MAX_CONTEXT_LIFETIME_MS`).
    #[must_use]
    pub const fn max_context_lifetime_ms(self) -> u64 {
        self.max_context_lifetime_ms
    }

    /// How long the browser waits for one Live response
    /// (`LIVE_REQUEST_TIMEOUT_MS`).
    #[must_use]
    pub const fn request_timeout_ms(self) -> u32 {
        self.request_timeout_ms
    }

    /// Requests one island may queue (`LIVE_MAX_QUEUED_PER_ISLAND`).
    #[must_use]
    pub const fn max_queued_per_island(self) -> u8 {
        self.max_queued_per_island
    }

    /// Requests one island may run at once (`LIVE_MAX_PARALLEL_PER_ISLAND`).
    #[must_use]
    pub const fn max_parallel_per_island(self) -> u8 {
        self.max_parallel_per_island
    }

    /// The browser morph's limits on one rendered island.
    #[must_use]
    pub const fn morph(self) -> LiveMorphLimits {
        self.morph
    }

    /// One asynchronous event payload (`LIVE_ASYNC_MAX_PAYLOAD_BYTES`).
    #[must_use]
    pub const fn async_max_payload_bytes(self) -> usize {
        self.async_max_payload_bytes
    }

    /// What one open document's asynchronous delivery queue may hold
    /// (`LIVE_ASYNC_MAX_BUFFER_BYTES`).
    #[must_use]
    pub const fn async_max_buffer_bytes(self) -> usize {
        self.async_max_buffer_bytes
    }

    /// Asynchronous events one open document may hold before its browser
    /// applies them (`LIVE_ASYNC_MAX_QUEUED_EVENTS`).
    #[must_use]
    pub const fn async_max_queued_events(self) -> usize {
        self.async_max_queued_events
    }

    /// Events one reconnect replay may carry (`LIVE_ASYNC_MAX_REPLAY_EVENTS`).
    /// A replay is queued whole, so it fits inside the queued-event limit.
    #[must_use]
    pub const fn async_max_replay_events(self) -> usize {
        self.async_max_replay_events
    }

    /// One redirect or history URL a response carries
    /// (`LIVE_MAX_REDIRECT_BYTES`).
    #[must_use]
    pub const fn max_redirect_bytes(self) -> usize {
        self.max_redirect_bytes
    }

    /// The upload limits.
    #[must_use]
    pub const fn upload(self) -> LiveUploadLimits {
        self.upload
    }

    /// Every configured limit as its `.env` key, unit and value, in the order
    /// the manual lists them; `live:inspect` reports exactly these.
    pub(crate) fn limit_values(self) -> Vec<(&'static str, &'static str, u64)> {
        LIVE_LIMIT_KEYS
            .iter()
            .map(|limit| (limit.key, limit.unit, (limit.value)(self)))
            .collect()
    }

    /// The engine's upload profile under these limits. Every value the
    /// configuration does not name stays at the engine's reference profile.
    pub(crate) fn engine_upload_limits(
        self,
    ) -> Result<suprnova_live::limits::UploadLimits, FrameworkError> {
        let upload = self.upload;
        let chunk = upload.chunk_bytes as u64;
        let chunks_per_file = upload
            .max_file_bytes
            .div_ceil(chunk)
            .max(MIN_UPLOAD_CHUNKS_PER_FILE);
        let in_flight = (chunk + UPLOAD_CHUNK_COPY_BYTES)
            .saturating_mul(upload.max_active as u64)
            .min(HARD_MAX_UPLOAD_IN_FLIGHT_BYTES);
        let rejected = || FrameworkError::internal("Live upload limits were rejected");
        let config = suprnova_live::limits::UploadLimitConfig {
            max_files_per_field: upload.max_pending_files,
            max_pending_per_scope: upload.max_pending_files,
            max_file_bytes: upload.max_file_bytes,
            max_aggregate_bytes: upload.max_pending_bytes,
            max_chunk_bytes: upload.chunk_bytes,
            max_chunks_per_file: usize::try_from(chunks_per_file).map_err(|_| rejected())?,
            max_in_flight_bytes: usize::try_from(in_flight).map_err(|_| rejected())?,
            max_concurrent_transfers: upload.max_active,
            max_creations_per_window: upload.max_pending_files,
            max_storage_bytes: upload.max_storage_bytes,
            max_idempotency_outcomes: usize::try_from(chunks_per_file + 6)
                .map_err(|_| rejected())?,
            ..suprnova_live::limits::UploadLimitConfig::reference()
        };
        suprnova_live::limits::UploadLimits::new(config).map_err(|_| rejected())
    }
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self::standard()
    }
}

/// Startup-only builder for [`LiveConfig`].
///
/// A setting left unset follows the setting it must fit inside: the response
/// limit follows the request limit down, the island HTML and redirect limits
/// follow the response limit, the per-element attribute limit follows the
/// island-wide one, the asynchronous payload limit follows the queue bytes,
/// the replay count follows the queue depth, and the upload chunk follows the
/// file size. The upload pending files, pending bytes and store size follow
/// up instead, to the limits they must hold. Setting only
/// `max_request_bytes(256 * 1024)` therefore lowers all three byte limits
/// together instead of failing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveConfigBuilder {
    max_request_bytes: usize,
    max_response_bytes: Option<usize>,
    max_html_bytes: Option<usize>,
    max_json_depth: usize,
    max_json_entries: usize,
    max_request_items: usize,
    max_response_items: usize,
    max_context_lifetime_ms: u64,
    request_timeout_ms: u32,
    max_queued_per_island: u8,
    max_parallel_per_island: u8,
    morph_max_nodes: u32,
    morph_max_depth: u32,
    morph_max_keys: u32,
    morph_max_attributes: u32,
    morph_max_attributes_per_element: Option<u32>,
    morph_deadline_ms: u32,
    async_max_payload_bytes: Option<usize>,
    async_max_buffer_bytes: usize,
    async_max_queued_events: usize,
    async_max_replay_events: Option<usize>,
    max_redirect_bytes: Option<usize>,
    upload_chunk_bytes: Option<usize>,
    upload_max_active: usize,
    upload_max_file_bytes: u64,
    upload_max_pending_files: Option<usize>,
    upload_max_pending_bytes: Option<u64>,
    upload_max_storage_bytes: Option<u64>,
}

impl LiveConfigBuilder {
    /// Creates a builder at the [`LiveConfig::standard`] defaults.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_request_bytes: DEFAULT_CONTROL_BYTES,
            max_response_bytes: None,
            max_html_bytes: None,
            max_json_depth: DEFAULT_JSON_DEPTH,
            max_json_entries: DEFAULT_JSON_ENTRIES,
            max_request_items: DEFAULT_ITEMS,
            max_response_items: DEFAULT_ITEMS,
            max_context_lifetime_ms: DEFAULT_CONTEXT_LIFETIME_MS,
            request_timeout_ms: DEFAULT_REQUEST_TIMEOUT_MS,
            max_queued_per_island: DEFAULT_QUEUED_PER_ISLAND,
            max_parallel_per_island: DEFAULT_PARALLEL_PER_ISLAND,
            morph_max_nodes: DEFAULT_MORPH_MAX_NODES,
            morph_max_depth: DEFAULT_MORPH_MAX_DEPTH,
            morph_max_keys: DEFAULT_MORPH_MAX_KEYS,
            morph_max_attributes: DEFAULT_MORPH_MAX_ATTRIBUTES,
            morph_max_attributes_per_element: None,
            morph_deadline_ms: DEFAULT_MORPH_DEADLINE_MS,
            async_max_payload_bytes: None,
            async_max_buffer_bytes: DEFAULT_ASYNC_MAX_BUFFER_BYTES,
            async_max_queued_events: DEFAULT_ASYNC_MAX_QUEUED_EVENTS,
            async_max_replay_events: None,
            max_redirect_bytes: None,
            upload_chunk_bytes: None,
            upload_max_active: DEFAULT_UPLOAD_MAX_ACTIVE,
            upload_max_file_bytes: DEFAULT_UPLOAD_MAX_FILE_BYTES,
            upload_max_pending_files: None,
            upload_max_pending_bytes: None,
            upload_max_storage_bytes: None,
        }
    }

    /// Starts a builder from the `LIVE_*` keys in the environment, so code
    /// can adjust what the `.env` file set before building.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] naming the key when a value is not a whole
    /// number. Range rules are checked by [`Self::build`].
    pub fn from_env() -> Result<Self, FrameworkError> {
        Self::from_source(&|name| std::env::var(name).ok())
    }

    fn from_source(read: &dyn Fn(&str) -> Option<String>) -> Result<Self, FrameworkError> {
        let number = |key: &str| -> Result<Option<u64>, FrameworkError> {
            let Some(raw) = read(key).filter(|value| !value.trim().is_empty()) else {
                return Ok(None);
            };
            raw.trim().parse::<u64>().map(Some).map_err(|_| {
                let unit = LIVE_LIMIT_KEYS
                    .iter()
                    .find(|limit| limit.key == key)
                    .map_or("units", |limit| limit.unit);
                FrameworkError::internal(format!(
                    "Live configuration rejected: {key}={} is not a whole number of {unit}. \
                     Set it to a plain integer, such as {key}=16777216, in the application's \
                     .env file.",
                    raw.trim()
                ))
            })
        };
        // A value too large for its field saturates, so the range check in
        // `build` stays the one place that refuses it, with its rule.
        let bytes = |value: u64| usize::try_from(value).unwrap_or(usize::MAX);
        let narrow32 = |value: u64| u32::try_from(value).unwrap_or(u32::MAX);
        let narrow8 = |value: u64| u8::try_from(value).unwrap_or(u8::MAX);
        let mut builder = Self::new();
        if let Some(value) = number("LIVE_MAX_REQUEST_BYTES")? {
            builder.max_request_bytes = bytes(value);
        }
        if let Some(value) = number("LIVE_MAX_RESPONSE_BYTES")? {
            builder.max_response_bytes = Some(bytes(value));
        }
        if let Some(value) = number("LIVE_MAX_HTML_BYTES")? {
            builder.max_html_bytes = Some(bytes(value));
        }
        if let Some(value) = number("LIVE_MAX_JSON_DEPTH")? {
            builder.max_json_depth = bytes(value);
        }
        if let Some(value) = number("LIVE_MAX_JSON_ENTRIES")? {
            builder.max_json_entries = bytes(value);
        }
        if let Some(value) = number("LIVE_MAX_REQUEST_ITEMS")? {
            builder.max_request_items = bytes(value);
        }
        if let Some(value) = number("LIVE_MAX_RESPONSE_ITEMS")? {
            builder.max_response_items = bytes(value);
        }
        if let Some(value) = number("LIVE_MAX_CONTEXT_LIFETIME_MS")? {
            builder.max_context_lifetime_ms = value;
        }
        if let Some(value) = number("LIVE_REQUEST_TIMEOUT_MS")? {
            builder.request_timeout_ms = narrow32(value);
        }
        if let Some(value) = number("LIVE_MAX_QUEUED_PER_ISLAND")? {
            builder.max_queued_per_island = narrow8(value);
        }
        if let Some(value) = number("LIVE_MAX_PARALLEL_PER_ISLAND")? {
            builder.max_parallel_per_island = narrow8(value);
        }
        if let Some(value) = number("LIVE_MORPH_MAX_NODES")? {
            builder.morph_max_nodes = narrow32(value);
        }
        if let Some(value) = number("LIVE_MORPH_MAX_DEPTH")? {
            builder.morph_max_depth = narrow32(value);
        }
        if let Some(value) = number("LIVE_MORPH_MAX_KEYS")? {
            builder.morph_max_keys = narrow32(value);
        }
        if let Some(value) = number("LIVE_MORPH_MAX_ATTRIBUTES")? {
            builder.morph_max_attributes = narrow32(value);
        }
        if let Some(value) = number("LIVE_MORPH_MAX_ATTRIBUTES_PER_ELEMENT")? {
            builder.morph_max_attributes_per_element = Some(narrow32(value));
        }
        if let Some(value) = number("LIVE_MORPH_DEADLINE_MS")? {
            builder.morph_deadline_ms = narrow32(value);
        }
        if let Some(value) = number("LIVE_ASYNC_MAX_PAYLOAD_BYTES")? {
            builder.async_max_payload_bytes = Some(bytes(value));
        }
        if let Some(value) = number("LIVE_ASYNC_MAX_BUFFER_BYTES")? {
            builder.async_max_buffer_bytes = bytes(value);
        }
        if let Some(value) = number("LIVE_ASYNC_MAX_QUEUED_EVENTS")? {
            builder.async_max_queued_events = bytes(value);
        }
        if let Some(value) = number("LIVE_ASYNC_MAX_REPLAY_EVENTS")? {
            builder.async_max_replay_events = Some(bytes(value));
        }
        if let Some(value) = number("LIVE_MAX_REDIRECT_BYTES")? {
            builder.max_redirect_bytes = Some(bytes(value));
        }
        if let Some(value) = number("LIVE_UPLOAD_CHUNK_BYTES")? {
            builder.upload_chunk_bytes = Some(bytes(value));
        }
        if let Some(value) = number("LIVE_UPLOAD_MAX_ACTIVE")? {
            builder.upload_max_active = bytes(value);
        }
        if let Some(value) = number("LIVE_UPLOAD_MAX_FILE_BYTES")? {
            builder.upload_max_file_bytes = value;
        }
        if let Some(value) = number("LIVE_UPLOAD_MAX_PENDING_FILES")? {
            builder.upload_max_pending_files = Some(bytes(value));
        }
        if let Some(value) = number("LIVE_UPLOAD_MAX_PENDING_BYTES")? {
            builder.upload_max_pending_bytes = Some(value);
        }
        if let Some(value) = number("LIVE_UPLOAD_MAX_STORAGE_BYTES")? {
            builder.upload_max_storage_bytes = Some(value);
        }
        Ok(builder)
    }

    /// Sets the whole-request body limit (`LIVE_MAX_REQUEST_BYTES`).
    #[must_use]
    pub const fn max_request_bytes(mut self, max: usize) -> Self {
        self.max_request_bytes = max;
        self
    }

    /// Sets the whole-response body limit (`LIVE_MAX_RESPONSE_BYTES`).
    #[must_use]
    pub const fn max_response_bytes(mut self, max: usize) -> Self {
        self.max_response_bytes = Some(max);
        self
    }

    /// Sets the island HTML limit (`LIVE_MAX_HTML_BYTES`).
    #[must_use]
    pub const fn max_html_bytes(mut self, max: usize) -> Self {
        self.max_html_bytes = Some(max);
        self
    }

    /// Sets the JSON nesting limit (`LIVE_MAX_JSON_DEPTH`).
    #[must_use]
    pub const fn max_json_depth(mut self, max: usize) -> Self {
        self.max_json_depth = max;
        self
    }

    /// Sets the request JSON entry limit (`LIVE_MAX_JSON_ENTRIES`).
    #[must_use]
    pub const fn max_json_entries(mut self, max: usize) -> Self {
        self.max_json_entries = max;
        self
    }

    /// Sets the request collection limit (`LIVE_MAX_REQUEST_ITEMS`).
    #[must_use]
    pub const fn max_request_items(mut self, max: usize) -> Self {
        self.max_request_items = max;
        self
    }

    /// Sets the response collection limit (`LIVE_MAX_RESPONSE_ITEMS`).
    #[must_use]
    pub const fn max_response_items(mut self, max: usize) -> Self {
        self.max_response_items = max;
        self
    }

    /// Sets the trusted request context lifetime
    /// (`LIVE_MAX_CONTEXT_LIFETIME_MS`).
    #[must_use]
    pub const fn max_context_lifetime_ms(mut self, max: u64) -> Self {
        self.max_context_lifetime_ms = max;
        self
    }

    /// Sets the browser's request timeout (`LIVE_REQUEST_TIMEOUT_MS`).
    #[must_use]
    pub const fn request_timeout_ms(mut self, timeout: u32) -> Self {
        self.request_timeout_ms = timeout;
        self
    }

    /// Sets the per-island queue depth (`LIVE_MAX_QUEUED_PER_ISLAND`).
    #[must_use]
    pub const fn max_queued_per_island(mut self, max: u8) -> Self {
        self.max_queued_per_island = max;
        self
    }

    /// Sets the per-island parallel request count
    /// (`LIVE_MAX_PARALLEL_PER_ISLAND`).
    #[must_use]
    pub const fn max_parallel_per_island(mut self, max: u8) -> Self {
        self.max_parallel_per_island = max;
        self
    }

    /// Sets the morph node limit (`LIVE_MORPH_MAX_NODES`).
    #[must_use]
    pub const fn morph_max_nodes(mut self, max: u32) -> Self {
        self.morph_max_nodes = max;
        self
    }

    /// Sets the morph nesting limit (`LIVE_MORPH_MAX_DEPTH`).
    #[must_use]
    pub const fn morph_max_depth(mut self, max: u32) -> Self {
        self.morph_max_depth = max;
        self
    }

    /// Sets the morph key limit (`LIVE_MORPH_MAX_KEYS`).
    #[must_use]
    pub const fn morph_max_keys(mut self, max: u32) -> Self {
        self.morph_max_keys = max;
        self
    }

    /// Sets the island-wide morph attribute limit
    /// (`LIVE_MORPH_MAX_ATTRIBUTES`).
    #[must_use]
    pub const fn morph_max_attributes(mut self, max: u32) -> Self {
        self.morph_max_attributes = max;
        self
    }

    /// Sets the per-element morph attribute limit
    /// (`LIVE_MORPH_MAX_ATTRIBUTES_PER_ELEMENT`).
    #[must_use]
    pub const fn morph_max_attributes_per_element(mut self, max: u32) -> Self {
        self.morph_max_attributes_per_element = Some(max);
        self
    }

    /// Sets the morph deadline; `0` means none (`LIVE_MORPH_DEADLINE_MS`).
    #[must_use]
    pub const fn morph_deadline_ms(mut self, deadline: u32) -> Self {
        self.morph_deadline_ms = deadline;
        self
    }

    /// Sets the asynchronous payload limit (`LIVE_ASYNC_MAX_PAYLOAD_BYTES`).
    #[must_use]
    pub const fn async_max_payload_bytes(mut self, max: usize) -> Self {
        self.async_max_payload_bytes = Some(max);
        self
    }

    /// Sets the per-document asynchronous queue limit
    /// (`LIVE_ASYNC_MAX_BUFFER_BYTES`).
    #[must_use]
    pub const fn async_max_buffer_bytes(mut self, max: usize) -> Self {
        self.async_max_buffer_bytes = max;
        self
    }

    /// Sets the per-document asynchronous queue depth
    /// (`LIVE_ASYNC_MAX_QUEUED_EVENTS`).
    #[must_use]
    pub const fn async_max_queued_events(mut self, max: usize) -> Self {
        self.async_max_queued_events = max;
        self
    }

    /// Sets the events one reconnect replay may carry
    /// (`LIVE_ASYNC_MAX_REPLAY_EVENTS`).
    #[must_use]
    pub const fn async_max_replay_events(mut self, max: usize) -> Self {
        self.async_max_replay_events = Some(max);
        self
    }

    /// Sets the redirect URL limit (`LIVE_MAX_REDIRECT_BYTES`).
    #[must_use]
    pub const fn max_redirect_bytes(mut self, max: usize) -> Self {
        self.max_redirect_bytes = Some(max);
        self
    }

    /// Sets the upload chunk size (`LIVE_UPLOAD_CHUNK_BYTES`).
    #[must_use]
    pub const fn upload_chunk_bytes(mut self, bytes: usize) -> Self {
        self.upload_chunk_bytes = Some(bytes);
        self
    }

    /// Sets the upload transfers running at once (`LIVE_UPLOAD_MAX_ACTIVE`).
    #[must_use]
    pub const fn upload_max_active(mut self, max: usize) -> Self {
        self.upload_max_active = max;
        self
    }

    /// Sets the upload file size limit (`LIVE_UPLOAD_MAX_FILE_BYTES`).
    #[must_use]
    pub const fn upload_max_file_bytes(mut self, max: u64) -> Self {
        self.upload_max_file_bytes = max;
        self
    }

    /// Sets the unfinished files one visitor may hold
    /// (`LIVE_UPLOAD_MAX_PENDING_FILES`).
    #[must_use]
    pub const fn upload_max_pending_files(mut self, max: usize) -> Self {
        self.upload_max_pending_files = Some(max);
        self
    }

    /// Sets the bytes across one visitor's unfinished files
    /// (`LIVE_UPLOAD_MAX_PENDING_BYTES`).
    #[must_use]
    pub const fn upload_max_pending_bytes(mut self, max: u64) -> Self {
        self.upload_max_pending_bytes = Some(max);
        self
    }

    /// Sets the temporary upload store's size (`LIVE_UPLOAD_MAX_STORAGE_BYTES`).
    #[must_use]
    pub const fn upload_max_storage_bytes(mut self, max: u64) -> Self {
        self.upload_max_storage_bytes = Some(max);
        self
    }

    /// Validates every limit and creates the immutable configuration.
    ///
    /// # Errors
    ///
    /// Returns [`LiveConfigError`] naming the first key whose value breaks
    /// its rule.
    pub fn build(self) -> Result<LiveConfig, LiveConfigError> {
        use LiveConfigErrorKind as Kind;
        let request = self.max_request_bytes;
        check(
            (1..=HARD_MAX_CONTROL_BYTES).contains(&request),
            Kind::InvalidByteLimits,
            "LIVE_MAX_REQUEST_BYTES",
            "must be from 1 to 1073741824 bytes",
        )?;
        let response = self
            .max_response_bytes
            .unwrap_or(DEFAULT_CONTROL_BYTES.min(request));
        check(
            response >= 1 && response <= request,
            Kind::InvalidByteLimits,
            "LIVE_MAX_RESPONSE_BYTES",
            "must be from 1 byte to LIVE_MAX_REQUEST_BYTES: a response's snapshot comes back in \
             the next request",
        )?;
        let html = self
            .max_html_bytes
            .unwrap_or(response.min(DEFAULT_CONTROL_BYTES));
        check(
            html >= 1 && html <= response,
            Kind::InvalidByteLimits,
            "LIVE_MAX_HTML_BYTES",
            "must be from 1 byte to LIVE_MAX_RESPONSE_BYTES: the island HTML travels inside the \
             response",
        )?;
        check(
            (1..=HARD_MAX_JSON_DEPTH).contains(&self.max_json_depth),
            Kind::InvalidJsonLimits,
            "LIVE_MAX_JSON_DEPTH",
            "must be from 1 to 64 levels: the parser is recursive and 64 is its stack ceiling",
        )?;
        check(
            (1..=HARD_MAX_JSON_ENTRIES).contains(&self.max_json_entries),
            Kind::InvalidJsonLimits,
            "LIVE_MAX_JSON_ENTRIES",
            "must be from 1 to 100000000 entries",
        )?;
        check(
            (1..=HARD_MAX_ITEMS).contains(&self.max_request_items),
            Kind::InvalidItemLimits,
            "LIVE_MAX_REQUEST_ITEMS",
            "must be from 1 to 16777216 items",
        )?;
        check(
            (1..=HARD_MAX_ITEMS).contains(&self.max_response_items),
            Kind::InvalidItemLimits,
            "LIVE_MAX_RESPONSE_ITEMS",
            "must be from 1 to 16777216 items",
        )?;
        check(
            (1..=HARD_MAX_CONTEXT_LIFETIME_MS).contains(&self.max_context_lifetime_ms),
            Kind::InvalidContextLifetime,
            "LIVE_MAX_CONTEXT_LIFETIME_MS",
            "must be from 1 to 300000 ms, the engine's ceiling on a trusted context",
        )?;
        check(
            (1..=HARD_MAX_TIMER_MS).contains(&self.request_timeout_ms),
            Kind::InvalidRequestTimeout,
            "LIVE_REQUEST_TIMEOUT_MS",
            "must be from 1 to 2147483647 ms, the longest delay a browser timer holds",
        )?;
        check(
            (1..=HARD_MAX_QUEUED_PER_ISLAND).contains(&self.max_queued_per_island),
            Kind::InvalidIslandScheduling,
            "LIVE_MAX_QUEUED_PER_ISLAND",
            "must be from 1 to 64 requests, the browser scheduler's per-island queue",
        )?;
        check(
            (1..=HARD_MAX_PARALLEL_PER_ISLAND).contains(&self.max_parallel_per_island)
                && self.max_parallel_per_island <= self.max_queued_per_island,
            Kind::InvalidIslandScheduling,
            "LIVE_MAX_PARALLEL_PER_ISLAND",
            "must be from 1 to 8 requests and at most LIVE_MAX_QUEUED_PER_ISLAND",
        )?;
        check(
            (1..=HARD_MAX_MORPH_COUNT).contains(&self.morph_max_nodes),
            Kind::InvalidMorphLimits,
            "LIVE_MORPH_MAX_NODES",
            "must be from 1 to 1073741824 nodes",
        )?;
        check(
            (1..=HARD_MAX_MORPH_DEPTH).contains(&self.morph_max_depth),
            Kind::InvalidMorphLimits,
            "LIVE_MORPH_MAX_DEPTH",
            "must be from 1 to 4096 levels: the morph walks the tree recursively",
        )?;
        check(
            (1..=HARD_MAX_MORPH_COUNT).contains(&self.morph_max_keys),
            Kind::InvalidMorphLimits,
            "LIVE_MORPH_MAX_KEYS",
            "must be from 1 to 1073741824 keyed elements",
        )?;
        check(
            (1..=HARD_MAX_MORPH_COUNT).contains(&self.morph_max_attributes),
            Kind::InvalidMorphLimits,
            "LIVE_MORPH_MAX_ATTRIBUTES",
            "must be from 1 to 1073741824 attributes",
        )?;
        let per_element = self
            .morph_max_attributes_per_element
            .unwrap_or(DEFAULT_MORPH_MAX_ATTRIBUTES_PER_ELEMENT.min(self.morph_max_attributes));
        check(
            per_element >= 1 && per_element <= self.morph_max_attributes,
            Kind::InvalidMorphLimits,
            "LIVE_MORPH_MAX_ATTRIBUTES_PER_ELEMENT",
            "must be from 1 attribute to LIVE_MORPH_MAX_ATTRIBUTES",
        )?;
        check(
            self.morph_deadline_ms <= HARD_MAX_TIMER_MS,
            Kind::InvalidMorphLimits,
            "LIVE_MORPH_DEADLINE_MS",
            "must be from 0 (no deadline) to 2147483647 ms",
        )?;
        check(
            (1..=HARD_MAX_ASYNC_BUFFER_BYTES).contains(&self.async_max_buffer_bytes),
            Kind::InvalidAsyncLimits,
            "LIVE_ASYNC_MAX_BUFFER_BYTES",
            "must be from 1 to 1073741824 bytes",
        )?;
        let payload = self
            .async_max_payload_bytes
            .unwrap_or(DEFAULT_ASYNC_MAX_PAYLOAD_BYTES.min(self.async_max_buffer_bytes));
        check(
            (1..=HARD_MAX_ASYNC_PAYLOAD_BYTES).contains(&payload)
                && payload <= self.async_max_buffer_bytes,
            Kind::InvalidAsyncLimits,
            "LIVE_ASYNC_MAX_PAYLOAD_BYTES",
            "must be from 1 byte to 16777216 bytes and at most LIVE_ASYNC_MAX_BUFFER_BYTES: a \
             payload is queued whole",
        )?;
        let queued = self.async_max_queued_events;
        check(
            (1..=HARD_MAX_ASYNC_EVENTS).contains(&queued),
            Kind::InvalidAsyncLimits,
            "LIVE_ASYNC_MAX_QUEUED_EVENTS",
            "must be from 1 to 65536 events, the engine's ceiling on one document's queue",
        )?;
        let replay = self
            .async_max_replay_events
            .unwrap_or(DEFAULT_ASYNC_MAX_REPLAY_EVENTS.min(queued));
        check(
            replay >= 1 && replay <= queued,
            Kind::InvalidAsyncLimits,
            "LIVE_ASYNC_MAX_REPLAY_EVENTS",
            "must be from 1 event to LIVE_ASYNC_MAX_QUEUED_EVENTS: a replay is queued whole",
        )?;
        let redirect = self
            .max_redirect_bytes
            .unwrap_or(DEFAULT_MAX_REDIRECT_BYTES.min(response));
        check(
            (1..=HARD_MAX_REDIRECT_BYTES).contains(&redirect) && redirect <= response,
            Kind::InvalidByteLimits,
            "LIVE_MAX_REDIRECT_BYTES",
            "must be from 1 byte to 2097152 bytes, the longest URL a browser follows, and at \
             most LIVE_MAX_RESPONSE_BYTES: the URL travels inside the response",
        )?;
        let upload = self.build_upload()?;
        Ok(LiveConfig {
            max_request_bytes: request,
            max_response_bytes: response,
            max_html_bytes: html,
            max_json_depth: self.max_json_depth,
            max_json_entries: self.max_json_entries,
            max_request_items: self.max_request_items,
            max_response_items: self.max_response_items,
            max_context_lifetime_ms: self.max_context_lifetime_ms,
            request_timeout_ms: self.request_timeout_ms,
            max_queued_per_island: self.max_queued_per_island,
            max_parallel_per_island: self.max_parallel_per_island,
            morph: LiveMorphLimits {
                max_nodes: self.morph_max_nodes,
                max_depth: self.morph_max_depth,
                max_keys: self.morph_max_keys,
                max_attributes: self.morph_max_attributes,
                max_attributes_per_element: per_element,
                deadline_ms: self.morph_deadline_ms,
            },
            async_max_payload_bytes: payload,
            async_max_buffer_bytes: self.async_max_buffer_bytes,
            async_max_queued_events: queued,
            async_max_replay_events: replay,
            max_redirect_bytes: redirect,
            upload,
        })
    }

    /// Validates the upload limits. An unset limit follows the one it must
    /// hold: the chunk follows the file size down, and the pending files,
    /// pending bytes and store size follow up to the limits they must hold.
    fn build_upload(self) -> Result<LiveUploadLimits, LiveConfigError> {
        use LiveConfigErrorKind as Kind;
        let file = self.upload_max_file_bytes;
        check(
            (1..=HARD_MAX_UPLOAD_FILE_BYTES).contains(&file),
            Kind::InvalidUploadLimits,
            "LIVE_UPLOAD_MAX_FILE_BYTES",
            "must be from 1 to 1099511627776 bytes (1 TiB)",
        )?;
        let chunk = self.upload_chunk_bytes.unwrap_or_else(|| {
            let fitted = DEFAULT_UPLOAD_CHUNK_BYTES as u64;
            usize::try_from(fitted.min(file)).unwrap_or(DEFAULT_UPLOAD_CHUNK_BYTES)
        });
        check(
            (1..=HARD_MAX_UPLOAD_CHUNK_BYTES).contains(&chunk)
                && chunk as u64 <= file
                && file.div_ceil(chunk as u64) <= HARD_MAX_UPLOAD_CHUNKS_PER_FILE,
            Kind::InvalidUploadLimits,
            "LIVE_UPLOAD_CHUNK_BYTES",
            "must be from 1 to 67108864 bytes, at most LIVE_UPLOAD_MAX_FILE_BYTES, and large \
             enough that a file of LIVE_UPLOAD_MAX_FILE_BYTES takes at most 99994 chunks",
        )?;
        let active = self.upload_max_active;
        check(
            (1..=HARD_MAX_UPLOAD_ACTIVE).contains(&active)
                && (chunk as u64).saturating_mul(active as u64) <= HARD_MAX_UPLOAD_IN_FLIGHT_BYTES,
            Kind::InvalidUploadLimits,
            "LIVE_UPLOAD_MAX_ACTIVE",
            "must be from 1 to 1024 transfers, and times LIVE_UPLOAD_CHUNK_BYTES at most \
             1073741824 bytes: each running transfer holds one chunk in memory",
        )?;
        let files = self
            .upload_max_pending_files
            .unwrap_or(DEFAULT_UPLOAD_MAX_PENDING_FILES.max(active));
        check(
            (1..=HARD_MAX_UPLOAD_PENDING_FILES).contains(&files) && files >= active,
            Kind::InvalidUploadLimits,
            "LIVE_UPLOAD_MAX_PENDING_FILES",
            "must be from LIVE_UPLOAD_MAX_ACTIVE to 100000 files: every running transfer is a \
             pending file",
        )?;
        let pending = self
            .upload_max_pending_bytes
            .unwrap_or(DEFAULT_UPLOAD_MAX_PENDING_BYTES.max(file));
        check(
            pending >= file && pending <= HARD_MAX_UPLOAD_PENDING_BYTES,
            Kind::InvalidUploadLimits,
            "LIVE_UPLOAD_MAX_PENDING_BYTES",
            "must be from LIVE_UPLOAD_MAX_FILE_BYTES to 17592186044416 bytes (16 TiB): one file \
             is pending on its own",
        )?;
        let storage = self
            .upload_max_storage_bytes
            .unwrap_or(DEFAULT_UPLOAD_MAX_STORAGE_BYTES.max(pending));
        check(
            storage >= pending && storage <= HARD_MAX_UPLOAD_STORAGE_BYTES,
            Kind::InvalidUploadLimits,
            "LIVE_UPLOAD_MAX_STORAGE_BYTES",
            "must be from LIVE_UPLOAD_MAX_PENDING_BYTES to 70368744177664 bytes (64 TiB): the \
             store holds every visitor's pending files",
        )?;
        Ok(LiveUploadLimits {
            chunk_bytes: chunk,
            max_active: active,
            max_file_bytes: file,
            max_pending_files: files,
            max_pending_bytes: pending,
            max_storage_bytes: storage,
        })
    }
}

fn check(
    valid: bool,
    kind: LiveConfigErrorKind,
    key: &'static str,
    rule: &'static str,
) -> Result<(), LiveConfigError> {
    if valid {
        Ok(())
    } else {
        Err(LiveConfigError { kind, key, rule })
    }
}

impl Default for LiveConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Closed reason Live startup configuration was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LiveConfigErrorKind {
    /// A request, response or island HTML byte limit was zero, above the
    /// hard ceiling, or larger than the limit it must fit inside.
    InvalidByteLimits,
    /// The trusted request-context lifetime was zero or exceeded the engine ceiling.
    InvalidContextLifetime,
    /// A JSON nesting or entry limit was zero or above its ceiling.
    InvalidJsonLimits,
    /// A request or response collection limit was zero or above its ceiling.
    InvalidItemLimits,
    /// The browser request timeout was zero or longer than a timer can hold.
    InvalidRequestTimeout,
    /// A per-island queue or parallel request count was out of range.
    InvalidIslandScheduling,
    /// A morph node, depth, key, attribute or deadline limit was out of range.
    InvalidMorphLimits,
    /// An asynchronous payload, queue or replay limit was out of range.
    InvalidAsyncLimits,
    /// An upload chunk, transfer, file, pending or store limit was out of
    /// range.
    InvalidUploadLimits,
}

impl LiveConfigErrorKind {
    /// Returns the stable machine-readable failure value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidByteLimits => "invalid_live_byte_limits",
            Self::InvalidContextLifetime => "invalid_live_context_lifetime",
            Self::InvalidJsonLimits => "invalid_live_json_limits",
            Self::InvalidItemLimits => "invalid_live_item_limits",
            Self::InvalidRequestTimeout => "invalid_live_request_timeout",
            Self::InvalidIslandScheduling => "invalid_live_island_scheduling",
            Self::InvalidMorphLimits => "invalid_live_morph_limits",
            Self::InvalidAsyncLimits => "invalid_live_async_limits",
            Self::InvalidUploadLimits => "invalid_live_upload_limits",
        }
    }
}

/// Live configuration failure, naming the key whose value broke its rule.
///
/// It carries only the key and the rule, never the rejected value, so it is
/// `Copy`; [`LiveConfig::from_env`] adds the value to its message.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct LiveConfigError {
    kind: LiveConfigErrorKind,
    key: &'static str,
    rule: &'static str,
}

impl LiveConfigError {
    /// Returns the closed configuration failure category.
    #[must_use]
    pub const fn kind(self) -> LiveConfigErrorKind {
        self.kind
    }

    /// Returns the configuration key whose value broke its rule, such as
    /// `LIVE_MAX_HTML_BYTES`.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.key
    }

    /// Returns the rule the value broke, in words.
    #[must_use]
    pub const fn rule(self) -> &'static str {
        self.rule
    }
}

impl fmt::Display for LiveConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: {} {}",
            self.kind.as_str(),
            self.key,
            self.rule
        )
    }
}

impl fmt::Debug for LiveConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for LiveConfigError {}

#[cfg(test)]
mod limit_tests {
    //! Every Live limit: its default, its key, its rule, and the message a
    //! developer reads when a value is wrong or a limit trips.
    use super::*;

    const MIB: usize = 1024 * 1024;

    fn source(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn parsed(pairs: &[(&str, &str)]) -> Result<LiveConfig, FrameworkError> {
        let table = source(pairs);
        LiveConfig::from_source(&|name| table.get(name).cloned())
    }

    #[test]
    fn the_defaults_are_sized_for_large_modern_pages() {
        let config = LiveConfig::standard();
        assert_eq!(config.max_request_bytes(), 16 * MIB);
        assert_eq!(config.max_response_bytes(), 16 * MIB);
        assert_eq!(config.max_html_bytes(), 16 * MIB);
        assert_eq!(config.max_json_depth(), 32);
        assert_eq!(config.max_json_entries(), 1_000_000);
        assert_eq!(config.max_request_items(), 65_536);
        assert_eq!(config.max_response_items(), 65_536);
        assert_eq!(config.request_timeout_ms(), 60_000);
        assert_eq!(config.morph().max_nodes(), 1_000_000);
        assert_eq!(config.morph().max_keys(), 1_000_000);
        assert_eq!(config.morph().max_attributes(), 10_000_000);
        assert_eq!(config.morph().max_attributes_per_element(), 4_096);
        assert_eq!(config.morph().max_depth(), 512);
        assert_eq!(config.morph().deadline_ms(), 0);
        assert_eq!(config.async_max_payload_bytes(), MIB);
        assert_eq!(config.async_max_buffer_bytes(), 16 * MIB);
        assert_eq!(config.async_max_queued_events(), 4_096);
        assert_eq!(config.async_max_replay_events(), 4_096);
        assert_eq!(config.max_redirect_bytes(), 64 * 1024);
        let upload = config.upload();
        assert_eq!(upload.chunk_bytes(), 8 * MIB);
        assert_eq!(upload.max_active(), 8);
        assert_eq!(upload.max_file_bytes(), 1024 * 1024 * 1024);
        assert_eq!(upload.max_pending_files(), 1_024);
        assert_eq!(upload.max_pending_bytes(), 4 * 1024 * 1024 * 1024);
        assert_eq!(upload.max_storage_bytes(), 16 * 1024 * 1024 * 1024);
        assert_eq!(LiveConfig::builder().build(), Ok(config));
        assert_eq!(
            parsed(&[]).expect("an empty environment is the defaults"),
            config
        );
    }

    #[test]
    fn every_key_is_read_from_the_environment() {
        let config = parsed(&[
            ("LIVE_MAX_REQUEST_BYTES", "1073741824"),
            ("LIVE_MAX_RESPONSE_BYTES", "536870912"),
            ("LIVE_MAX_HTML_BYTES", "268435456"),
            ("LIVE_MAX_JSON_DEPTH", "64"),
            ("LIVE_MAX_JSON_ENTRIES", "5000000"),
            ("LIVE_MAX_REQUEST_ITEMS", "100000"),
            ("LIVE_MAX_RESPONSE_ITEMS", "200000"),
            ("LIVE_MAX_CONTEXT_LIFETIME_MS", "60000"),
            ("LIVE_REQUEST_TIMEOUT_MS", "120000"),
            ("LIVE_MAX_QUEUED_PER_ISLAND", "16"),
            ("LIVE_MAX_PARALLEL_PER_ISLAND", "2"),
            ("LIVE_MORPH_MAX_NODES", "5000000"),
            ("LIVE_MORPH_MAX_DEPTH", "1024"),
            ("LIVE_MORPH_MAX_KEYS", "4000000"),
            ("LIVE_MORPH_MAX_ATTRIBUTES", "50000000"),
            ("LIVE_MORPH_MAX_ATTRIBUTES_PER_ELEMENT", "8192"),
            ("LIVE_MORPH_DEADLINE_MS", "30000"),
            ("LIVE_ASYNC_MAX_PAYLOAD_BYTES", "8388608"),
            ("LIVE_ASYNC_MAX_BUFFER_BYTES", "67108864"),
            ("LIVE_ASYNC_MAX_QUEUED_EVENTS", "65536"),
            ("LIVE_ASYNC_MAX_REPLAY_EVENTS", "20000"),
            ("LIVE_MAX_REDIRECT_BYTES", "2097152"),
            ("LIVE_UPLOAD_CHUNK_BYTES", "33554432"),
            ("LIVE_UPLOAD_MAX_ACTIVE", "32"),
            ("LIVE_UPLOAD_MAX_FILE_BYTES", "1099511627776"),
            ("LIVE_UPLOAD_MAX_PENDING_FILES", "100000"),
            ("LIVE_UPLOAD_MAX_PENDING_BYTES", "4398046511104"),
            ("LIVE_UPLOAD_MAX_STORAGE_BYTES", "8796093022208"),
        ])
        .expect("every value is in range");
        assert_eq!(config.max_request_bytes(), 1024 * MIB);
        assert_eq!(config.max_response_bytes(), 512 * MIB);
        assert_eq!(config.max_html_bytes(), 256 * MIB);
        assert_eq!(config.max_json_depth(), 64);
        assert_eq!(config.max_json_entries(), 5_000_000);
        assert_eq!(config.max_request_items(), 100_000);
        assert_eq!(config.max_response_items(), 200_000);
        assert_eq!(config.max_context_lifetime_ms(), 60_000);
        assert_eq!(config.request_timeout_ms(), 120_000);
        assert_eq!(config.max_queued_per_island(), 16);
        assert_eq!(config.max_parallel_per_island(), 2);
        assert_eq!(config.morph().max_nodes(), 5_000_000);
        assert_eq!(config.morph().max_depth(), 1_024);
        assert_eq!(config.morph().max_keys(), 4_000_000);
        assert_eq!(config.morph().max_attributes(), 50_000_000);
        assert_eq!(config.morph().max_attributes_per_element(), 8_192);
        assert_eq!(config.morph().deadline_ms(), 30_000);
        assert_eq!(config.async_max_payload_bytes(), 8 * MIB);
        assert_eq!(config.async_max_buffer_bytes(), 64 * MIB);
        assert_eq!(config.async_max_queued_events(), 65_536);
        assert_eq!(config.async_max_replay_events(), 20_000);
        assert_eq!(config.max_redirect_bytes(), 2 * MIB);
        let upload = config.upload();
        assert_eq!(upload.chunk_bytes(), 32 * MIB);
        assert_eq!(upload.max_active(), 32);
        assert_eq!(upload.max_file_bytes(), 1 << 40);
        assert_eq!(upload.max_pending_files(), 100_000);
        assert_eq!(upload.max_pending_bytes(), 4 << 40);
        assert_eq!(upload.max_storage_bytes(), 8 << 40);
        let engine = config
            .engine_upload_limits()
            .expect("the engine accepts the profile");
        assert_eq!(engine.max_chunk_bytes(), 32 * MIB);
        assert_eq!(engine.max_file_bytes(), 1 << 40);
        assert_eq!(engine.max_concurrent_transfers(), 32);
        assert_eq!(engine.max_files_per_field(), 100_000);
    }

    #[test]
    fn an_unset_limit_follows_the_one_it_must_fit_inside() {
        let config = parsed(&[("LIVE_MAX_REQUEST_BYTES", "262144")]).expect("a small request");
        assert_eq!(config.max_response_bytes(), 262_144);
        assert_eq!(config.max_html_bytes(), 262_144);
        let config = parsed(&[("LIVE_MORPH_MAX_ATTRIBUTES", "100")]).expect("few attributes");
        assert_eq!(config.morph().max_attributes_per_element(), 100);
        let config = parsed(&[("LIVE_ASYNC_MAX_BUFFER_BYTES", "4096")]).expect("a small queue");
        assert_eq!(config.async_max_payload_bytes(), 4_096);
        let config = parsed(&[("LIVE_ASYNC_MAX_QUEUED_EVENTS", "64")]).expect("a short queue");
        assert_eq!(config.async_max_replay_events(), 64);
        let config = parsed(&[("LIVE_MAX_REQUEST_BYTES", "4096")]).expect("a small request");
        assert_eq!(config.max_redirect_bytes(), 4_096);
        let config = parsed(&[("LIVE_UPLOAD_MAX_FILE_BYTES", "1048576")]).expect("small files");
        assert_eq!(config.upload().chunk_bytes(), 1_048_576);
        let config = parsed(&[("LIVE_UPLOAD_MAX_FILE_BYTES", "17179869184")]).expect("16 GiB");
        assert_eq!(config.upload().max_pending_bytes(), 17_179_869_184);
        assert_eq!(config.upload().max_storage_bytes(), 17_179_869_184);
        let config = parsed(&[
            ("LIVE_UPLOAD_CHUNK_BYTES", "1048576"),
            ("LIVE_UPLOAD_MAX_ACTIVE", "1024"),
        ])
        .expect("many transfers");
        assert_eq!(config.upload().max_pending_files(), 1_024);
    }

    #[test]
    fn a_value_that_is_not_a_number_names_the_key_and_the_value() {
        let message = parsed(&[("LIVE_MAX_HTML_BYTES", "16MiB")])
            .expect_err("a unit suffix is not a number")
            .to_string();
        assert!(message.contains("LIVE_MAX_HTML_BYTES=16MiB"), "{message}");
        assert!(
            message.contains("is not a whole number of bytes"),
            "{message}"
        );
        assert!(message.contains(".env"), "{message}");
    }

    #[test]
    fn a_value_out_of_range_names_the_key_the_value_and_the_rule() {
        let message = parsed(&[("LIVE_MAX_REQUEST_BYTES", "1073741825")])
            .expect_err("above the hard maximum")
            .to_string();
        assert!(
            message.contains("LIVE_MAX_REQUEST_BYTES=1073741825"),
            "{message}"
        );
        assert!(message.contains("from 1 to 1073741824 bytes"), "{message}");

        let message = parsed(&[
            ("LIVE_MAX_RESPONSE_BYTES", "1048576"),
            ("LIVE_MAX_HTML_BYTES", "2097152"),
        ])
        .expect_err("the island HTML cannot outgrow the response")
        .to_string();
        assert!(message.contains("LIVE_MAX_HTML_BYTES=2097152"), "{message}");
        assert!(message.contains("LIVE_MAX_RESPONSE_BYTES"), "{message}");

        let message = parsed(&[("LIVE_MAX_RESPONSE_BYTES", "33554432")])
            .expect_err("a response above the default request limit")
            .to_string();
        assert!(
            message.contains("LIVE_MAX_RESPONSE_BYTES=33554432"),
            "{message}"
        );

        for (key, value) in [
            ("LIVE_MAX_JSON_DEPTH", "65"),
            ("LIVE_MAX_JSON_ENTRIES", "0"),
            ("LIVE_MAX_REQUEST_ITEMS", "0"),
            ("LIVE_REQUEST_TIMEOUT_MS", "2147483648"),
            ("LIVE_MAX_QUEUED_PER_ISLAND", "65"),
            ("LIVE_MORPH_MAX_NODES", "0"),
            ("LIVE_MORPH_MAX_DEPTH", "4097"),
            ("LIVE_MORPH_DEADLINE_MS", "2147483648"),
            ("LIVE_ASYNC_MAX_PAYLOAD_BYTES", "16777217"),
            ("LIVE_ASYNC_MAX_QUEUED_EVENTS", "65537"),
            ("LIVE_ASYNC_MAX_REPLAY_EVENTS", "4097"),
            ("LIVE_MAX_REDIRECT_BYTES", "2097153"),
            ("LIVE_UPLOAD_CHUNK_BYTES", "67108865"),
            ("LIVE_UPLOAD_CHUNK_BYTES", "1024"),
            ("LIVE_UPLOAD_MAX_ACTIVE", "129"),
            ("LIVE_UPLOAD_MAX_FILE_BYTES", "1099511627777"),
            ("LIVE_UPLOAD_MAX_PENDING_FILES", "4"),
            ("LIVE_UPLOAD_MAX_PENDING_BYTES", "1024"),
            ("LIVE_UPLOAD_MAX_STORAGE_BYTES", "1024"),
        ] {
            let message = parsed(&[(key, value)])
                .expect_err("out of range")
                .to_string();
            assert!(message.contains(&format!("{key}={value}")), "{message}");
        }
    }

    #[test]
    fn the_builder_reports_the_key_and_rule_it_broke() {
        let error = LiveConfig::builder()
            .max_html_bytes(32 * MIB)
            .build()
            .expect_err("the island HTML cannot outgrow the response");
        assert_eq!(error.kind(), LiveConfigErrorKind::InvalidByteLimits);
        assert_eq!(error.key(), "LIVE_MAX_HTML_BYTES");
        assert!(
            error
                .to_string()
                .starts_with("invalid_live_byte_limits: LIVE_MAX_HTML_BYTES")
        );
        let error = LiveConfig::builder()
            .morph_max_attributes(10)
            .morph_max_attributes_per_element(11)
            .build()
            .expect_err("per element cannot exceed the island");
        assert_eq!(error.key(), "LIVE_MORPH_MAX_ATTRIBUTES_PER_ELEMENT");
        assert_eq!(error.kind(), LiveConfigErrorKind::InvalidMorphLimits);
    }

    #[test]
    fn a_tripped_limit_names_the_limit_both_values_and_the_key() {
        assert_eq!(
            LiveLimitExceeded::html_bytes(2_097_152, 1_048_576, false).to_string(),
            "Suprnova Live island HTML size limit exceeded: measured 2097152 bytes, configured \
             1048576 bytes. Raise LIVE_MAX_HTML_BYTES in the application's .env file to allow it."
        );
        let request = LiveLimitExceeded::request_bytes(16_777_217, 16_777_216, true).to_string();
        assert!(request.contains("request size limit exceeded"), "{request}");
        assert!(
            request.contains("measured at least 16777217 bytes"),
            "{request}"
        );
        assert!(request.contains("LIVE_MAX_REQUEST_BYTES"), "{request}");
        let response = LiveLimitExceeded::response_bytes(20, 10, false).to_string();
        assert!(response.contains("LIVE_MAX_RESPONSE_BYTES"), "{response}");
        let redirect = LiveLimitExceeded::redirect_bytes(70_000, 65_536).to_string();
        assert_eq!(
            redirect,
            "Suprnova Live redirect URL size limit exceeded: measured 70000 bytes, configured \
             65536 bytes. Raise LIVE_MAX_REDIRECT_BYTES in the application's .env file to allow it."
        );
        let file = LiveLimitExceeded::upload_file_bytes(9, 8).to_string();
        assert!(
            file.contains("upload file size limit exceeded: measured 9 bytes, configured 8 bytes"),
            "{file}"
        );
        assert!(file.contains("LIVE_UPLOAD_MAX_FILE_BYTES"), "{file}");
        let chunk = LiveLimitExceeded::upload_chunk_bytes(9, 8, true).to_string();
        assert!(chunk.contains("measured at least 9 bytes"), "{chunk}");
        assert!(chunk.contains("LIVE_UPLOAD_CHUNK_BYTES"), "{chunk}");
        let payload = LiveLimitExceeded::async_payload_bytes(2_000_000, 1_048_576).to_string();
        assert!(payload.contains("async payload size"), "{payload}");
        assert!(
            payload.contains("LIVE_ASYNC_MAX_PAYLOAD_BYTES"),
            "{payload}"
        );
    }

    #[test]
    fn every_limit_is_reported_by_its_key_with_its_configured_value() {
        let config = parsed(&[
            ("LIVE_MAX_HTML_BYTES", "1048576"),
            ("LIVE_UPLOAD_MAX_FILE_BYTES", "17179869184"),
            ("LIVE_ASYNC_MAX_REPLAY_EVENTS", "100"),
        ])
        .expect("in range");
        let values = config.limit_values();
        assert_eq!(values.len(), LIVE_LIMIT_KEYS.len());
        for expected in [
            ("LIVE_MAX_HTML_BYTES", "bytes", 1_048_576),
            ("LIVE_UPLOAD_MAX_FILE_BYTES", "bytes", 17_179_869_184),
            ("LIVE_UPLOAD_MAX_STORAGE_BYTES", "bytes", 17_179_869_184),
            ("LIVE_ASYNC_MAX_REPLAY_EVENTS", "events", 100),
            ("LIVE_MORPH_MAX_KEYS", "keyed elements", 1_000_000),
            ("LIVE_REQUEST_TIMEOUT_MS", "ms", 60_000),
        ] {
            assert!(
                values.contains(&expected),
                "{expected:?} missing from {values:?}"
            );
        }
    }

    #[test]
    fn every_key_is_documented_in_both_manual_chapters() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../manual");
        let env_vars = std::fs::read_to_string(root.join("env-vars.md")).expect("env-vars.md");
        let live = std::fs::read_to_string(root.join("live.md")).expect("live.md");
        for LiveLimitKey { key, .. } in LIVE_LIMIT_KEYS {
            assert!(
                env_vars.contains(&format!("`{key}`")),
                "{key} missing from env-vars.md"
            );
            assert!(
                live.contains(&format!("`{key}`")),
                "{key} missing from live.md"
            );
        }
    }
}

#[cfg(test)]
mod ledger_driver_tests {
    //! Which instance ledger a deployment runs, and what a rejected value
    //! may say.
    //!
    //! Every test reads a table rather than the process environment, for the
    //! reason `render_cache::config`'s own tests do: the environment is
    //! shared by every test in a binary and a table is not.
    use super::*;

    fn source(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn parsed(pairs: &[(&str, &str)]) -> LedgerDriver {
        let table = source(pairs);
        LedgerDriver::from_source(&|name| table.get(name).cloned())
            .expect("the fixture configuration parses")
    }

    #[test]
    fn the_memory_ledger_is_the_default_and_the_only_one_needing_nothing() {
        assert_eq!(parsed(&[]), LedgerDriver::Memory);
        assert_eq!(
            parsed(&[("LIVE_LEDGER_DRIVER", "memory")]),
            LedgerDriver::Memory
        );
    }

    #[test]
    fn the_distributed_drivers_carry_what_they_need_to_reach_their_store() {
        assert_eq!(
            parsed(&[("LIVE_LEDGER_DRIVER", "database")]),
            LedgerDriver::Database
        );
        assert_eq!(
            parsed(&[("LIVE_LEDGER_DRIVER", "redis")]),
            LedgerDriver::Redis {
                url: "redis://127.0.0.1:6379".to_owned(),
                prefix: "suprnova_live:".to_owned(),
            }
        );
        // The shared endpoint, then Live's own, then the prefix.
        assert_eq!(
            parsed(&[
                ("LIVE_LEDGER_DRIVER", "redis"),
                ("REDIS_URL", "redis://shared:6379"),
            ]),
            LedgerDriver::Redis {
                url: "redis://shared:6379".to_owned(),
                prefix: "suprnova_live:".to_owned(),
            }
        );
        assert_eq!(
            parsed(&[
                ("LIVE_LEDGER_DRIVER", "redis"),
                ("REDIS_URL", "redis://shared:6379"),
                ("LIVE_REDIS_URL", "redis://live:6379"),
                ("LIVE_REDIS_PREFIX", "tenant_a_live:"),
            ]),
            LedgerDriver::Redis {
                url: "redis://live:6379".to_owned(),
                prefix: "tenant_a_live:".to_owned(),
            }
        );
    }

    #[test]
    fn an_unknown_driver_names_the_variable_and_repeats_nothing() {
        let table = source(&[("LIVE_LEDGER_DRIVER", "cassandra")]);
        let message = LedgerDriver::from_source(&|name| table.get(name).cloned())
            .expect_err("an unknown driver is refused")
            .to_string();
        assert!(message.contains("LIVE_LEDGER_DRIVER"), "{message}");
        assert!(!message.contains("cassandra"), "{message}");
    }

    #[test]
    fn a_live_redis_endpoint_never_reaches_a_printed_driver() {
        let printed = format!(
            "{:?}",
            parsed(&[
                ("LIVE_LEDGER_DRIVER", "redis"),
                ("LIVE_REDIS_URL", "redis://someone:hunter2@10.0.0.1:6379"),
            ])
        );
        assert!(printed.contains("redis://<redacted>"), "{printed}");
        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(!printed.contains("10.0.0.1"), "{printed}");
    }
}
