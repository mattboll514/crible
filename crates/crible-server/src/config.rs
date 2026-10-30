// SPDX-License-Identifier: Apache-2.0

//! Instance configuration: `crible.toml` and its `CRIBLE_*` overrides.
//!
//! References: `docs/07-stack-et-deploiement.md` ("Configuration de
//! l'instance"), ADR-0097 (form of an override, refusal at startup, closed list
//! of the variables documented outside the configuration, `crible serve
//! [--config]` and the default path), ADR-0051
//! (`[limits].query_cache_entries`) and ADR-0052 (`[encoder].query_precision`).
//!
//! The key list is **closed**: it is the one of `docs/07`, and no task adds a
//! key. Every key is required. A file that omits one does not start the server
//! on an implicit value, which is the reason ADR-0097 gives for refusing an
//! absent or unreadable file.
//!
//! An override follows ADR-0097: `CRIBLE_` followed by the key path in upper
//! case, the sections separated by `__`. The value is parsed as a scalar of the
//! type the targeted key already has. Three cases stop the startup, each with a
//! message that names the variable: the key does not exist, the value does not
//! parse in that type, or the variable targets a table of tables
//! (`[[auth.keys]]`). The seven variables documented outside the configuration
//! are ignored without error; any other unknown `CRIBLE_*` variable is a fault.
//! A refusal that comes from the key itself (a bound, an enumeration, an empty
//! value) names the variable too: the set of overrides is kept, and every
//! validation message of a key it set adds "set by `CRIBLE_…`".
//!
//! A `CRIBLE_*` variable whose name or value is not UTF-8 is refused at startup
//! (complement of 2026-10-29 to ADR-0097), so no override is ignored silently.
//!
//! A variable may hold a float when its key holds one (`sample_rate`). ADR-0097
//! lists integer, boolean, string and enumeration, and its complement of
//! 2026-10-29 adds the float: `sample_rate` would otherwise be the only key of
//! `docs/07` that no variable could override, against the first sentence of
//! ADR-0097 ("chaque clé peut être surchargée").

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crible_core::error::CribleError;
use crible_encode::Precision;
use toml::{Table, Value};

use crate::error::invalid;

/// Prefix of every override variable.
const VARIABLE_PREFIX: &str = "CRIBLE_";

/// Separator between the sections of a key path in a variable name.
const SECTION_SEPARATOR: &str = "__";

/// Variables that `CRIBLE_*` documents outside the configuration (ADR-0097).
///
/// The list is closed. These variables name no key of `crible.toml`, so they
/// escape the three refusals above and are ignored without error.
const EXCLUDED_VARIABLES: [&str; 7] = [
    "CRIBLE_FAILPOINT",
    "CRIBLE_BIN",
    "CRIBLE_PYTHON",
    "CRIBLE_EXPORT_REPRO",
    "CRIBLE_CRASH_CHILD",
    "CRIBLE_CRASH_BATCH",
    "CRIBLE_CRASH_INDEX",
];

/// Key path of `crible.toml` to the `CRIBLE_*` variable that set it.
type Overrides = BTreeMap<String, String>;

/// Top-level sections of `crible.toml`, the closed list of `docs/07`.
const SECTIONS: [&str; 7] = [
    "server",
    "paths",
    "auth",
    "analytics",
    "debug_journal",
    "encoder",
    "limits",
];

/// Role of an API key (`docs/05-api.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Search, document reads, index listing.
    Read,
    /// [`Role::Read`] plus documents, batches and jobs.
    Write,
    /// [`Role::Write`] plus index administration, compaction, export,
    /// snapshots, stats and the debug journal.
    Admin,
}

impl Role {
    /// Wire name used in `crible.toml` and in `docs/05-api.md`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Admin => "admin",
        }
    }

    /// Whether a key of this role reaches a route that requires `required`.
    ///
    /// `admin` contains `write`, which contains `read` (`docs/05-api.md`,
    /// ADR-0100 decision 5).
    #[must_use]
    pub const fn grants(self, required: Self) -> bool {
        self.rank() >= required.rank()
    }

    /// Level of the role: `admin` above `write`, `write` above `read`.
    const fn rank(self) -> u8 {
        match self {
            Self::Read => 0,
            Self::Write => 1,
            Self::Admin => 2,
        }
    }

    fn parse(text: &str, path: &str) -> Result<Self, CribleError> {
        match text {
            "read" => Ok(Self::Read),
            "write" => Ok(Self::Write),
            "admin" => Ok(Self::Admin),
            other => Err(invalid(format!(
                "unknown role '{other}' at {path}; expected read, write or admin"
            ))),
        }
    }
}

/// One declared API key.
///
/// The key itself is never stored: `crible.toml` holds the SHA-256 digest only,
/// which the authentication middleware of `AUTHENTIFIER-CLES` compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKey {
    /// Name of the key, attached to a request for the analytics.
    pub name: String,
    /// Role of the key.
    pub role: Role,
    /// SHA-256 digest of the secret, as `crible key generate` prints it.
    pub sha256: String,
}

