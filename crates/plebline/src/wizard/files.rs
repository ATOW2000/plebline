//! Every file the flows write: owner-only, all or nothing, and never over
//! a signed file that is still there.

use std::path::{Path, PathBuf};

use plebline_core::batch::{self, SignedBatch};
use plebline_core::lang::t;
use plebline_core::tf;
use plebline_core::{Error, Result};

use crate::qr;
use crate::ui;

/// Moves an existing file aside instead of overwriting it.
pub(super) fn archive_existing(path: &Path) -> Result<Option<PathBuf>> {
    if !path.is_file() {
        return Ok(None);
    }
    let parent = path.parent().unwrap_or(Path::new("."));
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "batch".to_string());

    // Keep the extension: `.hex` files come through here too.
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "json".to_string());

    for n in 1..1000 {
        let candidate = parent.join(format!("{stem}-{n}.{ext}"));
        match move_without_replacing(path, &candidate) {
            Ok(()) => {
                sync_folder(folder_of(path));
                return Ok(Some(candidate));
            }
            Err(taken) if taken.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(Error::Io {
                    path: candidate.display().to_string(),
                    source,
                })
            }
        }
    }
    Err(Error::Refused(
        t(
            "troppi batch archiviati: fai spazio nella cartella del progetto",
            "too many archived batches: make room in the project folder",
        )
        .into(),
    ))
}

/// `rename` would silently replace an archive that appeared in between,
/// and the coins it commits would look free. A hard link fails instead if
/// the name is taken; the old name goes only after, so the content is under
/// at least one name at every instant. Where links do not exist (FAT and
/// exFAT sticks), check then rename: a gap only another writer could use.
fn move_without_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::hard_link(from, to) {
        Ok(()) => std::fs::remove_file(from),
        Err(taken) if taken.kind() == std::io::ErrorKind::AlreadyExists => Err(taken),
        Err(_) if to.symlink_metadata().is_ok() => {
            Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
        }
        Err(_) => std::fs::rename(from, to),
    }
}

pub const PER_TX_PREFIX: &str = "tx-";

/// `true` if `name` is an archive of `file` (`batch.json` → `batch-1.json`).
/// Next to `archive_existing` so the two cannot drift. The suffix must be a
/// number: `batch-config.json` is not an archive.
pub fn is_archive_of(file: &str, name: &str) -> bool {
    let Some((stem, ext)) = file.rsplit_once('.') else {
        return false;
    };
    name.strip_prefix(&format!("{stem}-"))
        .and_then(|rest| rest.strip_suffix(&format!(".{ext}")))
        .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
}

/// `tx-1-2.hex`: archive of a per-transaction file.
pub fn is_per_tx_archive(name: &str) -> bool {
    let Some(rest) = name
        .strip_prefix(PER_TX_PREFIX)
        .and_then(|rest| rest.strip_suffix(".hex"))
    else {
        return false;
    };
    let Some((index, archive)) = rest.split_once('-') else {
        return false;
    };
    [index, archive]
        .iter()
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

pub fn is_per_tx_name(name: &str) -> bool {
    name.strip_prefix(PER_TX_PREFIX)
        .and_then(|rest| rest.strip_suffix(".hex"))
        .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
}

/// One `tx-K.hex` per transaction, hex only: pasteable into any push form or
/// `sendrawtransaction`. `K` is the batch index.
pub(super) fn write_per_tx_files(signed: &SignedBatch, out: &Path) -> Result<Vec<PathBuf>> {
    let dir = out.parent().unwrap_or(Path::new("."));
    archive_per_tx_files(dir)?;

    let mut written = Vec::with_capacity(signed.txs.len());
    for tx in &signed.txs {
        let path = dir.join(format!("{PER_TX_PREFIX}{}.hex", tx.index));
        // Trailing newline, for `cat` and scripts.
        write_file(&path, &format!("{}\n", tx.hex))?;
        written.push(path);
    }
    Ok(written)
}

/// Archives (never deletes) the previous per-transaction files, so a stale
/// `tx-2.hex` cannot pass for a new one.
fn archive_per_tx_files(dir: &Path) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    let mut stale: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| is_per_tx_name(&entry.file_name().to_string_lossy()))
        .map(|entry| entry.path())
        .collect();
    stale.sort();
    for path in stale {
        archive_existing(&path)?;
    }
    Ok(())
}

