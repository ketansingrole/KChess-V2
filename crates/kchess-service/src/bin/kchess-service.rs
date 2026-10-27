//! `kchess-service serve --profile <dir> [--config <json>] [--idle-ms <ms>]`: the shared core
//! of one KChess profile. `kchess-service version` prints the release and protocol a client of
//! this build sends (`docs/shared-core.md`). Apps start it through
//! `kchess_service::connect_or_start`; it stops on its own once no app has used it for the idle
//! period.

use std::path::PathBuf;
use std::time::Duration;

use kchess_service::ServiceConfig;
use kchess_service::server::{ServeOptions, serve};

/// The default wait after the last client leaves (a window reloading, an app restarting).
const IDLE: Duration = Duration::from_secs(10);

fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("serve") => {}
        // What a client of this build sends in `hello` (the Electron app asks its bundled copy).
        Some("version") => {
            println!(
                "{}",
                serde_json::json!({
                    "release": kchess_service::protocol::release(),
                    "protocol": kchess_service::protocol::protocol(),
                })
            );
            return std::process::ExitCode::SUCCESS;
        }
        _ => {
            eprintln!(
                "usage: kchess-service serve --profile <dir> [--config <json>] [--idle-ms <ms>]\n       kchess-service version"
            );
            return std::process::ExitCode::from(2);
        }
    }
    let (mut profile, mut config, mut idle) = (None, ServiceConfig::default(), IDLE);
    while let Some(flag) = args.next() {
        let value = args.next();
        match (flag.as_str(), value) {
            ("--profile", Some(path)) => profile = Some(PathBuf::from(path)),
            ("--config", Some(json)) => match serde_json::from_str(&json) {
                Ok(parsed) => config = parsed,
                Err(error) => {
                    eprintln!("kchess-service: invalid --config: {error}");
                    return std::process::ExitCode::from(2);
                }
            },
            ("--idle-ms", Some(ms)) => match ms.parse::<u64>() {
                Ok(ms) => idle = Duration::from_millis(ms),
                Err(error) => {
                    eprintln!("kchess-service: invalid --idle-ms: {error}");
                    return std::process::ExitCode::from(2);
                }
            },
            (flag, _) => {
                eprintln!("kchess-service: unknown or incomplete option {flag}");
                return std::process::ExitCode::from(2);
            }
        }
    }
    let Some(profile) = profile else {
        eprintln!("kchess-service: --profile is required");
        return std::process::ExitCode::from(2);
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("kchess-core")
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("kchess-service: no runtime: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    kchess_service::runtime::remove_other_releases();
    let options = ServeOptions {
        config: config.into_config(profile),
        idle,
    };
    match runtime.block_on(serve(options)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("kchess-service: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}