/// `[server]` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// Address the server binds to, for example `0.0.0.0:8080`.
    pub bind: String,
    /// Threads of the search pool; zero means the number of cores.
    pub search_threads: usize,
    /// Concurrent searches allowed; zero means twice the number of cores.
    pub max_concurrent_searches: usize,
}

/// `[paths]` section.
///
/// The three roots come from the file: they are not deduced from its location,
/// and no command-line option replaces them (ADR-0097).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathsConfig {
    /// Root of the data of the instance, holding `indexes/` and `tmp/`.
    pub data: PathBuf,
    /// Root of the model directories.
    pub models: PathBuf,
    /// Root of the exchange directory: batches, exports and snapshots.
    pub exchange: PathBuf,
}

/// `[auth]` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthConfig {
    /// Declared keys, possibly none before `AUTHENTIFIER-CLES` exists.
    pub keys: Vec<ApiKey>,
}

/// `[analytics]` section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalyticsConfig {
    /// Retention of the aggregated rows, in days.
    pub retention_days: usize,
}

/// `[debug_journal]` section.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugJournalConfig {
    /// Whether the debug journal is written.
    pub enabled: bool,
    /// Fraction of the requests it records, from 0 to 1.
    pub sample_rate: f64,
    /// Life of a recorded entry, in days.
    pub ttl_days: usize,
}

/// `[encoder]` section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderConfig {
    /// Precision of the query encoding (ADR-0052).
    pub query_precision: Precision,
}

/// `[limits]` section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitsConfig {
    /// Depth of the write queue of an index.
    pub writer_queue: usize,
    /// Longest life of a request, in milliseconds.
    ///
    /// ADR-0099 bounds the shutdown drainage with it plus 1 000 ms; no key is
    /// added for the bound.
    pub default_timeout_ms: u64,
    /// Longest wait of `?wait=true`, in milliseconds (ADR-0067).
    pub wait_timeout_ms: u64,
    /// Entries of the query cache, per instance, never persisted (ADR-0051).
    pub query_cache_entries: usize,
}

/// Configuration of one instance, as `docs/07` describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// `[server]` section.
    pub server: ServerConfig,
    /// `[paths]` section.
    pub paths: PathsConfig,
    /// `[auth]` section.
    pub auth: AuthConfig,
    /// `[analytics]` section.
    pub analytics: AnalyticsConfig,
    /// `[debug_journal]` section.
    pub debug_journal: DebugJournalConfig,
    /// `[encoder]` section.
    pub encoder: EncoderConfig,
    /// `[limits]` section.
    pub limits: LimitsConfig,
}

impl Config {
    /// Reads the configuration file and applies the `CRIBLE_*` overrides of the
    /// process environment.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidRequest`](crible_core::error::ErrorCode::InvalidRequest)
    /// when the file is absent, unreadable or
    /// malformed, when a key or a section is missing or unknown, and in the
    /// three cases of ADR-0097.
    pub fn load(path: &Path) -> Result<Self, CribleError> {
        Self::load_with_env(path, environment()?)
    }

