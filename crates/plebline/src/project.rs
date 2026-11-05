//! A project: a directory under `~/PlebLine` with fixed file names, so
//! nobody types paths.
//!
//! Nothing is deleted on its own: `batch.json` must outlive the last
//! broadcast, or the cold storage has to be opened again. Files go only
//! when the user asks, even once the chain confirms every payment: signed
//! payments go out whenever the user likes.

use std::path::{Path, PathBuf};

use plebline_core::lang::t;
use plebline_core::tf;
use plebline_core::{Error, Result};

pub const WALLET: &str = "wallet.json";
pub const UTXOS: &str = "utxos.json";
pub const PLAN: &str = "plan.json";
pub const BATCH: &str = "batch.json";
pub const SPLIT: &str = "split.json";
pub const CONFIG: &str = "batch-config.json";

/// Every file a project can contain, in the order they come into being.
pub const FILES: [&str; 6] = [WALLET, UTXOS, CONFIG, SPLIT, PLAN, BATCH];

/// Printable BIP39 wordlist written by the guide. Public, but written by the
/// program, so it is deleted with the rest.
pub const WORDLIST: &str = "parole-bip39.txt";

#[derive(Clone)]
pub struct Project {
    pub name: String,
    pub dir: PathBuf,
}

impl Project {
    pub fn path(&self, file: &str) -> PathBuf {
        self.dir.join(file)
    }

    pub fn has(&self, file: &str) -> bool {
        self.path(file).is_file()
    }

    /// Removes one of the ceremony's files if it is there; `true` if it was.
    pub fn remove_file(&self, file: &str) -> Result<bool> {
        remove_if_present(&self.path(file))
    }
}

fn remove_if_present(path: &Path) -> Result<bool> {
    if !path.is_file() {
        return Ok(false);
    }
    std::fs::remove_file(path).map_err(Error::io(path))?;
    Ok(true)
}

/// Under `$HOME`, so the ceremony is found the same way whatever the
/// launch directory.
pub fn root() -> Result<PathBuf> {
    Ok(home()?.join(FOLDER_NAME))
}

const FOLDER_NAME: &str = "PlebLine";

fn home() -> Result<PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(home) = crate::testing::home() {
        return Ok(home);
    }
    let home = std::env::var("HOME").map_err(|_| {
        Error::Refused(
            t(
                "non trovo la cartella home: la variabile HOME non è impostata",
                "cannot find the home directory: the HOME variable is not set",
            )
            .into(),
        )
    })?;
    Ok(PathBuf::from(home))
}

/// The existing projects, most recent first.
pub fn list() -> Vec<Project> {
    let Ok(root) = root() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };

    let mut projects: Vec<(std::time::SystemTime, Project)> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| {
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (
                modified,
                Project {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    dir: entry.path(),
                },
            )
        })
        .collect();

    projects.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    projects.into_iter().map(|(_, p)| p).collect()
}

pub fn open_or_create(name: &str) -> Result<Project> {
    let name = sanitize(name);
    if name.is_empty() {
        return Err(Error::Input(
            t(
                "il nome del progetto non può essere vuoto",
                "the project name cannot be empty",
            )
            .into(),
        ));
    }

    let dir = root()?.join(&name);
    std::fs::create_dir_all(&dir).map_err(Error::io(&dir))?;

    // Even the file names reveal ceremonies.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for target in [root()?, dir.clone()] {
            let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700));
        }
    }

    // macOS: keep Spotlight from indexing labels and JSON. Failure is ignored.
    #[cfg(target_os = "macos")]
    {
        let marker = root()?.join(".metadata_never_index");
        if !marker.exists() {
            let _ = std::fs::write(&marker, b"");
        }
    }

    Ok(Project { name, dir })
}

/// Deletes only known file names; the directory goes only if left empty.
/// Never recursive on a prompted name. Signed files set aside go too.
pub fn wipe(project: &Project) -> Result<usize> {
    let mut removed = 0;
    for dir in set_aside_dirs(project) {
        removed += remove(&earlier(project, dir.clone()), &[BATCH, SPLIT])?;
        let _ = std::fs::remove_dir(&dir);
    }
    let _ = std::fs::remove_dir(project.path(SET_ASIDE));

    let mut files: Vec<&str> = FILES.to_vec();
    files.push(WORDLIST);
    removed += remove(project, &files)?;
    let _ = std::fs::remove_dir(&project.dir);
    Ok(removed)
}

