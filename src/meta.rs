//! `meta.json`: the spec, base, target agent and review pane of one review.
//!
//! The only file that is rewritten, by `open`, the TUI and the `send` action. A save takes the
//! store lock, reads the current file, changes the caller's fields and renames a temp file over it.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::store::{PaneId, Spec, StoreError, TerminalId, Warning, ensure_dir, lock, state_dir};

const META_FILE: &str = "meta.json";

/// The agent that receives a send.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    #[serde(rename = "pane_id")]
    pub pane: PaneId,
    #[serde(rename = "terminal_id")]
    pub terminal: TerminalId,
    pub agent: String,
}

/// Every field is optional, so a missing or reset file is an empty `Meta`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<Spec>,
    /// The base ref last chosen, kept while the spec is the working tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
    #[serde(
        default,
        rename = "review_pane_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub review_pane: Option<PaneId>,
}

/// The directory of the review of `root` under `base`. When the hashed directory is missing, a
/// sibling whose `meta.json` names `root` wins, since `DefaultHasher` may change between Rust
/// releases.
pub fn locate(base: &Path, root: &Path) -> PathBuf {
    let hashed = state_dir(base, root);
    if hashed.exists() {
        return hashed;
    }
    let siblings = fs::read_dir(base).into_iter().flatten().flatten();
    siblings
        .map(|entry| entry.path())
        .find(|dir| load(dir).0.root.as_deref() == Some(root))
        .unwrap_or(hashed)
}

/// Read `meta.json`. A missing file is an empty `Meta`. A file that cannot be read or parsed is an
/// empty `Meta` with a warning, and the next save rewrites it.
pub fn load(dir: &Path) -> (Meta, Option<Warning>) {
    match fs::read(dir.join(META_FILE)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => (Meta::default(), None),
        Err(_) => (Meta::default(), Some(Warning::MetaUnreadable)),
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_or((Meta::default(), Some(Warning::MetaUnreadable)), |meta| {
                (meta, None)
            }),
    }
}