    /// Same as [`Config::load`], with the environment passed instead of read.
    ///
    /// The parameter exists so a test never mutates the environment of the
    /// process, which no test can restore safely.
    ///
    /// # Errors
    ///
    /// The errors of [`Config::load`].
    pub fn load_with_env<I>(path: &Path, env: I) -> Result<Self, CribleError>
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let raw = fs::read_to_string(path).map_err(|error| {
            invalid(format!(
                "cannot read the instance configuration {}: {error}",
                path.display()
            ))
        })?;
        let mut document: Table = toml::from_str(&raw).map_err(|error| {
            invalid(format!(
                "invalid instance configuration {}: {error}",
                path.display()
            ))
        })?;
        let overrides = apply_overrides(&mut document, env)?;
        Self::from_table(&document, &overrides)
    }

    /// Reads the configuration from a decoded TOML document.
    fn from_table(document: &Table, overrides: &Overrides) -> Result<Self, CribleError> {
        only_keys(document, "", &SECTIONS)?;

        let server = section(document, "server")?;
        only_keys(
            server,
            "server",
            &["bind", "search_threads", "max_concurrent_searches"],
        )?;
        let bind = text(server, "server", "bind", overrides)?;
        if bind.trim().is_empty() {
            return Err(invalid(format!(
                "setting 'bind' must not be empty at {}",
                origin(overrides, "server.bind")
            )));
        }
        let server = ServerConfig {
            bind,
            search_threads: count(server, "server", "search_threads", overrides)?,
            max_concurrent_searches: count(server, "server", "max_concurrent_searches", overrides)?,
        };

        let paths = section(document, "paths")?;
        only_keys(paths, "paths", &["data", "models", "exchange"])?;
        let paths = PathsConfig {
            data: path_setting(paths, "paths", "data", overrides)?,
            models: path_setting(paths, "paths", "models", overrides)?,
            exchange: path_setting(paths, "paths", "exchange", overrides)?,
        };

        let auth = section(document, "auth")?;
        only_keys(auth, "auth", &["keys"])?;
        let auth = AuthConfig {
            keys: keys(required(auth, "auth", "keys")?, overrides)?,
        };

        let analytics = section(document, "analytics")?;
        only_keys(analytics, "analytics", &["retention_days"])?;
        let analytics = AnalyticsConfig {
            retention_days: count(analytics, "analytics", "retention_days", overrides)?,
        };

        let journal = section(document, "debug_journal")?;
        only_keys(
            journal,
            "debug_journal",
            &["enabled", "sample_rate", "ttl_days"],
        )?;
        let sample_rate = float(journal, "debug_journal", "sample_rate", overrides)?;
        if !(0.0..=1.0).contains(&sample_rate) {
            return Err(invalid(format!(
                "setting 'sample_rate' must be in [0, 1] at {}, got {sample_rate}",
                origin(overrides, "debug_journal.sample_rate")
            )));
        }
        let debug_journal = DebugJournalConfig {
            enabled: boolean(journal, "debug_journal", "enabled", overrides)?,
            sample_rate,
            ttl_days: count(journal, "debug_journal", "ttl_days", overrides)?,
        };

        let encoder = section(document, "encoder")?;
        only_keys(encoder, "encoder", &["query_precision"])?;
        let encoder = EncoderConfig {
            query_precision: precision(
                &text(encoder, "encoder", "query_precision", overrides)?,
                overrides,
            )?,
        };

        let limits = section(document, "limits")?;
        only_keys(
            limits,
            "limits",
            &[
                "writer_queue",
                "default_timeout_ms",
                "wait_timeout_ms",
                "query_cache_entries",
            ],
        )?;
        let limits = LimitsConfig {
            writer_queue: count(limits, "limits", "writer_queue", overrides)?,
            default_timeout_ms: milliseconds(limits, "limits", "default_timeout_ms", overrides)?,
            wait_timeout_ms: milliseconds(limits, "limits", "wait_timeout_ms", overrides)?,
            query_cache_entries: count(limits, "limits", "query_cache_entries", overrides)?,
        };

        Ok(Self {
            server,
            paths,
            auth,
            analytics,
            debug_journal,
            encoder,
            limits,
        })
    }
}

/// Reads the process environment, refusing a `CRIBLE_*` variable that is not
/// UTF-8.
///
/// A variable of another prefix is dropped when its name or value is not UTF-8:
/// no key of `crible.toml` depends on it, and its bytes must not stop the
/// server. A `CRIBLE_*` one is a fault (complement of 2026-10-29 to ADR-0097):
/// it may name an override, and ignoring it would let a typo pass unnoticed.
fn environment() -> Result<Vec<(String, String)>, CribleError> {
    from_os_environment(std::env::vars_os())
}

/// Same as [`environment`], on an environment the caller provides.
///
/// The parameter exists so a test can hand over bytes that are not UTF-8
/// without mutating the environment of the process.
fn from_os_environment<I>(vars: I) -> Result<Vec<(String, String)>, CribleError>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    let mut environment = Vec::new();
    for (name, value) in vars {
        let name = match name.into_string() {
            Ok(name) => name,
            Err(raw) => {
                let lossy = raw.to_string_lossy();
                if lossy.starts_with(VARIABLE_PREFIX) {
                    return Err(invalid(format!(
                        "the variable '{lossy}' is not valid UTF-8; a CRIBLE_* variable must \
                         be (ADR-0097)"
                    )));
                }
                continue;
            }
        };
        let value = match value.into_string() {
            Ok(value) => value,
            Err(_) => {
                if name.starts_with(VARIABLE_PREFIX) {
                    return Err(invalid(format!(
                        "the value of '{name}' is not valid UTF-8; a CRIBLE_* variable must \
                         be (ADR-0097)"
                    )));
                }
                continue;
            }
        };
        environment.push((name, value));
    }
    Ok(environment)
}

/// Applies every `CRIBLE_*` variable of `env` to `document`.
///
/// The returned map remembers the variable that set each key, so a later
/// validation of that key can name it.
fn apply_overrides<I>(document: &mut Table, env: I) -> Result<Overrides, CribleError>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut overrides = Overrides::new();
    for (name, value) in env {
        let Some(path) = name.strip_prefix(VARIABLE_PREFIX) else {
            continue;
        };
        if EXCLUDED_VARIABLES.contains(&name.as_str()) {
            continue;
        }
        let key = apply_override(document, &name, path, &value)?;
        overrides.insert(key, name);
    }
    Ok(overrides)
}

