//! Text messages received with OnionShare (preinstalled on Tails): the way a
//! phone sends the recipient addresses without a stick. OnionShare writes
//! each one as `<data_dir>/<YYYY-MM-DD>/<HHMMSSffffff>-message.txt`.
//!
//! Only text is read, never images: this runs in the process that will hold
//! the seed. What is read gets no privilege, it goes through
//! `plebline_core::address::parse_list`.

use std::path::{Path, PathBuf};

use plebline_core::tf;

/// OnionShare caps messages at 512 KiB; a list of addresses is a few hundred
/// bytes.
const MAX_BYTES: u64 = 64 * 1024;

const SUFFIX: &str = "-message.txt";

#[derive(Debug)]
pub struct Message {
    pub path: PathBuf,
    /// The text, or why it was not read.
    pub text: Result<String, String>,
}

/// Where OnionShare saves what it receives: on Tails in `~/Downloads`,
/// elsewhere in `~/OnionShare`. On Tails the home is in RAM. Not
/// `~/Downloads` on a Mac: reading it makes macOS ask the user for
/// permission, and OnionShare does not save there.
pub fn default_roots() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let mut roots = Vec::new();
    if !cfg!(target_os = "macos") {
        roots.push(home.join("Downloads"));
    }
    roots.push(home.join("OnionShare"));
    roots
}

/// Messages under every root, newest first. Symlinks are skipped: a message
/// is a file OnionShare wrote, nothing else.
pub fn messages(roots: &[PathBuf]) -> Vec<Message> {
    let mut found = Vec::new();
    for root in roots {
        for day in plain_entries(root).into_iter().filter(|p| is_real_dir(p)) {
            for path in plain_entries(&day) {
                let is_message = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(SUFFIX));
                if is_message && is_real_file(&path) {
                    let text = read(&path);
                    found.push(Message { path, text });
                }
            }
        }
    }
    // Date folder and zero-padded time in the name: their order is time
    // order, whichever root the message is in.
    found.sort_by(|a, b| moment(&b.path).cmp(&moment(&a.path)));
    // The same message copied from one folder to the other counts once.
    found.dedup_by(|a, b| moment(&a.path) == moment(&b.path) && a.text == b.text);
    found
}

/// `(day folder, file name)`.
fn moment(path: &Path) -> (Option<&std::ffi::OsStr>, Option<&std::ffi::OsStr>) {
    (path.parent().and_then(Path::file_name), path.file_name())
}

fn read(path: &Path) -> Result<String, String> {
    let size = std::fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .len();
    if size > MAX_BYTES {
        return Err(tf!(
            { "è troppo grande ({size} byte) per essere una lista di indirizzi" },
            { "it is too large ({size} bytes) to be a list of addresses" }
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    String::from_utf8(bytes)
        .map_err(|_| tf!({ "non è testo leggibile" }, { "it is not readable text" }))
}

fn plain_entries(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| !n.starts_with('.'))
        })
        .collect()
}

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

fn is_real_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Temp;

    #[test]
    fn messages_come_newest_first_and_other_files_are_ignored() {
        let root = Temp::new("order");
        root.write("2026-09-30/101500123456-message.txt", b"primo");
        root.write("2026-09-30/113000000001-message.txt", b"secondo");
        root.write("2026-09-29/235959000000-message.txt", b"ieri");
        // An uploaded file, inside the time folder: not a message.
        root.write("2026-09-30/113000000001/foto.png", b"x");
        root.write("2026-09-30/note.txt", b"non un messaggio");

        let texts: Vec<_> = messages(std::slice::from_ref(&root.path))
            .into_iter()
            .map(|m| m.text.unwrap())
            .collect();
        assert_eq!(texts, ["secondo", "primo", "ieri"]);
    }

    /// Tails saves in `~/Downloads`: both folders are read, and the order is
    /// the time of the message, not the folder.
    #[test]
    fn messages_from_both_folders_come_in_time_order() {
        let home = Temp::new("two-roots");
        home.write("Downloads/2026-10-07/151000000000-message.txt", b"tails");
        home.write("OnionShare/2026-10-07/090000000000-message.txt", b"prima");
        home.write("OnionShare/2026-10-07/160000000000-message.txt", b"dopo");

        let roots = [home.path.join("Downloads"), home.path.join("OnionShare")];
        let texts: Vec<_> = messages(&roots)
            .into_iter()
            .map(|m| m.text.unwrap())
            .collect();
        assert_eq!(texts, ["dopo", "tails", "prima"]);
    }

    /// Copied by hand from one folder to the other, it still counts as one
    /// message.
    #[test]
    fn a_message_copied_to_the_other_folder_counts_once() {
        let home = Temp::new("copied");
        home.write("Downloads/2026-10-07/151000000000-message.txt", b"lista");
        home.write("OnionShare/2026-10-07/151000000000-message.txt", b"lista");

        let roots = [home.path.join("Downloads"), home.path.join("OnionShare")];
        assert_eq!(messages(&roots).len(), 1);
    }

    #[test]
    fn the_downloads_folder_is_read_except_on_a_mac() {
        let roots = default_roots();
        let reads_downloads = roots.iter().any(|r| r.ends_with("Downloads"));
        assert_eq!(reads_downloads, !cfg!(target_os = "macos"), "{roots:?}");
        assert!(roots.iter().any(|r| r.ends_with("OnionShare")), "{roots:?}");
    }

    #[test]
    fn a_missing_folder_is_no_message() {
        assert!(messages(&[PathBuf::from("/non/esiste/OnionShare")]).is_empty());
    }

    #[test]
    fn an_oversized_message_is_reported_not_read() {
        let root = Temp::new("big");
        root.write(
            "2026-09-30/120000000000-message.txt",
            &vec![b'a'; MAX_BYTES as usize + 1],
        );
        let found = messages(std::slice::from_ref(&root.path));
        assert_eq!(found.len(), 1);
        assert!(found[0].text.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_message_is_not_followed() {
        let root = Temp::new("link");
        let elsewhere = root.write("altrove.txt", b"segreto");
        std::fs::create_dir_all(root.path.join("2026-09-30")).unwrap();
        std::os::unix::fs::symlink(
            &elsewhere,
            root.path.join("2026-09-30/120000000000-message.txt"),
        )
        .unwrap();
        assert!(messages(std::slice::from_ref(&root.path)).is_empty());
    }
}
