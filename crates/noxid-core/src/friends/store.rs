//! The friends list on disk: `friends.json` in the state directory.
//!
//! Unlike `profile.json`, a file that cannot be read is never quietly
//! replaced with an empty list: that would drop every friendship without a
//! word. A corrupt file is set aside under a new name, a file from a newer
//! version or another identity is left alone, and in both cases the store
//! runs degraded: nothing is written, and peers are told to come back later
//! rather than that we do not know them.
//!
//! Writes go to a temporary file that is synced and renamed over the real
//! one, so a crash mid-write leaves the previous list in place.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::state::{Book, PeerRecord};

/// The newest file format this build reads and writes.
pub const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    /// The identity this list belongs to, so a state directory copied
    /// between identities is never mixed up.
    owner: String,
    #[serde(default)]
    peers: BTreeMap<String, PeerRecord>,
}

#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    writable: bool,
}

/// What opening the store found.
#[derive(Debug)]
pub struct Opened {
    pub store: Store,
    pub book: Book,
    /// Why the store is degraded, in words for the user.
    pub problem: Option<String>,
}

impl Store {
    /// Reads the list at `path` for the identity `own`.
    pub fn open(path: PathBuf, own: EndpointId) -> Opened {
        let degraded = |path: PathBuf, problem: String| {
            warn!("friends list unavailable: {problem}");
            Opened {
                store: Store { path, writable: false },
                book: Book::new(own),
                problem: Some(problem),
            }
        };

        let data = match std::fs::read(&path) {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Opened {
                    store: Store { path, writable: true },
                    book: Book::new(own),
                    problem: None,
                };
            }
            // Unreadable is not the same as absent: leave it for next time.
            Err(e) => return degraded(path, format!("your friends list could not be read ({e})")),
        };

        let file: File = match serde_json::from_slice(&data) {
            Ok(file) => file,
            Err(e) => {
                let aside = set_aside(&path);
                return degraded(
                    path,
                    format!(
                        "your friends list was damaged ({e}) and has been kept as {}; \
                         friends you had will show up again when they contact you",
                        aside.as_deref().unwrap_or("a backup")
                    ),
                );
            }
        };
        if file.version > VERSION {
            return degraded(
                path,
                "your friends list was saved by a newer version of Noxid; update to see it".into(),
            );
        }
        if file.owner != own.to_string() {
            return degraded(path, "your friends list belongs to a different Noxid identity".into());
        }

        let mut peers = BTreeMap::new();
        for (id, record) in file.peers {
            match id.parse::<EndpointId>() {
                Ok(peer) if peer != own => {
                    peers.insert(peer, record);
                }
                _ => warn!(%id, "skipping a friends list entry with a bad id"),
            }
        }
        Opened {
            store: Store { path, writable: true },
            book: Book::from_records(own, peers),
            problem: None,
        }
    }

    pub fn is_degraded(&self) -> bool {
        !self.writable
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes `book` in full. Refused while degraded.
    pub fn save(&self, book: &Book) -> Result<()> {
        anyhow::ensure!(self.writable, "the friends list is read-only until it can be loaded");
        let file = File {
            version: VERSION,
            owner: book.own_id().to_string(),
            peers: book
                .records()
                .iter()
                .map(|(id, record)| (id.to_string(), record.clone()))
                .collect(),
        };
        let data = serde_json::to_vec_pretty(&file)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let mut out = std::fs::File::create(&tmp).with_context(|| format!("writing {}", tmp.display()))?;
        out.write_all(&data)?;
        out.sync_all()?;
        drop(out);
        std::fs::rename(&tmp, &self.path).with_context(|| format!("replacing {}", self.path.display()))?;
        Ok(())
    }
}