/// Applies one override, whose variable is `name` and whose path is `path`.
///
/// The value is parsed against the type of the key it targets: the existing
/// value is the authority on that type, so a mistyped variable cannot silently
/// change a type. The function returns the key path it set, for the caller to
/// remember the variable.
fn apply_override(
    document: &mut Table,
    name: &str,
    path: &str,
    value: &str,
) -> Result<String, CribleError> {
    let mut keys = Vec::new();
    for part in path.split(SECTION_SEPARATOR) {
        if part.is_empty() {
            return Err(invalid(format!(
                "{name} does not name a key of crible.toml"
            )));
        }
        keys.push(part.to_ascii_lowercase());
    }
    let (last, sections) = keys
        .split_last()
        .ok_or_else(|| invalid(format!("{name} does not name a key of crible.toml")))?;

    let mut table = document;
    let mut traversed = String::new();
    for key in sections {
        traversed = if traversed.is_empty() {
            key.clone()
        } else {
            format!("{traversed}.{key}")
        };
        table = match table.get_mut(key) {
            Some(Value::Table(inner)) => inner,
            Some(Value::Array(_)) => {
                return Err(invalid(format!(
                    "{name} targets '{traversed}', a table of tables; those are not \
                     overridable by the environment in v1 (ADR-0097)"
                )));
            }
            Some(_) => {
                return Err(invalid(format!(
                    "{name} names '{traversed}', which is not a section of crible.toml"
                )));
            }
            None => {
                return Err(invalid(format!(
                    "{name} names no key of crible.toml: '{traversed}' is unknown"
                )));
            }
        };
    }
    let full = if traversed.is_empty() {
        last.clone()
    } else {
        format!("{traversed}.{last}")
    };

    match table.get_mut(last) {
        Some(Value::Array(_)) => Err(invalid(format!(
            "{name} targets '{full}', a table of tables; those are not overridable by the \
             environment in v1 (ADR-0097)"
        ))),
        Some(Value::Table(_)) => Err(invalid(format!(
            "{name} targets the section '{full}', not a key"
        ))),
        Some(Value::String(target)) => {
            *target = value.to_owned();
            Ok(full)
        }
        Some(Value::Integer(target)) => {
            *target = value
                .parse::<i64>()
                .map_err(|_| invalid(format!("{name} does not hold an integer at '{full}'")))?;
            Ok(full)
        }
        Some(Value::Float(target)) => {
            let parsed = value
                .parse::<f64>()
                .map_err(|_| invalid(format!("{name} does not hold a number at '{full}'")))?;
            if !parsed.is_finite() {
                return Err(invalid(format!(
                    "{name} does not hold a finite number at '{full}'"
                )));
            }
            *target = parsed;
            Ok(full)
        }
        Some(Value::Boolean(target)) => {
            *target = value
                .parse::<bool>()
                .map_err(|_| invalid(format!("{name} does not hold a boolean at '{full}'")))?;
            Ok(full)
        }
        Some(Value::Datetime(_)) => Err(invalid(format!(
            "{name} targets '{full}', a date and time no key of crible.toml holds"
        ))),
        None => Err(invalid(format!(
            "{name} names no key of crible.toml: '{full}' is unknown"
        ))),
    }
}

/// Reads the `[[auth.keys]]` array of tables.
///
/// The format and the uniqueness of ADR-0100 are checked here, at startup: a
/// digest of 64 lowercase hexadecimal characters, a unique name and a unique
/// digest, and the role parsed by [`Role::parse`]. An empty list is accepted:
/// the instance starts and every authenticated route answers `401`.
fn keys(value: &Value, overrides: &Overrides) -> Result<Vec<ApiKey>, CribleError> {
    let Value::Array(entries) = value else {
        return Err(invalid(
            "section 'auth.keys' must be an array of tables ([[auth.keys]])",
        ));
    };
    let mut keys = Vec::with_capacity(entries.len());
    for (position, entry) in entries.iter().enumerate() {
        let path = format!("auth.keys[{position}]");
        let Value::Table(entry) = entry else {
            return Err(invalid(format!("{path} must be a table")));
        };
        only_keys(entry, &path, &["name", "role", "sha256"])?;
        let name = text(entry, &path, "name", overrides)?;
        if name.trim().is_empty() {
            return Err(invalid(format!(
                "setting 'name' must not be empty at {}",
                origin(overrides, &format!("{path}.name"))
            )));
        }
        let sha256 = text(entry, &path, "sha256", overrides)?;
        if crate::auth::parse_digest(&sha256).is_none() {
            return Err(invalid(format!(
                "key '{name}': 'sha256' must be 64 lowercase hexadecimal characters at {} \
                 (ADR-0100)",
                origin(overrides, &format!("{path}.sha256"))
            )));
        }
        let role = Role::parse(
            &text(entry, &path, "role", overrides)?,
            &format!("{path}.role"),
        )
        // ADR-0100, decision 4: the message names the faulty key by its `name`,
        // as the three other cases of the startup validation do.
        .map_err(|error| invalid(format!("key '{name}': {}", error.message())))?;
        keys.push(ApiKey { name, role, sha256 });
    }
    refuse_duplicates(&keys)?;
    Ok(keys)
}

/// Refuses two keys with the same `name` or the same `sha256` (ADR-0100).
///
/// The message names the key, as ADR-0097 does for every configuration fault.
fn refuse_duplicates(keys: &[ApiKey]) -> Result<(), CribleError> {
    let mut names = BTreeSet::new();
    let mut digests = BTreeSet::new();
    for key in keys {
        if !names.insert(key.name.as_str()) {
            return Err(invalid(format!(
                "key '{}': 'name' is declared twice in [[auth.keys]] (ADR-0100)",
                key.name
            )));
        }
        if !digests.insert(key.sha256.as_str()) {
            return Err(invalid(format!(
                "key '{}': its 'sha256' is declared twice in [[auth.keys]] (ADR-0100)",
                key.name
            )));
        }
    }
    Ok(())
}

