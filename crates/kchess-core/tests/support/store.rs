//! A temporary profile's `kchess.db` driven through the storage methods (`store.*`), as the
//! TypeScript test fixture (`tests/fixtures/nativeGames.ts`) drove them before these tests moved
//! to Rust. The fixture's own duties are here too: the one-time import of an earlier profile runs
//! before each call, its plaintext tokens are encrypted with the fake secret store, and login
//! and token calls check the OS encryption and the logout race. The fake store "encrypts" by
//! prefixing `enc:`, so a test can see the stored ciphertext without a keychain.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kchess_core::store::Database;
use kchess_core::{Config, CoreError};
use kchess_domain::misc::validate_arguments;
use rusqlite::Connection;
use rusqlite::types::ValueRef;
use serde_json::{Map, Value, json};
use tempfile::TempDir;

use super::Recording;

pub const ENCRYPTED_PREFIX: &str = "enc:";

/// What the fake secret store answers: whether encryption is available, and whether stored
/// ciphertext can be decrypted (a locked keychain cannot).
#[derive(Clone, Copy, Debug)]
pub struct Secrets {
    pub available: bool,
    pub decrypts: bool,
}

impl Secrets {
    pub const ON: Secrets = Secrets {
        available: true,
        decrypts: true,
    };
    pub const OFF: Secrets = Secrets {
        available: false,
        decrypts: false,
    };
}

/// One profile: its directory (kept until the value drops), the database, and the fake secrets.
pub struct Store {
    dir: TempDir,
    legacy: Option<PathBuf>,
    host: Arc<Recording>,
    pub db: Database,
    pub secrets: Secrets,
}

/// A fresh profile on a temporary directory, with OS encryption available.
pub fn profile() -> Store {
    Store::new(None, Secrets::ON)
}

impl Store {
    /// A profile over `legacy` (an earlier release's `kchess.db`, imported once when present).
    pub fn new(legacy: Option<PathBuf>, secrets: Secrets) -> Store {
        let dir = tempfile::tempdir().expect("temporary profile");
        Store::in_dir(dir, legacy, secrets)
    }