/// QR payload: hex for push forms and nodes, or PSBT for importing wallets
/// (it carries input amounts, so the fee can be shown).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QrContent {
    #[default]
    Hex,
    Psbt,
}

impl QrContent {
    /// Payload and file prefix; distinct prefixes keep the kinds apart.
    pub fn payload(self, tx: &batch::SignedTx) -> Result<(String, &'static str)> {
        match self {
            QrContent::Hex => Ok((tx.hex.clone(), PER_TX_PREFIX)),
            QrContent::Psbt => Ok((batch::shareable_psbt(tx)?, "psbt-")),
        }
    }
}

pub fn write_qr_files(signed: &SignedBatch, dir: &Path, content: QrContent) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(Error::io(dir))?;
    let mut too_big = 0;
    for tx in &signed.txs {
        let (payload, prefix) = content.payload(tx)?;
        if !qr::fits(&payload) {
            too_big += 1;
            continue;
        }
        // The txid comes from the file: a byte slice could panic, a `/`
        // could leave the folder.
        let short = tx
            .txid
            .get(..8)
            .filter(|s| s.chars().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| {
                let number = batch::number(tx.index);
                Error::Verification(tf!(
                    { "la ricarica #{number} ha un identificativo illeggibile: niente QR" },
                    { "refill #{number} has an unreadable identifier: no QR code" }
                ))
            })?;
        let file = dir.join(format!("{prefix}{}-{short}.png", tx.index));
        qr::png(&payload, &file).map_err(Error::Refused)?;
        // The QR holds the whole transaction: same permissions as the JSON.
        restrict(&file)?;
    }
    ui::success(&tf!(
        { "QR delle ricariche scritti in {}", dir.display() },
        { "Refill QR codes written to {}", dir.display() }
    ));
    if too_big > 0 {
        ui::warning(&tf!(
            {
                "{too_big} ricariche sono troppo grandi per un singolo QR: \
              trasferisci il file batch.json su chiavetta."
            },
            {
                "{too_big} refills are too large for a single QR code: move batch.json onto a \
              USB stick instead."
            }
        ));
    }
    Ok(())
}

/// Writes a file readable by its owner only: `wallet.json` and `utxos.json`
/// hold the whole financial history.
///
/// All or nothing: after a crash the file is either the old one or the new
/// one. A truncated signed batch would hide the coins it commits, and a later
/// ceremony could sign them again.
pub fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(Error::io(parent))?;
        }
    }
    write_atomic(path, content.as_bytes()).map_err(Error::io(path))
}

/// Serialize and write. Failure is returned as an error, because after
/// signing a panic would abort the ceremony.
pub fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(value).map_err(|source| Error::Serialize {
        path: path.display().to_string(),
        source,
    })?;
    write_file(path, &json)
}

/// A temporary file beside the destination, synced, then renamed over it:
/// `rename` within a folder swaps the whole file at once. A link planted at
/// the destination is replaced, never written through.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let dir = folder_of(path);
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no file name"))?;
    let (temp, mut file) = create_temp(dir, &name.to_string_lossy())?;
    let synced = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(problem) = synced.and_then(|()| std::fs::rename(&temp, path)) {
        let _ = std::fs::remove_file(&temp);
        return Err(problem);
    }
    sync_folder(dir);
    Ok(())
}

fn folder_of(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// Created new (never an existing file) and owner-only before the first
/// byte: nobody else can read it even for an instant.
fn create_temp(dir: &Path, name: &str) -> std::io::Result<(PathBuf, std::fs::File)> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut attempt = 0;
    loop {
        let temp = dir.join(temp_name(name, attempt));
        match options.open(&temp) {
            Ok(file) => {
                // The umask may have narrowed it further: exactly 0600.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Err(problem) =
                        file.set_permissions(std::fs::Permissions::from_mode(0o600))
                    {
                        let _ = std::fs::remove_file(&temp);
                        return Err(problem);
                    }
                }
                return Ok((temp, file));
            }
            // Left by an earlier crash: take the next name.
            Err(problem) if problem.kind() == std::io::ErrorKind::AlreadyExists && attempt < 64 => {
                attempt += 1;
            }
            Err(problem) => return Err(problem),
        }
    }
}