/// Parses `[encoder].query_precision`.
fn precision(text: &str, overrides: &Overrides) -> Result<Precision, CribleError> {
    match text {
        "fp32" => Ok(Precision::Fp32),
        "int8" => Ok(Precision::Int8),
        other => Err(invalid(format!(
            "unknown precision '{other}' at {}; expected fp32 or int8",
            origin(overrides, "encoder.query_precision")
        ))),
    }
}

/// Names a key and, when an environment variable set it, that variable too.
fn origin(overrides: &Overrides, path: &str) -> String {
    match overrides.get(path) {
        Some(name) => format!("{path} (set by {name})"),
        None => path.to_owned(),
    }
}

/// Joins a section path and a key for an error message.
fn dotted(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

/// Refuses a key outside the closed list of the section at `path`.
fn only_keys(table: &Table, path: &str, allowed: &[&str]) -> Result<(), CribleError> {
    for key in table.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(invalid(format!(
                "unknown key '{key}' at {}",
                dotted(path, key)
            )));
        }
    }
    Ok(())
}

fn section<'a>(document: &'a Table, name: &str) -> Result<&'a Table, CribleError> {
    match document.get(name) {
        Some(Value::Table(table)) => Ok(table),
        Some(_) => Err(invalid(format!("'{name}' must be a table at {name}"))),
        None => Err(invalid(format!("missing section '{name}' of crible.toml"))),
    }
}

fn required<'a>(table: &'a Table, path: &str, key: &str) -> Result<&'a Value, CribleError> {
    table
        .get(key)
        .ok_or_else(|| invalid(format!("missing key '{key}' at {}", dotted(path, key))))
}

/// Renders "at <key>" and, when an environment variable set the key, its name.
fn where_at(overrides: &Overrides, path: &str, key: &str) -> String {
    format!("at {}", origin(overrides, &dotted(path, key)))
}

fn text(
    table: &Table,
    path: &str,
    key: &str,
    overrides: &Overrides,
) -> Result<String, CribleError> {
    match required(table, path, key)? {
        Value::String(value) => Ok(value.clone()),
        _ => Err(invalid(format!(
            "'{key}' must be a string {}",
            where_at(overrides, path, key)
        ))),
    }
}

fn path_setting(
    table: &Table,
    path: &str,
    key: &str,
    overrides: &Overrides,
) -> Result<PathBuf, CribleError> {
    let value = text(table, path, key, overrides)?;
    if value.trim().is_empty() {
        return Err(invalid(format!(
            "'{key}' must not be empty {}",
            where_at(overrides, path, key)
        )));
    }
    Ok(PathBuf::from(value))
}

fn boolean(
    table: &Table,
    path: &str,
    key: &str,
    overrides: &Overrides,
) -> Result<bool, CribleError> {
    match required(table, path, key)? {
        Value::Boolean(value) => Ok(*value),
        _ => Err(invalid(format!(
            "'{key}' must be a boolean {}",
            where_at(overrides, path, key)
        ))),
    }
}

fn float(table: &Table, path: &str, key: &str, overrides: &Overrides) -> Result<f64, CribleError> {
    match required(table, path, key)? {
        Value::Float(value) => Ok(*value),
        Value::Integer(value) => Ok(*value as f64),
        _ => Err(invalid(format!(
            "'{key}' must be a number {}",
            where_at(overrides, path, key)
        ))),
    }
}

fn milliseconds(
    table: &Table,
    path: &str,
    key: &str,
    overrides: &Overrides,
) -> Result<u64, CribleError> {
    let value = integer(table, path, key, overrides)?;
    u64::try_from(value).map_err(|_| {
        invalid(format!(
            "'{key}' must not be negative {}",
            where_at(overrides, path, key)
        ))
    })
}

/// Reads a non-negative integer that counts something.
fn count(
    table: &Table,
    path: &str,
    key: &str,
    overrides: &Overrides,
) -> Result<usize, CribleError> {
    let value = integer(table, path, key, overrides)?;
    usize::try_from(value).map_err(|_| {
        invalid(format!(
            "'{key}' must be a non-negative integer {}",
            where_at(overrides, path, key)
        ))
    })
}

