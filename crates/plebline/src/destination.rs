//! Turning user input into a destination: a pasted address, a BIP 21 URI,
//! or the path of an image with a QR code (Tails has no camera; the address
//! arrives on a stick). QR text gets no privilege: it goes through the same
//! `plebline_core::address::parse`.

use std::path::{Path, PathBuf};

use plebline_core::address::{self, Destination};
use plebline_core::bitcoin::Network;
use plebline_core::lang::t;
use plebline_core::tf;

use crate::qr;

/// Destination and origin: a QR-read address is echoed back, since nobody
/// read it by eye.
#[derive(Debug)]
pub struct Interpreted {
    pub destination: Destination,
    pub from_file: Option<PathBuf>,
}

/// Errors come back as localized strings, shown under the field.
pub fn interpret(input: &str, network: Network) -> Result<Interpreted, String> {
    let text = strip_file_uri(input.trim());

    if let Some(path) = as_existing_file(&text) {
        let text = qr::decode_file(&path)?;
        let destination = address::parse(&text, network).map_err(|e| {
            tf!(
                { "il QR in {} dice «{text}», e non è una destinazione valida: {e}",
                  path.display() },
                { "the QR in {} says «{text}», which is not a valid destination: {e}",
                  path.display() }
            )
        })?;
        return Ok(Interpreted {
            destination,
            from_file: Some(path),
        });
    }

    // Path-like input gets a path error, not an address parser complaint.
    if looks_like_path(&text) {
        let path = Path::new(&text);
        return Err(if path.is_file() {
            tf!(
                { "{text} c'è, ma non è un'immagine: servono PNG o JPEG" },
                { "{text} is there, but it is not an image: PNG or JPEG needed" }
            )
        } else {
            tf!({ "il file {text} non c'è" }, { "there is no file {text}" })
        });
    }

    Ok(Interpreted {
        destination: address::parse(&text, network).map_err(|e| e.to_string())?,
        from_file: None,
    })
}

/// Groups of four, for comparing by eye.
pub fn in_groups(address: &str) -> String {
    let mut out = String::with_capacity(address.len() + address.len() / 4);
    for (index, character) in address.chars().enumerate() {
        if index > 0 && index % 4 == 0 {
            out.push(' ');
        }
        out.push(character);
    }
    out
}

pub fn help_text() -> &'static str {
    t(
        "incolla l'indirizzo (Ctrl+V, o Ctrl+Shift+V nel terminale), oppure dai il percorso di \
         un'immagine con dentro un QR",
        "paste the address (Ctrl+V, or Ctrl+Shift+V in a terminal), or give the path of an image \
         holding a QR code",
    )
}

/// Images on mounted sticks (`/media/amnesia/<label>` on Tails) and home.
/// Two levels deep, capped, skipping hidden directories, so it stays a quick
/// look.
pub fn images_on_sticks() -> Vec<PathBuf> {
    const MAX_FILE: usize = 200;
    let mut found = Vec::new();

    for root in mount_points() {
        collect(&root, 2, MAX_FILE, &mut found);
        if found.len() >= MAX_FILE {
            break;
        }
    }

    found.sort();
    found
}

fn mount_points() -> Vec<PathBuf> {
    let mut places: Vec<PathBuf> = Vec::new();

    places.extend(subfolders(Path::new("/media/amnesia")));
    for root in ["/media", "/run/media"] {
        for user in subfolders(Path::new(root)) {
            places.extend(subfolders(&user));
        }
    }
    places.extend(subfolders(Path::new("/Volumes")));

    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        places.push(home.join("Persistent"));
        places.push(home.join("Downloads"));
        places.push(home);
    }

    places.retain(|p| p.is_dir());
    places.dedup();
    places
}

fn subfolders(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|v| v.path())
        .filter(|p| p.is_dir() && !hidden(p))
        .collect()
}

fn collect(folder: &Path, depth: usize, cap: usize, into: &mut Vec<PathBuf>) {
    if into.len() >= cap {
        return;
    }
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if hidden(&path) {
            continue;
        }
        if path.is_dir() {
            if depth > 0 {
                collect(&path, depth - 1, cap, into);
            }
        } else if qr::looks_like_image(&path) {
            into.push(path);
        }
        if into.len() >= cap {
            return;
        }
    }
}