/// Where the guided path sets earlier sessions' signed files aside: inside
/// the project, so no listing takes it for a project and no commitment
/// reads it.
pub const SET_ASIDE: &str = "precedenti";

/// Moves the signed files and their per-payment hex into a new numbered
/// folder under [`SET_ASIDE`]: kept, no longer read. `None` with nothing
/// signed here.
pub fn set_aside(project: &Project) -> Result<Option<PathBuf>> {
    let mut moving = signed_files(project);
    if moving.is_empty() {
        return Ok(None);
    }
    if let Ok(entries) = std::fs::read_dir(&project.dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if crate::wizard::is_per_tx_name(&name) || crate::wizard::is_per_tx_archive(&name) {
                moving.push(entry.path());
            }
        }
    }

    let parent = project.path(SET_ASIDE);
    let mut number = 1u32;
    while parent.join(number.to_string()).exists() {
        number += 1;
    }
    let dir = parent.join(number.to_string());
    std::fs::create_dir_all(&dir).map_err(Error::io(&dir))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for target in [&parent, &dir] {
            let _ = std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o700));
        }
    }

    for path in moving {
        let Some(name) = path.file_name() else {
            continue;
        };
        std::fs::rename(&path, dir.join(name)).map_err(Error::io(&path))?;
    }
    Ok(Some(dir))
}

/// The signed files set aside so far, oldest folder first.
pub fn set_aside_files(project: &Project) -> Vec<PathBuf> {
    set_aside_dirs(project)
        .into_iter()
        .flat_map(|dir| signed_files(&earlier(project, dir)))
        .collect()
}

/// Only the numbered folders, which the program created.
fn set_aside_dirs(project: &Project) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(project.path(SET_ASIDE)) else {
        return Vec::new();
    };
    let mut dirs: Vec<(u32, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let number = entry.file_name().to_str()?.parse().ok()?;
            Some((number, entry.path()))
        })
        .collect();
    dirs.sort();
    dirs.into_iter().map(|(_, dir)| dir).collect()
}

fn earlier(project: &Project, dir: PathBuf) -> Project {
    Project {
        name: project.name.clone(),
        dir,
    }
}

/// Deletes `wallet.json` only: public, but it watches the wallet forever.
///
/// Call only when no step follows: every chain step reads it. The next
/// ceremony will ask for the seed to rebuild the descriptors. Signed
/// payments are untouched. Returns `true` if the file existed.
pub fn forget_wallet(project: &Project) -> Result<bool> {
    project.remove_file(WALLET)
}