/// `.batch.json.<pid>-<n>.tmp`: hidden, and matched by no name that is read
/// as a signed file (see [`is_temp_name`]).
fn temp_name(name: &str, attempt: u32) -> String {
    format!(".{name}.{}-{attempt}.tmp", std::process::id())
}

/// A write cut short by a crash. Never a source of commitments: until the
/// rename, the old file is still in place. Wiped with the ceremony.
pub fn is_temp_name(name: &str) -> bool {
    let Some(rest) = name
        .strip_prefix('.')
        .and_then(|rest| rest.strip_suffix(".tmp"))
    else {
        return false;
    };
    let Some((file, tag)) = rest.rsplit_once('.') else {
        return false;
    };
    let Some((pid, attempt)) = tag.split_once('-') else {
        return false;
    };
    !file.is_empty()
        && [pid, attempt]
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

/// Makes the rename itself durable. Best effort: not every system syncs a
/// folder, and the file content is already on disk.
fn sync_folder(dir: &Path) {
    #[cfg(unix)]
    {
        if let Ok(folder) = std::fs::File::open(dir) {
            let _ = folder.sync_all();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
}

#[cfg(unix)]
fn restrict(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(Error::io(path))
}

/// No POSIX permissions here.
#[cfg(not(unix))]
fn restrict(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{batch_of, set_of, Temp};
    use crate::wizard::commitments::committed_outpoints;
    use plebline_core::verify as verifier;

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("leggibile")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn every_transaction_ends_up_in_a_file_of_its_own() {
        let dir = Temp::new("per-tx");
        let signed = batch_of(&[0, 1, 2]);

        let written = write_per_tx_files(&signed, &dir.path.join("batch.json")).expect("scritti");

        assert_eq!(written.len(), 3);
        assert_eq!(names(&dir.path), ["tx-0.hex", "tx-1.hex", "tx-2.hex"]);

        // Only the hex of that index.
        for (n, path) in written.iter().enumerate() {
            let parsed = std::fs::read_to_string(path).expect("leggibile");
            assert_eq!(parsed.trim_end(), signed.txs[n].hex);
            assert!(parsed.ends_with('\n'), "va chiuso da un a capo");
        }
    }

    #[test]
    fn the_previous_ceremonys_files_are_archived_not_overwritten() {
        let dir = Temp::new("archivio");
        let out = dir.path.join("batch.json");

        write_per_tx_files(&batch_of(&[0, 1, 2]), &out).expect("scritti");
        let previous = std::fs::read_to_string(dir.path.join("tx-2.hex")).expect("leggibile");

        // Shorter batch: a stale tx-2.hex must be archived.
        write_per_tx_files(&batch_of(&[0]), &out).expect("scritti");

        assert_eq!(
            names(&dir.path),
            ["tx-0-1.hex", "tx-0.hex", "tx-1-1.hex", "tx-2-1.hex"],
            "i vecchi vanno messi da parte, non cancellati né lasciati in mezzo"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path.join("tx-2-1.hex")).expect("leggibile"),
            previous,
            "l'archivio deve contenere la transazione di prima, per intero"
        );
    }

    #[test]
    fn archives_are_not_archived_again() {
        assert!(is_per_tx_name("tx-0.hex"));
        assert!(is_per_tx_name("tx-12.hex"));
        assert!(!is_per_tx_name("tx-0-1.hex"), "è già una copia archiviata");
        assert!(!is_per_tx_name("tx-.hex"));
        assert!(!is_per_tx_name("tx-0.txt"));
        assert!(!is_per_tx_name("tx-0.json"));
        assert!(!is_per_tx_name("batch.json"));
        assert!(!is_per_tx_name("appunti-tx-0.hex"));
    }

    /// `batch-config.json` is not an archive: mistaken for one, it would make
    /// every guided signing fail.
    #[test]
    fn the_batch_definition_is_not_an_archive() {
        assert!(is_archive_of("batch.json", "batch-1.json"));
        assert!(is_archive_of("batch.json", "batch-12.json"));
        assert!(
            !is_archive_of("batch.json", "batch-config.json"),
            "è la definizione della cerimonia, non un gruppo firmato"
        );
        assert!(!is_archive_of("batch.json", "batch.json"), "è l'originale");
        assert!(!is_archive_of("batch.json", "batch-.json"));
        assert!(!is_archive_of("batch.json", "batch-1.hex"));
        assert!(!is_archive_of("batch.json", "split-1.json"));
        assert!(is_archive_of("split.json", "split-1.json"));
    }

    /// The name tested is the one archiving produces.
    #[test]
    fn the_name_recognized_is_the_one_written() {
        let temp = Temp::new("archivio");

        let batch = temp.path.join(crate::project::BATCH);
        std::fs::write(&batch, "{}").expect("scrivibile");
        let archived = archive_existing(&batch)
            .expect("archiviabile")
            .expect("c'era un file da archiviare");
        let name = archived
            .file_name()
            .expect("ha un nome")
            .to_string_lossy()
            .into_owned();

        assert!(
            is_archive_of(crate::project::BATCH, &name),
            "l'archiviazione ha scritto {name}, che il riconoscitore non accetta"
        );
    }

    #[test]
    fn what_cannot_be_serialized_leaves_no_file_behind() {
        let dir = Temp::new("json-impossibile");
        let path = dir.path.join("impossibile.json");
        let mut non_string_keys = std::collections::BTreeMap::new();
        non_string_keys.insert((1u32, 2u32), 3u32);

        let error = write_json(&path, &non_string_keys).expect_err("chiavi non stringhe");

        assert!(
            matches!(error, Error::Serialize { .. }),
            "errore sbagliato: {error:?}"
        );
        assert!(
            error.to_string().contains("impossibile.json"),
            "l'errore non dice quale file: {error}"
        );
        assert!(!path.exists(), "il file non doveva essere creato");
    }

    #[test]
    fn a_rewrite_leaves_the_new_file_and_nothing_else() {
        let dir = Temp::new("riscrittura");
        let path = dir.path.join(crate::project::BATCH);
        write_json(&path, &batch_of(&[1, 2, 3, 4, 5])).expect("scritto");
        write_json(&path, &batch_of(&[7])).expect("riscritto");

        let back = verifier::load(&path).expect("rileggibile");
        assert_eq!(back.txs.len(), 1);
        assert_eq!(back.txs[0].index, 7);
        assert_eq!(names(&dir.path), [crate::project::BATCH]);
    }

    /// A link planted where a ceremony file goes is replaced, not written
    /// through: the target, wherever it is, stays as it was.
    #[cfg(unix)]
    #[test]
    fn a_link_at_the_destination_is_not_followed() {
        let dir = Temp::new("collegamento");
        let elsewhere = dir.path.join("altrove.txt");
        std::fs::write(&elsewhere, "non toccare").expect("scrivibile");
        let path = dir.path.join(crate::project::BATCH);
        std::os::unix::fs::symlink(&elsewhere, &path).expect("collegamento");

        write_json(&path, &batch_of(&[1])).expect("scritto");

        assert_eq!(
            std::fs::read_to_string(&elsewhere).expect("leggibile"),
            "non toccare"
        );
        let kind = std::fs::symlink_metadata(&path).expect("c'è").file_type();
        assert!(kind.is_file(), "al suo posto c'è un file vero");
    }

    /// Owner only, even when the file was there before with wider rights.
    #[cfg(unix)]
    #[test]
    fn a_rewritten_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = Temp::new("permessi");
        let path = dir.path.join(crate::project::UTXOS);
        std::fs::write(&path, "{}").expect("scrivibile");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");

        write_file(&path, "nuovo").expect("scritto");
        let mode = std::fs::metadata(&path).expect("c'è").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    /// When the rename cannot happen, the temporary file does not stay.
    #[test]
    fn a_write_that_fails_leaves_nothing_behind() {
        let dir = Temp::new("scrittura-fallita");
        // A non-empty folder where the file should go: rename refuses.
        let path = dir.path.join(crate::project::BATCH);
        std::fs::create_dir_all(path.join("dentro")).expect("cartella creabile");

        assert!(write_json(&path, &batch_of(&[1])).is_err());
        assert_eq!(names(&dir.path), [crate::project::BATCH]);
        assert!(path.join("dentro").is_dir(), "la cartella resta com'era");
    }

    #[test]
    fn temporary_names_are_recognized_and_nothing_else() {
        let ours = temp_name(crate::project::BATCH, 3);
        assert!(ours.starts_with('.'), "nascosto: {ours}");
        assert!(is_temp_name(&ours), "{ours}");
        assert!(is_temp_name(".tx-0.hex.12-0.tmp"));
        for other in [
            crate::project::BATCH,
            "batch-1.json",
            ".batch.json.nuovo",
            ".DS_Store",
            ".appunti.tmp",
            ".batch.json.12-x.tmp",
            ".batch.json.-1.tmp",
            "..12-0.tmp",
            "batch.json.12-0.tmp",
        ] {
            assert!(!is_temp_name(other), "{other}");
        }
        // Never mistaken for a signed file or a per-transaction one.
        assert!(!is_archive_of(crate::project::BATCH, &ours));
        assert!(!is_per_tx_name(&temp_name("tx-0.hex", 0)));
        assert!(!is_per_tx_archive(&temp_name("tx-0.hex", 0)));
    }

    /// The archive step fails rather than replace an archive that is
    /// already there: a lost archive would free the coins it commits.
    #[test]
    fn moving_aside_never_replaces_what_is_there() {
        let dir = Temp::new("archivio-occupato");
        let from = dir.path.join(crate::project::BATCH);
        let to = dir.path.join("batch-1.json");
        write_json(&from, &batch_of(&[2])).expect("scritto");
        write_json(&to, &batch_of(&[1])).expect("scritto");

        let error = move_without_replacing(&from, &to).expect_err("occupato");
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(verifier::load(&to).expect("intatto").txs[0].index, 1);
        assert_eq!(verifier::load(&from).expect("intatto").txs[0].index, 2);

        let archived = archive_existing(&from)
            .expect("archiviabile")
            .expect("c'era");
        assert_eq!(archived, dir.path.join("batch-2.json"));
        assert_eq!(verifier::load(&to).expect("intatto").txs[0].index, 1);
        assert_eq!(verifier::load(&archived).expect("spostato").txs[0].index, 2);
        assert!(!from.exists());
    }

    /// A crash anywhere between archiving the old batch and writing the new
    /// one leaves every committed coin in a file that is read.
    #[test]
    fn a_crash_while_signing_again_hides_no_committed_coin() {
        let dir = Temp::new("crollo");
        let project = crate::project::Project {
            name: "prova".into(),
            dir: dir.path.clone(),
        };
        let out = project.path(crate::project::BATCH);
        write_json(&out, &batch_of(&[1, 2])).expect("scritto");
        let committed = |project: &crate::project::Project| {
            committed_outpoints(&crate::project::signed_files(project), &out, None).expect("letti")
        };

        // Crash right after the archive step.
        archive_existing(&out).expect("archiviato");
        assert_eq!(committed(&project), set_of(&[1, 2]));

        // Crash with the new batch still in its temporary file.
        let temp = dir.path.join(temp_name(crate::project::BATCH, 0));
        std::fs::write(&temp, serde_json::to_string(&batch_of(&[3])).expect("json"))
            .expect("scrivibile");
        assert_eq!(committed(&project), set_of(&[1, 2]));
        assert!(!crate::project::signed_files(&project).contains(&temp));

        // Done: the new batch is read too.
        write_json(&out, &batch_of(&[3])).expect("scritto");
        assert_eq!(committed(&project), set_of(&[1, 2, 3]));
    }

    /// The QR file name takes eight characters of a txid read from a file:
    /// cut inside a character, or holding a `/`, it must neither panic nor
    /// write outside the folder.
    #[test]
    fn a_hostile_txid_gets_no_qr_and_no_panic() {
        let dir = Temp::new("qr-ostile");
        let out = dir.path.join("qr");
        for txid in [
            "abc".to_string(),
            "é".repeat(40),
            "../../../../fuori".to_string(),
            String::new(),
        ] {
            let mut signed = batch_of(&[0]);
            signed.txs[0].txid = txid.clone();
            assert!(
                write_qr_files(&signed, &out, QrContent::Hex).is_err(),
                "accettato: {txid:?}"
            );
        }
        assert_eq!(names(&dir.path), ["qr"], "niente fuori dalla cartella");
        assert!(names(&out).is_empty());

        write_qr_files(&batch_of(&[0]), &out, QrContent::Hex).expect("txid buono");
        assert_eq!(names(&out), ["tx-0-00000000.png"]);
    }
}