/// Apply `change` to the current `meta.json` under the lock and write the result. Fields the
/// closure does not touch keep the value another process saved. `root` is always recorded.
pub fn save(dir: &Path, root: &Path, change: impl FnOnce(&mut Meta)) -> Result<(), StoreError> {
    ensure_dir(dir)?;
    let _lock = lock(dir)?;
    let mut meta = load(dir).0;
    change(&mut meta);
    meta.root = Some(root.to_path_buf());
    let path = dir.join(META_FILE);
    let temp = dir.join(format!("{META_FILE}.{}.tmp", std::process::id()));
    let io_error = |path: &Path| {
        let path = path.to_path_buf();
        move |error: io::Error| StoreError::Io {
            path,
            kind: error.kind(),
        }
    };
    let bytes = serde_json::to_vec_pretty(&meta)
        .map_err(|error| io_error(&path)(io::Error::from(error)))?;
    let written = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp)
        .and_then(|mut file| file.write_all(&bytes))
        .and_then(|()| fs::rename(&temp, &path));
    written.map_err(|error| {
        let _ = fs::remove_file(&temp);
        io_error(&path)(error)
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("herdr-review-meta-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn pane(text: &str) -> PaneId {
        PaneId::parse(text).unwrap()
    }

    fn target() -> Target {
        Target {
            pane: pane("w1:p2"),
            terminal: TerminalId::parse("term_1").unwrap(),
            agent: "claude".into(),
        }
    }

    const ROOT: &str = "/repo";

    #[test]
    fn a_missing_file_loads_as_an_empty_meta_with_no_warning() {
        assert_eq!(load(&temp_dir("missing")), (Meta::default(), None));
    }

    #[test]
    fn a_save_changes_only_the_callers_fields_and_keeps_the_rest() {
        let dir = temp_dir("fields");
        save(&dir, Path::new(ROOT), |meta| meta.target = Some(target())).unwrap();
        save(&dir, Path::new(ROOT), |meta| {
            meta.review_pane = Some(pane("w1:p9"));
        })
        .unwrap();
        save(&dir, Path::new(ROOT), |meta| {
            meta.spec = Some(Spec::Branch {
                base: "main".into(),
            });
        })
        .unwrap();
        let meta = load(&dir).0;
        assert_eq!(meta.target, Some(target()));
        assert_eq!(meta.review_pane, Some(pane("w1:p9")));
        assert_eq!(
            meta.spec,
            Some(Spec::Branch {
                base: "main".into()
            })
        );
        assert_eq!(meta.root, Some(PathBuf::from(ROOT)));
        save(&dir, Path::new(ROOT), |meta| meta.target = None).unwrap();
        assert_eq!(load(&dir).0.review_pane, Some(pane("w1:p9")));
        assert_eq!(load(&dir).0.target, None);
    }

    #[test]
    fn the_file_uses_the_names_in_the_plan() {
        let dir = temp_dir("names");
        save(&dir, Path::new(ROOT), |meta| {
            meta.target = Some(target());
            meta.review_pane = Some(pane("w1:p9"));
            meta.base = Some("origin/main".into());
        })
        .unwrap();
        let json =
            serde_json::from_slice::<serde_json::Value>(&fs::read(dir.join(META_FILE)).unwrap())
                .unwrap();
        assert_eq!(json["root"], ROOT);
        assert_eq!(json["base"], "origin/main");
        assert_eq!(json["review_pane_id"], "w1:p9");
        assert_eq!(json["target"]["pane_id"], "w1:p2");
        assert_eq!(json["target"]["terminal_id"], "term_1");
    }

    #[test]
    fn an_unreadable_file_loads_empty_with_a_warning_and_the_next_save_rewrites_it() {
        let dir = temp_dir("unreadable");
        fs::create_dir_all(&dir).unwrap();
        for bad in ["{not json", r#"{"target":{"pane_id":"a b"}}"#, ""] {
            fs::write(dir.join(META_FILE), bad).unwrap();
            assert_eq!(
                load(&dir),
                (Meta::default(), Some(Warning::MetaUnreadable)),
                "{bad}"
            );
        }
        save(&dir, Path::new(ROOT), |meta| {
            meta.review_pane = Some(pane("w1:p1"));
        })
        .unwrap();
        let (meta, warning) = load(&dir);
        assert_eq!((meta.review_pane, warning), (Some(pane("w1:p1")), None));
    }

    #[test]
    fn a_directory_in_place_of_the_file_is_unreadable() {
        let dir = temp_dir("directory");
        fs::create_dir_all(dir.join(META_FILE)).unwrap();
        assert_eq!(load(&dir).1, Some(Warning::MetaUnreadable));
    }

    #[test]
    fn a_save_leaves_a_private_file_and_no_temp_file() {
        let dir = temp_dir("private");
        save(&dir, Path::new(ROOT), |meta| meta.target = Some(target())).unwrap();
        let mode = fs::metadata(dir.join(META_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let names = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect::<Vec<_>>();
        assert!(
            names
                .iter()
                .all(|name| !name.to_string_lossy().ends_with(".tmp")),
            "{names:?}"
        );
    }

    #[test]
    fn a_save_that_cannot_write_is_an_error_and_not_a_panic() {
        let dir = temp_dir("blocked");
        fs::create_dir_all(dir.join(META_FILE)).unwrap();
        let result = save(&dir, Path::new(ROOT), |meta| meta.target = Some(target()));
        assert!(matches!(result, Err(StoreError::Io { .. })));
        assert!(
            !fs::read_dir(&dir).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
        );
    }

    #[test]
    fn two_processes_saving_different_fields_at_once_keep_both() {
        let dir = temp_dir("race");
        let spawn = |round: u32, field: u32| {
            let dir = dir.clone();
            std::thread::spawn(move || {
                for n in 0..40 {
                    save(&dir, Path::new(ROOT), |meta| {
                        if field == 0 {
                            meta.review_pane = PaneId::parse(&format!("w{round}:p{n}"));
                        } else {
                            meta.base = Some(format!("base-{n}"));
                        }
                    })
                    .unwrap();
                }
            })
        };
        let threads = [spawn(1, 0), spawn(1, 1)];
        for thread in threads {
            thread.join().unwrap();
        }
        let meta = load(&dir).0;
        assert_eq!(meta.review_pane, Some(pane("w1:p39")));
        assert_eq!(meta.base.as_deref(), Some("base-39"));
    }

    #[test]
    fn locate_prefers_the_hashed_directory() {
        let base = temp_dir("locate-hashed");
        let hashed = state_dir(&base, Path::new(ROOT));
        fs::create_dir_all(&hashed).unwrap();
        save(&base.join("old"), Path::new(ROOT), |_| {}).unwrap();
        assert_eq!(locate(&base, Path::new(ROOT)), hashed);
    }

    #[test]
    fn locate_scans_siblings_by_root_when_the_hashed_directory_is_missing() {
        let base = temp_dir("locate-scan");
        save(&base.join("other-hash"), Path::new("/else"), |_| {}).unwrap();
        save(&base.join("old-hash"), Path::new(ROOT), |_| {}).unwrap();
        fs::create_dir_all(base.join("no-meta")).unwrap();
        assert_eq!(locate(&base, Path::new(ROOT)), base.join("old-hash"));
    }

    #[test]
    fn locate_falls_back_to_the_hashed_directory_for_a_new_review() {
        let base = temp_dir("locate-new");
        let expected = state_dir(&base, Path::new(ROOT));
        assert_eq!(locate(&base, Path::new(ROOT)), expected);
        fs::create_dir_all(&base).unwrap();
        assert_eq!(locate(&base, Path::new(ROOT)), expected);
    }
}
