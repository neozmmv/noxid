//! Logging to the console and, optionally, a file.
//!
//! Adapted from Quix's `log.rs`. The reasoning carries over: a Windows GUI app,
//! like a Windows service, has no console, so without a file there is nothing
//! to ask a user for when something goes wrong. Two changes: it is built on
//! `tracing`, because iroh and iroh-live already log through it and one
//! pipeline is easier to read than two; and the file is rotated, where Quix's
//! grew without bound.

use std::fs::OpenOptions;
use std::path::PathBuf;
use std::sync::Mutex;

use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

/// The file is set aside once it passes this size, keeping one previous
/// generation. Checked when logging starts, which is often enough for an app
/// that is restarted between calls.
const ROTATE_AT: u64 = 10 * 1024 * 1024;

/// The default filter when `RUST_LOG` is not set: our own progress, and only
/// warnings from the stack under us. mDNS is quieter still, because it warns
/// about every second node on one machine, which is how tests run.
const DEFAULT_FILTER: &str = "warn,iroh_mdns_peer_lookup=error,noxid_core=info,noxid_cli=info,noxid_app=info";

/// Where the log file goes: `NOXID_LOG_PATH`, else nowhere.
pub fn path() -> Option<PathBuf> {
    std::env::var("NOXID_LOG_PATH").ok().map(PathBuf::from)
}

/// Installs the global subscriber. Safe to call more than once; later calls do
/// nothing.
pub fn init(file: Option<PathBuf>) {
    let filter = || EnvFilter::try_from_default_env().unwrap_or_else(|_| DEFAULT_FILTER.into());
    let console = tracing_subscriber::fmt::layer().with_filter(filter());

    let file_layer = file.and_then(|path| match open(&path) {
        Ok(file) => Some(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(Mutex::new(file))
                .with_filter(filter()),
        ),
        Err(e) => {
            // The logger cannot report its own failure through itself.
            eprintln!("could not open log file {}: {e}", path.display());
            None
        }
    });

    let _ = tracing_subscriber::registry()
        .with(console)
        .with(file_layer)
        .try_init();
}

fn open(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > ROTATE_AT) {
        let _ = std::fs::rename(path, path.with_extension("old.log"));
    }
    OpenOptions::new().create(true).append(true).open(path)
}