/// Deletes the given files plus `tx-K.hex`: signed, broadcastable, and not
/// in [`FILES`].
fn remove(project: &Project, files: &[&str]) -> Result<usize> {
    let mut removed = 0;
    let mut targets: Vec<PathBuf> = files.iter().map(|file| project.path(file)).collect();

    if let Ok(entries) = std::fs::read_dir(&project.dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Archives go with their originals, or old coins look committed.
            // A write cut short may hold a signed batch: it goes too.
            let ours = crate::wizard::is_per_tx_name(&name)
                || crate::wizard::is_per_tx_archive(&name)
                || crate::wizard::is_temp_name(&name)
                || files
                    .iter()
                    .any(|file| crate::wizard::is_archive_of(file, &name));
            if ours {
                targets.push(entry.path());
            }
        }
    }

    for path in targets {
        if remove_if_present(&path)? {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Every signed batch in the project (current, split, archives): the source
/// of truth for commitments, no separate ledger.
pub fn signed_files(project: &Project) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for name in [BATCH, SPLIT] {
        if project.has(name) {
            files.push(project.path(name));
        }
    }
    if let Ok(entries) = std::fs::read_dir(&project.dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            // `batch-N.json` and `split-N.json`, never `batch-config.json`.
            let archive = [BATCH, SPLIT]
                .iter()
                .any(|file| crate::wizard::is_archive_of(file, &name));
            if archive {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    files
}

/// Every signed batch on this machine: a signature commits its coin
/// wherever the file lives. Returns files; filtering by wallet checksum is
/// the reader's job.
pub fn all_signed_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    for project in list() {
        files.extend(signed_files(&project));
    }
    files.sort();
    files.dedup();
    files
}

pub fn describe(project: &Project) -> String {
    let present: Vec<&str> = FILES.iter().copied().filter(|f| project.has(f)).collect();
    if present.is_empty() {
        return t("vuoto", "empty").to_string();
    }
    let count = present.len();
    let list = present.join(", ");
    tf!({ "{count} file: {list}" }, { "{count} files: {list}" })
}

/// A directory name only: no `..`, no separators.
fn sanitize(name: &str) -> String {
    name.trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '\0' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect::<String>()
        .trim_matches('.')
        .trim()
        .to_string()
}

/// Shortened with `~` under home.
pub fn display(path: &Path) -> String {
    match std::env::var("HOME") {
        Ok(home) if path.starts_with(&home) => path
            .strip_prefix(&home)
            .map(|rest| format!("~/{}", rest.display()))
            .unwrap_or_else(|_| path.display().to_string()),
        _ => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Temp;

    /// Deletion includes archives, and only names the program writes.
    #[test]
    fn wiping_also_takes_the_archives_and_the_wordlist() {
        let home = Temp::new("archivi");
        let project = Project {
            name: "prova".into(),
            dir: home.path.join("cerimonia"),
        };
        std::fs::create_dir_all(&project.dir).expect("cartella creabile");

        let ours = [
            WALLET,
            UTXOS,
            BATCH,
            "batch-1.json",
            "batch-12.json",
            "split-3.json",
            "tx-0.hex",
            "tx-0-1.hex",
            WORDLIST,
        ];
        for file in ours {
            std::fs::write(project.dir.join(file), "x").expect("scrivibile");
        }
        // Look-alikes and a user file.
        for foreign in ["batch-config.json", "tx-uno.hex", "appunti.txt"] {
            std::fs::write(project.dir.join(foreign), "x").expect("scrivibile");
        }
        // `batch-config.json` goes because it is listed, not as an archive.
        let removed = wipe(&project).expect("cancellabile");
        assert_eq!(removed, ours.len() + 1, "nostri + batch-config.json");

        for file in ours {
            assert!(!project.dir.join(file).exists(), "{file} doveva sparire");
        }
        assert!(
            project.dir.join("appunti.txt").exists(),
            "quello che ci hai messo tu resta"
        );
        assert!(
            project.dir.join("tx-uno.hex").exists(),
            "l'indice deve essere un numero: questo non è un file nostro"
        );
        assert!(
            project.dir.exists(),
            "la cartella resta finché dentro c'è roba di qualcun altro"
        );
    }

    /// An archived split commits its coins like an archived batch; only a
    /// number after the dash makes an archive.
    #[test]
    fn split_archives_are_signed_files_and_look_alikes_are_not() {
        let home = Temp::new("archivi-divisione");
        let project = Project {
            name: "prova".into(),
            dir: home.path.join("cerimonia"),
        };
        std::fs::create_dir_all(&project.dir).expect("cartella creabile");
        for name in [
            SPLIT,
            "split-1.json",
            "split-12.json",
            "split-uno.json",
            "split-.json",
            "split-1.hex",
        ] {
            std::fs::write(project.dir.join(name), "{}").expect("scrivibile");
        }

        let names: Vec<String> = signed_files(&project)
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        assert_eq!(names, ["split-1.json", "split-12.json", SPLIT]);

        wipe(&project).expect("cancellabile");
        assert!(signed_files(&project).is_empty(), "vanno con la cerimonia");
    }

    /// A write cut short is neither a ceremony file nor a signed one, and it
    /// goes with the wipe: it may hold signed payments.
    #[test]
    fn a_write_cut_short_is_not_read_and_goes_with_the_wipe() {
        let home = Temp::new("interrotta");
        let project = Project {
            name: "prova".into(),
            dir: home.path.join("cerimonia"),
        };
        std::fs::create_dir_all(&project.dir).expect("cartella creabile");
        std::fs::write(project.path(BATCH), "{}").expect("scrivibile");
        let leftovers = [".batch.json.4242-0.tmp", ".batch-1.json.4242-1.tmp"];
        for name in leftovers {
            std::fs::write(project.dir.join(name), "{}").expect("scrivibile");
        }
        std::fs::write(project.dir.join(".appunti"), "x").expect("scrivibile");

        assert_eq!(signed_files(&project), [project.path(BATCH)]);

        wipe(&project).expect("cancellabile");
        for name in leftovers {
            assert!(!project.dir.join(name).exists(), "{name} doveva sparire");
        }
        assert!(project.dir.join(".appunti").exists(), "non è nostro");
    }

    /// Set aside: every signed file and its hex move to a numbered folder,
    /// nothing is deleted, and no listing reads them any more.
    #[test]
    fn setting_aside_keeps_the_signed_files_out_of_sight() {
        let home = Temp::new("da-parte");
        let project = Project {
            name: "prova".into(),
            dir: home.path.join("cerimonia"),
        };
        std::fs::create_dir_all(&project.dir).expect("cartella creabile");
        let signed = [BATCH, "batch-1.json", SPLIT, "tx-0.hex", "tx-0-1.hex"];
        for file in signed.iter().chain(&[WALLET, UTXOS, CONFIG]) {
            std::fs::write(project.dir.join(file), "{}").expect("scrivibile");
        }

        let first = set_aside(&project)
            .expect("spostati")
            .expect("c'era da spostare");
        assert_eq!(first, project.path(SET_ASIDE).join("1"));
        assert!(signed_files(&project).is_empty(), "fuori vista");
        for file in signed {
            assert!(first.join(file).is_file(), "{file} tenuto");
            assert!(!project.dir.join(file).exists(), "{file} spostato");
        }
        for file in [WALLET, UTXOS, CONFIG] {
            assert!(project.has(file), "{file} non è firmato: resta dov'è");
        }
        assert_eq!(set_aside_files(&project).len(), 3);

        std::fs::write(project.path(BATCH), "{}").expect("scrivibile");
        let second = set_aside(&project)
            .expect("spostati")
            .expect("c'era da spostare");
        assert_eq!(
            second,
            project.path(SET_ASIDE).join("2"),
            "il primo non si tocca"
        );
        assert_eq!(set_aside_files(&project).len(), 4);

        assert!(set_aside(&project).expect("niente da fare").is_none());
        assert!(!project.path(SET_ASIDE).join("3").exists());
    }

    /// Wiping takes what was set aside, folder included.
    #[test]
    fn wiping_takes_what_was_set_aside() {
        let home = Temp::new("da-parte-via");
        let project = Project {
            name: "prova".into(),
            dir: home.path.join("cerimonia"),
        };
        std::fs::create_dir_all(&project.dir).expect("cartella creabile");
        for file in [WALLET, BATCH, "tx-0.hex"] {
            std::fs::write(project.dir.join(file), "{}").expect("scrivibile");
        }
        set_aside(&project).expect("spostati");

        let removed = wipe(&project).expect("cancellabile");
        assert_eq!(removed, 3);
        assert!(set_aside_files(&project).is_empty());
        assert!(!project.dir.exists(), "non resta niente");
    }

    #[test]
    fn the_name_cannot_escape_the_folder() {
        // Whatever the prompt gives stays a single directory name.
        for hostile in [
            "../../etc",
            "..",
            "../..",
            "/etc/passwd",
            "..\\..\\Windows",
            "a/../../b",
            ".",
        ] {
            let safe = sanitize(hostile);
            assert!(!safe.contains('/'), "{hostile} → {safe}");
            assert!(!safe.contains('\\'), "{hostile} → {safe}");
            assert_ne!(safe, "..", "{hostile} → {safe}");
            assert_ne!(safe, ".", "{hostile} → {safe}");

            let joined = std::path::Path::new("/base").join(&safe);
            assert!(
                joined.starts_with("/base"),
                "{hostile} è uscito dalla cartella: {}",
                joined.display()
            );
        }

        assert_eq!(sanitize("cerimonia/2026"), "cerimonia-2026");
    }

    #[test]
    fn ordinary_names_stay_readable() {
        assert_eq!(sanitize(" cold estate 2026 "), "cold estate 2026");
    }
}