/// Moves a damaged file out of the way, keeping every byte. Returns the new
/// file name.
fn set_aside(path: &Path) -> Option<String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = format!("{}.corrupt-{stamp}", path.file_name()?.to_string_lossy());
    let aside = path.with_file_name(&name);
    match std::fs::rename(path, &aside) {
        Ok(()) => Some(name),
        Err(e) => {
            warn!("could not set the damaged friends list aside: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::friends::state::Relation;
    use iroh::SecretKey;

    fn key(n: u8) -> EndpointId {
        SecretKey::from_bytes(&[n; 32]).public()
    }

    const T0: i64 = 1_800_000_000_000;

    /// A fresh directory per test, so tests never share files.
    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("noxid-friends-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn book_with_a_request(own: EndpointId) -> Book {
        let mut book = Book::new(own);
        book.request(key(2), T0).unwrap();
        book
    }

    #[test]
    fn no_file_is_an_empty_list() {
        let opened = Store::open(dir("none").join("friends.json"), key(1));
        assert!(opened.book.records().is_empty());
        assert!(opened.problem.is_none());
        assert!(!opened.store.is_degraded());
    }

    #[test]
    fn a_saved_list_survives_a_restart() {
        let path = dir("restart").join("friends.json");
        let opened = Store::open(path.clone(), key(1));
        let book = book_with_a_request(key(1));
        opened.store.save(&book).unwrap();

        let again = Store::open(path, key(1));
        assert!(again.problem.is_none());
        assert_eq!(again.book.records(), book.records());
        assert!(matches!(again.book.relation(&key(2)), Some(Relation::Outgoing { .. })));
    }

    #[test]
    fn saving_leaves_no_temporary_file_behind() {
        let dir = dir("tmp");
        let path = dir.join("friends.json");
        let opened = Store::open(path.clone(), key(1));
        opened.store.save(&book_with_a_request(key(1))).unwrap();
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["friends.json".to_string()]);
    }

    #[test]
    fn a_leftover_temporary_file_from_a_crash_is_ignored() {
        let path = dir("crash").join("friends.json");
        Store::open(path.clone(), key(1)).store.save(&book_with_a_request(key(1))).unwrap();
        std::fs::write(path.with_extension("json.tmp"), b"{half a fi").unwrap();
        let again = Store::open(path, key(1));
        assert!(again.problem.is_none());
        assert_eq!(again.book.records().len(), 1);
    }

    #[test]
    fn a_corrupt_file_is_set_aside_not_replaced() {
        let dir = dir("corrupt");
        let path = dir.join("friends.json");
        std::fs::write(dir.join("key"), [7u8; 32]).unwrap();
        std::fs::write(&path, b"{not json").unwrap();

        let opened = Store::open(path.clone(), key(1));
        assert!(opened.problem.is_some());
        assert!(opened.store.is_degraded());
        assert!(opened.book.records().is_empty());
        assert!(!path.exists(), "the corrupt file was moved aside");
        let aside: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("friends.json.corrupt-"))
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(std::fs::read(&aside[0]).unwrap(), b"{not json");
        assert_eq!(std::fs::read(dir.join("key")).unwrap(), [7u8; 32], "the identity is untouched");
    }

    #[test]
    fn a_degraded_store_never_writes() {
        let path = dir("degraded").join("friends.json");
        std::fs::write(&path, b"garbage").unwrap();
        let opened = Store::open(path.clone(), key(1));
        assert!(opened.store.save(&book_with_a_request(key(1))).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn a_file_from_a_newer_version_is_left_alone() {
        let path = dir("newer").join("friends.json");
        let data = format!(r#"{{"version": 99, "owner": "{}", "peers": {{}}}}"#, key(1));
        std::fs::write(&path, &data).unwrap();
        let opened = Store::open(path.clone(), key(1));
        assert!(opened.store.is_degraded());
        assert!(opened.problem.is_some());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), data);
    }

    #[test]
    fn a_list_belonging_to_another_identity_is_left_alone() {
        let path = dir("owner").join("friends.json");
        Store::open(path.clone(), key(1)).store.save(&book_with_a_request(key(1))).unwrap();
        let before = std::fs::read(&path).unwrap();
        let opened = Store::open(path.clone(), key(9));
        assert!(opened.store.is_degraded());
        assert!(opened.book.records().is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn an_entry_with_a_bad_id_is_skipped_and_the_rest_kept() {
        let path = dir("badkey").join("friends.json");
        Store::open(path.clone(), key(1)).store.save(&book_with_a_request(key(1))).unwrap();
        let mut file: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let record = file["peers"][key(2).to_string()].clone();
        file["peers"]["not-an-id"] = record;
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();

        let opened = Store::open(path, key(1));
        assert!(opened.problem.is_none());
        assert_eq!(opened.book.records().len(), 1);
    }
}