fn integer(
    table: &Table,
    path: &str,
    key: &str,
    overrides: &Overrides,
) -> Result<i64, CribleError> {
    match required(table, path, key)? {
        Value::Integer(value) => Ok(*value),
        _ => Err(invalid(format!(
            "'{key}' must be an integer {}",
            where_at(overrides, path, key)
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, Role};
    use crible_encode::Precision;
    use std::fs;
    use std::path::Path;

    /// The configuration of `docs/07`, section "Configuration de l'instance".
    const EXAMPLE: &str = r#"
[server]
bind = "0.0.0.0:8080"
search_threads = 0
max_concurrent_searches = 0

[paths]
data = "/data"
models = "/models"
exchange = "/exchange"

[[auth.keys]]
name = "legifacile-mcp"
role = "read"
sha256 = "1111111111111111111111111111111111111111111111111111111111111111"

[[auth.keys]]
name = "legifacile-sync"
role = "write"
sha256 = "2222222222222222222222222222222222222222222222222222222222222222"

[analytics]
retention_days = 90

[debug_journal]
enabled = false
sample_rate = 0.0
ttl_days = 7

[encoder]
query_precision = "fp32"

[limits]
writer_queue = 16
default_timeout_ms = 2000
wait_timeout_ms = 60000
query_cache_entries = 1024
"#;

    /// Writes `raw` in a temporary file and loads it with `env`.
    fn load(raw: &str, env: &[(&str, &str)]) -> Result<Config, crible_core::error::CribleError> {
        let directory = tempfile::tempdir().unwrap_or_else(|error| {
            panic!("a temporary directory for crible.toml must be creatable: {error}")
        });
        let path = directory.path().join("crible.toml");
        fs::write(&path, raw)
            .unwrap_or_else(|error| panic!("crible.toml must be writable: {error}"));
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        Config::load_with_env(&path, env)
    }

    fn message(error: crible_core::error::CribleError) -> String {
        error.message().to_owned()
    }

    #[test]
    fn the_configuration_of_docs_07_parses() {
        let config = load(EXAMPLE, &[]).unwrap_or_else(|error| {
            panic!(
                "the configuration of docs/07 must parse, got {}: {}",
                error.code().as_str(),
                error.message()
            )
        });
        assert_eq!(config.server.bind, "0.0.0.0:8080");
        assert_eq!(config.server.search_threads, 0);
        assert_eq!(config.server.max_concurrent_searches, 0);
        assert_eq!(config.paths.data, Path::new("/data"));
        assert_eq!(config.paths.models, Path::new("/models"));
        assert_eq!(config.paths.exchange, Path::new("/exchange"));
        assert_eq!(config.auth.keys.len(), 2);
        assert_eq!(config.auth.keys[0].name, "legifacile-mcp");
        assert_eq!(config.auth.keys[0].role, Role::Read);
        assert_eq!(config.auth.keys[1].role, Role::Write);
        assert_eq!(config.analytics.retention_days, 90);
        assert!(!config.debug_journal.enabled);
        assert_eq!(config.debug_journal.sample_rate, 0.0);
        assert_eq!(config.debug_journal.ttl_days, 7);
        // ADR-0052: `[encoder].query_precision` decides the precision of the
        // queries, and fp32 is the documented default.
        assert_eq!(config.encoder.query_precision, Precision::Fp32);
        // ADR-0051: `[limits].query_cache_entries` is the size of the cache.
        assert_eq!(config.limits.query_cache_entries, 1024);
        assert_eq!(config.limits.writer_queue, 16);
        assert_eq!(config.limits.default_timeout_ms, 2000);
        assert_eq!(config.limits.wait_timeout_ms, 60000);
    }

    #[test]
    fn an_absent_file_is_refused() {
        let directory = tempfile::tempdir().expect("a temporary directory is creatable");
        let path = directory.path().join("absent.toml");
        let error = Config::load_with_env(&path, Vec::new())
            .expect_err("an absent configuration file must be refused");
        assert!(message(error).contains("absent.toml"));
    }

    #[test]
    fn a_missing_section_is_refused() {
        let raw = EXAMPLE.replace("[analytics]\nretention_days = 90\n", "");
        let error = load(&raw, &[]).expect_err("a missing section is refused");
        assert_eq!(error.code(), crible_core::error::ErrorCode::InvalidRequest);
        assert!(message(error).contains("analytics"));
    }

    #[test]
    fn a_missing_key_is_refused() {
        let raw = EXAMPLE.replace("query_cache_entries = 1024\n", "");
        let error = load(&raw, &[]).expect_err("a missing key is refused");
        assert!(message(error).contains("query_cache_entries"));
    }

    #[test]
    fn an_unknown_key_is_refused() {
        let raw = EXAMPLE.replace("writer_queue = 16", "writer_queue = 16\nwriter_queu = 4");
        let error = load(&raw, &[]).expect_err("an unknown key is refused");
        assert!(message(error).contains("writer_queu"));
    }

    #[test]
    fn a_crible_variable_overrides_a_key() {
        // ADR-0097: `CRIBLE_SERVER__BIND` names `server.bind`.
        let config = load(EXAMPLE, &[("CRIBLE_SERVER__BIND", "127.0.0.1:9999")])
            .expect("a documented override is accepted");
        assert_eq!(config.server.bind, "127.0.0.1:9999");
    }

    #[test]
    fn a_crible_variable_reaches_an_integer_key() {
        // The two keys the suivi note names: `[limits].query_cache_entries`
        // (ADR-0051) and `[encoder].query_precision` (ADR-0052).
        let config = load(
            EXAMPLE,
            &[
                ("CRIBLE_LIMITS__QUERY_CACHE_ENTRIES", "64"),
                ("CRIBLE_ENCODER__QUERY_PRECISION", "int8"),
                ("CRIBLE_DEBUG_JOURNAL__SAMPLE_RATE", "0.25"),
                ("CRIBLE_DEBUG_JOURNAL__ENABLED", "true"),
            ],
        )
        .expect("the overrides are accepted");
        assert_eq!(config.limits.query_cache_entries, 64);
        assert_eq!(config.encoder.query_precision, Precision::Int8);
        assert_eq!(config.debug_journal.sample_rate, 0.25);
        assert!(config.debug_journal.enabled);
    }

    #[test]
    fn an_unknown_crible_variable_is_refused_naming_it() {
        // ADR-0097: a typo must not pass without effect.
        let error = load(EXAMPLE, &[("CRIBLE_SERVER__BND", "127.0.0.1:9999")])
            .expect_err("an unknown variable is refused");
        let message = message(error);
        assert!(message.contains("CRIBLE_SERVER__BND"), "got {message}");
        assert!(message.contains("server.bnd"), "got {message}");
    }

    #[test]
    fn a_value_of_the_wrong_type_is_refused_naming_the_variable() {
        let error = load(EXAMPLE, &[("CRIBLE_LIMITS__WRITER_QUEUE", "sixteen")])
            .expect_err("a value of the wrong type is refused");
        let message = message(error);
        assert!(
            message.contains("CRIBLE_LIMITS__WRITER_QUEUE"),
            "got {message}"
        );
    }

    #[test]
    fn a_negative_count_is_refused_naming_the_variable() {
        let error = load(EXAMPLE, &[("CRIBLE_LIMITS__QUERY_CACHE_ENTRIES", "-1")])
            .expect_err("a negative count is refused");
        let message = message(error);
        assert!(message.contains("query_cache_entries"), "got {message}");
        // ADR-0097: the refusal names the variable, not only the key.
        assert!(
            message.contains("CRIBLE_LIMITS__QUERY_CACHE_ENTRIES"),
            "got {message}"
        );
    }

    #[test]
    fn a_table_of_tables_cannot_be_overridden() {
        // ADR-0097 refuses an override of `[[auth.keys]]`.
        for name in ["CRIBLE_AUTH__KEYS__NAME", "CRIBLE_AUTH__KEYS"] {
            let error = load(EXAMPLE, &[(name, "x")])
                .err()
                .unwrap_or_else(|| panic!("{name} must be refused"));
            let message = message(error);
            assert!(message.contains(name), "got {message}");
            assert!(message.contains("table of tables"), "got {message}");
        }
    }

    #[test]
    fn a_variable_documented_outside_the_configuration_is_ignored() {
        // ADR-0097: the list is closed, and these seven variables name no key.
        let env: Vec<(&str, &str)> = super::EXCLUDED_VARIABLES
            .iter()
            .map(|name| (*name, "ignored"))
            .collect();
        let config = load(EXAMPLE, &env).expect("the excluded variables are ignored without error");
        assert_eq!(config.server.bind, "0.0.0.0:8080");
    }

    #[test]
    fn a_second_unknown_variable_is_still_refused() {
        // The exclusion is a list, not a prefix rule.
        let error = load(EXAMPLE, &[("CRIBLE_DEPOT", "/tmp")])
            .expect_err("CRIBLE_DEPOT is not in the closed list and names no key");
        assert!(message(error).contains("CRIBLE_DEPOT"));
    }

    #[test]
    fn a_value_out_of_range_is_refused_naming_the_variable() {
        let error = load(EXAMPLE, &[("CRIBLE_DEBUG_JOURNAL__SAMPLE_RATE", "2.0")])
            .expect_err("a sample rate above one is refused");
        let message = message(error);
        assert!(message.contains("sample_rate"), "got {message}");
        assert!(
            message.contains("CRIBLE_DEBUG_JOURNAL__SAMPLE_RATE"),
            "got {message}"
        );
    }

    #[test]
    fn an_unknown_enumeration_is_refused_naming_the_variable() {
        // The value parses as a string; the refusal comes from the key, and it
        // still has to name the variable (ADR-0097).
        let error = load(EXAMPLE, &[("CRIBLE_ENCODER__QUERY_PRECISION", "bogus")])
            .expect_err("an unknown precision is refused");
        let message = message(error);
        assert!(message.contains("bogus"), "got {message}");
        assert!(
            message.contains("CRIBLE_ENCODER__QUERY_PRECISION"),
            "got {message}"
        );
    }

    #[test]
    fn an_empty_override_is_refused_naming_the_variable() {
        let error =
            load(EXAMPLE, &[("CRIBLE_SERVER__BIND", "")]).expect_err("an empty bind is refused");
        let message = message(error);
        assert!(message.contains("server.bind"), "got {message}");
        assert!(message.contains("CRIBLE_SERVER__BIND"), "got {message}");
    }

    #[cfg(unix)]
    #[test]
    fn a_crible_variable_that_is_not_utf8_is_refused() {
        // Complement of 2026-10-29 to ADR-0097: no `CRIBLE_*` variable is
        // ignored silently, and its name is cited in an approximate form.
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let name = OsString::from_vec(b"CRIBLE_SERVER__BIND\xff".to_vec());
        let error = super::from_os_environment(vec![(name, OsString::from("127.0.0.1:1"))])
            .expect_err("a CRIBLE_* name that is not UTF-8 is refused");
        let refused_name = message(error);
        assert!(
            refused_name.contains("CRIBLE_SERVER__BIND"),
            "got {refused_name}"
        );
        assert!(refused_name.contains("UTF-8"), "got {refused_name}");

        let value = OsString::from_vec(vec![0xff, 0xfe]);
        let error =
            super::from_os_environment(vec![(OsString::from("CRIBLE_SERVER__BIND"), value)])
                .expect_err("a CRIBLE_* value that is not UTF-8 is refused");
        let refused = message(error);
        assert!(refused.contains("CRIBLE_SERVER__BIND"), "got {refused}");
        assert!(refused.contains("UTF-8"), "got {refused}");

        // A variable of another prefix keeps the startup going: no key of
        // `crible.toml` depends on it.
        let foreign = OsString::from_vec(b"HOSTNAME\xff".to_vec());
        let kept = super::from_os_environment(vec![
            (foreign, OsString::from("ignored")),
            (OsString::from("HOME"), OsString::from("/home")),
        ])
        .expect("a non-UTF-8 variable outside CRIBLE_ is ignored");
        assert_eq!(kept, vec![("HOME".to_owned(), "/home".to_owned())]);
    }

    #[test]
    fn an_unknown_role_is_refused_naming_the_key() {
        // ADR-0100, decision 4: the refusal names the key by its `name`, as
        // `a_digest_outside_the_format_is_refused_naming_the_key` does.
        let raw = EXAMPLE.replace("role = \"read\"", "role = \"reader\"");
        let error = load(&raw, &[]).expect_err("an unknown role is refused");
        let message = message(error);
        assert!(message.contains("reader"), "got {message}");
        assert!(
            message.contains("legifacile-mcp"),
            "names the key: {message}"
        );
    }

    #[test]
    fn a_digest_outside_the_format_is_refused_naming_the_key() {
        // ADR-0100, decision 4: 64 characters of `[0-9a-f]`, and the message
        // names the faulty key.
        let bad = [
            "abc".to_owned(),
            "1".repeat(63),
            "1".repeat(65),
            "A".repeat(64),
            format!("{}g", "1".repeat(63)),
            format!("0x{}", "1".repeat(62)),
            String::new(),
        ];
        for digest in bad {
            let raw = EXAMPLE.replace(
                "sha256 = \"1111111111111111111111111111111111111111111111111111111111111111\"",
                &format!("sha256 = \"{digest}\""),
            );
            let error = load(&raw, &[]).expect_err("a digest outside the format is refused");
            assert_eq!(error.code(), crible_core::error::ErrorCode::InvalidRequest);
            let message = message(error);
            assert!(
                message.contains("legifacile-mcp"),
                "the message names the key: {message}"
            );
            assert!(message.contains("sha256"), "got {message}");
        }
    }

    #[test]
    fn a_digest_declared_twice_is_refused() {
        // ADR-0100, decision 4: two keys cannot share a digest.
        let raw = EXAMPLE.replace(
            "sha256 = \"2222222222222222222222222222222222222222222222222222222222222222\"",
            "sha256 = \"1111111111111111111111111111111111111111111111111111111111111111\"",
        );
        let error = load(&raw, &[]).expect_err("a digest declared twice is refused");
        let message = message(error);
        assert!(message.contains("legifacile-sync"), "got {message}");
        assert!(message.contains("twice"), "got {message}");
    }

    #[test]
    fn a_name_declared_twice_is_refused() {
        // ADR-0100, decision 4: the name identifies a key in the analytics, so
        // two keys cannot share it.
        let raw = EXAMPLE.replace("name = \"legifacile-sync\"", "name = \"legifacile-mcp\"");
        let error = load(&raw, &[]).expect_err("a name declared twice is refused");
        let message = message(error);
        assert!(message.contains("legifacile-mcp"), "got {message}");
        assert!(message.contains("name"), "got {message}");
    }

    #[test]
    fn an_empty_key_list_is_accepted() {
        // ADR-0100, decision 4: `[[auth.keys]]` empty (`keys = []`) is the
        // closed-by-default instance; the startup warns and every authenticated
        // route answers `401`.
        let raw = EXAMPLE.replace(
            "[[auth.keys]]\nname = \"legifacile-mcp\"\nrole = \"read\"\nsha256 = \
             \"1111111111111111111111111111111111111111111111111111111111111111\"\n",
            "",
        );
        let raw = raw.replace(
            "[[auth.keys]]\nname = \"legifacile-sync\"\nrole = \"write\"\nsha256 = \
             \"2222222222222222222222222222222222222222222222222222222222222222\"\n",
            "[auth]\nkeys = []\n",
        );
        let config = load(&raw, &[]).expect("an empty key list is accepted");
        assert!(config.auth.keys.is_empty());
    }
}