    /// A profile in an existing directory, so a test can reopen the same database.
    pub fn in_dir(dir: TempDir, legacy: Option<PathBuf>, secrets: Secrets) -> Store {
        let host = Arc::new(Recording::default());
        let db = Database::new(config(dir.path(), legacy.clone()), host.clone());
        Store {
            dir,
            legacy,
            host,
            db,
            secrets,
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// The host's log lines (for redaction and scope checks).
    pub fn host(&self) -> &Arc<Recording> {
        &self.host
    }

    /// Releases the database file; the next call opens it again.
    pub fn close(&self) {
        self.db.close();
    }

    /// Reopens the same profile with other secrets, as a relaunch with a different keychain.
    pub fn reopen_with(self, secrets: Secrets) -> Store {
        let Store { dir, legacy, .. } = self;
        Store::in_dir(dir, legacy, secrets)
    }

    /// The one-time import, then the storage method (the fixture's `storeCall`).
    pub fn call(&self, method: &str, args: Vec<Value>) -> Result<Value, CoreError> {
        self.migrate();
        self.db.call(method, &args)
    }

    /// `call`, which must succeed.
    pub fn ok(&self, method: &str, args: Vec<Value>) -> Value {
        self.call(method, args)
            .unwrap_or_else(|cause| panic!("{method} failed: {cause}"))
    }

    /// `call`, which must fail: its message.
    pub fn err(&self, method: &str, args: Vec<Value>) -> String {
        match self.call(method, args) {
            Ok(value) => panic!("{method} unexpectedly returned {value}"),
            Err(cause) => cause.message,
        }
    }

    /// The one-time import (`store.games.migrate`); its plaintext tokens are stored encrypted
    /// with the fake secret store, as the fixture did. Failed token imports are skipped.
    fn migrate(&self) {
        let available = self.secrets.available;
        let Ok(result) = self.db.call("store.games.migrate", &[json!(available)]) else {
            return;
        };
        if !available {
            return;
        }
        for entry in result["legacyTokens"]
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            let account = entry["account"].clone();
            let token = entry["token"].as_str().unwrap_or_default();
            let _ = self.db.call(
                "store.games.importTokenCiphertext",
                &[account, json!(encrypt(token))],
            );
        }
    }

    /// Stores a login with its token and connected account, unless a logout ran since
    /// (`saveLogin` in the fixture).
    pub fn save_login(
        &self,
        username: &str,
        token: &str,
        still_current: bool,
    ) -> Result<Value, String> {
        if !self.secrets.available {
            return Err("OS credential encryption is unavailable.".to_string());
        }
        let name = username.trim();
        check_username(name)?;
        self.migrate();
        if !still_current {
            return Err("Login was cancelled by logout.".to_string());
        }
        self.call(
            "store.games.saveTokenCiphertext",
            vec![json!(name), json!(encrypt(token))],
        )
        .map_err(|cause| cause.message)
    }

    /// The stored token of an account, decrypted; `None` when none is stored, encryption is off,
    /// or the ciphertext cannot be read (`getToken` in the fixture).
    pub fn token(&self, username: &str) -> Option<String> {
        let stored = self
            .call("store.games.tokenCiphertext", vec![json!(username)])
            .ok()?;
        let encrypted = stored.as_str()?;
        if !self.secrets.available || !self.secrets.decrypts {
            return None;
        }
        encrypted.strip_prefix(ENCRYPTED_PREFIX).map(str::to_string)
    }

    /// Runs a query on the profile's database file with no parameters: each row is an object
    /// keyed by column name, as the TypeScript tests read `db.prepare(sql).all()`.
    pub fn rows(&self, sql: &str) -> Vec<Value> {
        self.rows_with(sql, &[])
    }

    /// `rows` with text parameters.
    pub fn rows_with(&self, sql: &str, params: &[&str]) -> Vec<Value> {
        let connection = self.connection();
        let mut statement = connection.prepare(sql).expect("query compiles");
        let names: Vec<String> = statement
            .column_names()
            .iter()
            .map(|name| name.to_string())
            .collect();
        let mut rows = statement
            .query(rusqlite::params_from_iter(params.iter().copied()))
            .expect("query runs");
        let mut out = Vec::new();
        while let Some(row) = rows.next().expect("row") {
            let mut object = Map::new();
            for (index, name) in names.iter().enumerate() {
                let value = match row.get_ref(index).expect("column") {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(number) => json!(number),
                    ValueRef::Real(number) => json!(number),
                    ValueRef::Text(text) => json!(String::from_utf8_lossy(text)),
                    ValueRef::Blob(_) => Value::Null,
                };
                object.insert(name.clone(), value);
            }
            out.push(Value::Object(object));
        }
        out
    }

    /// Runs statements that seed rows (the earlier release's writes) on the profile's database.
    pub fn exec(&self, sql: &str) {
        self.connection()
            .execute_batch(sql)
            .unwrap_or_else(|cause| panic!("seeding failed: {cause}"));
    }

    /// Another connection to the profile's file; WAL lets it read and write beside the core's.
    fn connection(&self) -> Connection {
        // Opening (and migrating) the file through the core comes first, so the second connection
        // finds the schema in place.
        self.db.call("store.open", &[]).expect("the database opens");
        Connection::open(self.dir.path().join("kchess.db")).expect("the database file opens")
    }
}

fn config(dir: &Path, legacy: Option<PathBuf>) -> Config {
    let mut config = Config::new(dir.to_path_buf());
    config.legacy_database_path = legacy;
    config
}

/// The fake encryption of a token.
pub fn encrypt(plain: &str) -> String {
    format!("{ENCRYPTED_PREFIX}{plain}")
}

/// The facade's username check (`assertUsername`).
pub fn check_username(name: &str) -> Result<(), String> {
    match validate_arguments("addAccount", &[json!(name)]) {
        Some(Err(message)) => Err(message),
        _ => Ok(()),
    }
}

/// `assertGamePageQuery`: the facade's check of a page query.
pub fn check_page_query(query: &Value) -> Result<(), String> {
    match validate_arguments("gamePage", std::slice::from_ref(query)) {
        Some(Err(message)) => Err(message),
        _ => Ok(()),
    }
}

/// A played game as the TypeScript tests built it: an ordinary blitz win as White against Bob.
/// `overrides` replace fields; a `null` removes the field (JavaScript's `undefined`).
pub fn game(id: &str, overrides: &[(&str, Value)]) -> Value {
    let mut game = json!({
        "id": id,
        "account": "Alice",
        "createdAt": 1_000,
        "lastMoveAt": 1_000,
        "rated": true,
        "speed": "blitz",
        "perf": "blitz",
        "status": "mate",
        "winner": "white",
        "color": "white",
        "opponent": "Bob",
        "opponentRating": 1500,
        "playerRating": 1520,
        "ratingDiff": 12,
        "moves": "e2e4 e7e5",
    });
    for (key, value) in overrides {
        let object = game.as_object_mut().expect("a game is an object");
        if value.is_null() {
            object.remove(*key);
        } else {
            object.insert((*key).to_string(), value.clone());
        }
    }
    game
}

/// A game as the library tests built it: `Date.UTC(2026, 9, 1)` dates, a `pgn`, and no
/// opponent rating. `overrides` behave as in `game`.
pub fn library_game(id: &str, overrides: &[(&str, Value)]) -> Value {
    let mut game = game(id, &[]);
    let object = game.as_object_mut().expect("a game is an object");
    object.remove("opponentRating");
    let base = json!({
        "createdAt": 1_790_812_800_000_i64,
        "lastMoveAt": 1_790_812_800_000_i64,
        "playerRating": 1500,
        "ratingDiff": 8,
        "moves": "e4 e5",
        "pgn": "1. e4 e5 *",
    });
    for (key, value) in base.as_object().expect("base fields") {
        object.insert(key.clone(), value.clone());
    }
    for (key, value) in overrides {
        if value.is_null() {
            object.remove(*key);
        } else {
            object.insert((*key).to_string(), value.clone());
        }
    }
    game
}

/// `value` with whole-number floats written as integers, as JavaScript sees them: `1508.0` and
/// `1508` are the same number, which `serde_json::Value` equality would tell apart.
pub fn canonical(value: &Value) -> Value {
    match value {
        Value::Number(number) => match number.as_f64() {
            Some(float) if float.fract() == 0.0 && number.is_f64() => json!(float as i64),
            _ => value.clone(),
        },
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, inner)| (key.clone(), canonical(inner)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// Parses a JSON value the store returned, failing the test when it is not an object.
pub fn object(value: &Value) -> &Map<String, Value> {
    value.as_object().expect("an object")
}
