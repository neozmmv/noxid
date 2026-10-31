//! Where Noxid keeps its state, and the node's persistent identity.
//!
//! Adapted from Quix's `identity.rs`. One difference on purpose: every state
//! file derives from a single directory (`NOXID_HOME`), rather than one
//! environment variable per file. Quix's v0.1.0 shipped broken because a newer
//! file's variable was forgotten in one place; with one directory there is
//! nothing to forget. Several nodes on one machine (for testing) each get their
//! own `NOXID_HOME`.

use anyhow::{Context, Result};
use iroh::SecretKey;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The directory every state file lives in.
pub fn state_dir() -> Result<PathBuf> {
    if let Ok(custom) = std::env::var("NOXID_HOME") {
        return Ok(PathBuf::from(custom));
    }
    let config_dir = dirs::config_dir().context("resolve config dir")?;
    Ok(config_dir.join("noxid"))
}

pub fn key_path() -> Result<PathBuf> {
    Ok(state_dir()?.join("key"))
}

/// Loads the persisted secret key, generating and saving a new one on first run.
pub fn load_or_create() -> Result<SecretKey> {
    load_or_create_at(&key_path()?)
}

fn load_or_create_at(path: &Path) -> Result<SecretKey> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let array: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("key file {} has wrong length", path.display()))?;
            return Ok(SecretKey::from_bytes(&array));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        // Anything else (permissions, I/O) must not be mistaken for "no key":
        // minting a new one would silently give this machine a new identity.
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    }

    let key = SecretKey::generate();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_protected(path, &key.to_bytes())
        .with_context(|| format!("writing the identity key to {}", path.display()))?;
    Ok(key)
}

/// Writes the key so it is never on disk readable by anyone else.
///
/// On Unix the mode goes on at creation, so there is no window in which the
/// file is world readable. On Windows the file inherits the ACL of the user's
/// own profile directory, which already excludes other ordinary users; Quix
/// had to tighten its ACL because it ran as a service writing to a shared
/// location, which Noxid does not.
///
/// `create_new` rather than a plain write: an existing key is never clobbered.
fn write_protected(path: &Path, key: &[u8]) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(path)?;
    file.write_all(key)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("noxid-{name}-{}", std::process::id()))
    }

    #[test]
    fn a_key_survives_a_restart() {
        let path = temp("key-restart");
        let _ = std::fs::remove_file(&path);

        let first = load_or_create_at(&path).unwrap();
        let second = load_or_create_at(&path).unwrap();
        assert_eq!(first.public(), second.public());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_existing_key_is_never_overwritten() {
        let path = temp("key-keep");
        std::fs::write(&path, b"existing").unwrap();

        assert!(write_protected(&path, &[7u8; 32]).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"existing");
        assert!(load_or_create_at(&path).is_err(), "a corrupt key is an error, not a new identity");

        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn a_freshly_written_key_is_not_readable_by_others() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp("key-mode");
        let _ = std::fs::remove_file(&path);

        write_protected(&path, &[7u8; 32]).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "group or other can reach it: {mode:o}");

        let _ = std::fs::remove_file(&path);
    }
}