fn hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.'))
}

/// File managers sometimes give `file:///path`. In a path `+` is a plus,
/// not the space it stands for in a payment URI.
fn strip_file_uri(text: &str) -> String {
    match text.strip_prefix("file://") {
        Some(path) => address::percent_decode(&path.replace('+', "%2B")),
        None => text.to_string(),
    }
}

/// Existing file with an image extension: only that reaches the decoder,
/// which runs in the process holding the seed.
fn as_existing_file(text: &str) -> Option<PathBuf> {
    let path = PathBuf::from(text);
    (path.is_file() && qr::looks_like_image(&path)).then_some(path)
}

fn looks_like_path(text: &str) -> bool {
    text.starts_with('/') || text.starts_with("./") || text.starts_with("~/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Temp;

    #[test]
    fn a_plus_in_a_dragged_path_stays_a_plus() {
        assert_eq!(
            strip_file_uri("file:///tmp/qr+1%20bis.png"),
            "/tmp/qr+1 bis.png"
        );
    }

    const SIGNET: Network = Network::Signet;

    /// A non-image file never reaches the decoder, and is not called missing.
    #[test]
    fn an_existing_file_that_is_not_an_image_is_told_apart_from_a_missing_one() {
        let dir = Temp::new("dest-nonimg");
        let not_an_image = dir.write("appunti.txt", b"non sono un'immagine");

        let error = interpret(not_an_image.to_str().unwrap(), SIGNET)
            .expect_err("un .txt non è una destinazione");
        assert!(error.contains("non è un'immagine"), "{error}");
        assert!(!error.contains("non c'è"), "{error}");

        let missing = dir.path.join("questo-non-esiste.png");
        let error =
            interpret(missing.to_str().unwrap(), SIGNET).expect_err("il file non esiste davvero");
        assert!(error.contains("non c'è"), "{error}");
    }
    const TB: &str = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";

    #[test]
    fn a_pasted_address_passes() {
        let parsed = interpret(TB, SIGNET).unwrap();
        assert_eq!(parsed.destination.address.to_string(), TB);
        assert!(parsed.from_file.is_none());
    }

    #[test]
    fn a_qr_in_a_file_is_read_and_carries_the_amount_too() {
        let dir = Temp::new("dest-qr");
        let file = dir.path.join("da-stampare.png");
        qr::png(&format!("bitcoin:{TB}?amount=0.002"), &file).unwrap();

        let parsed = interpret(file.to_str().unwrap(), SIGNET).unwrap();
        assert_eq!(parsed.destination.address.to_string(), TB);
        assert_eq!(parsed.destination.amount_sat, Some(200_000));
        assert_eq!(parsed.from_file.as_deref(), Some(file.as_path()));
    }

    #[test]
    fn a_mainnet_qr_inside_a_signet_ceremony_stops() {
        let dir = Temp::new("dest-rete");
        let file = dir.path.join("mainnet.png");
        qr::png("bitcoin:bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4", &file).unwrap();

        let error = interpret(file.to_str().unwrap(), SIGNET).unwrap_err();
        assert!(error.contains("signet"), "{error}");
    }

    #[test]
    fn a_path_that_does_not_exist_says_so_as_a_path() {
        let error = interpret("/media/amnesia/CHIAVETTA/qr.png", SIGNET).unwrap_err();
        assert!(error.contains("qr.png"), "{error}");
    }

    #[test]
    fn the_drag_and_drop_file_uri_is_understood() {
        let dir = Temp::new("dest-trascinato");
        let file = dir.path.join("con spazio.png");
        qr::png(&format!("bitcoin:{TB}"), &file).unwrap();

        let uri = format!("file://{}", file.to_str().unwrap().replace(' ', "%20"));
        let parsed = interpret(&uri, SIGNET).unwrap();
        assert_eq!(parsed.destination.address.to_string(), TB);
    }

    #[test]
    fn groups_of_four_are_there_for_comparing() {
        assert_eq!(in_groups("tb1qw508d6"), "tb1q w508 d6");
    }
}
