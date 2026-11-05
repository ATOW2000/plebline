//! QR codes: ASCII in the terminal, PNG in a file.

use plebline_core::tf;
use qrcode::render::unicode::Dense1x2;
use qrcode::{EcLevel, QrCode};
use std::path::Path;

fn too_long(e: qrcode::types::QrError) -> String {
    tf!({ "dato troppo lungo per un QR: {e}" }, {
        "data too long for a QR code: {e}"
    })
}

/// Inverted colours: modules must be light on a dark terminal to scan.
pub fn ascii(data: &str) -> Result<String, String> {
    let code =
        QrCode::with_error_correction_level(data.as_bytes(), EcLevel::M).map_err(too_long)?;
    Ok(code
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .quiet_zone(true)
        .build())
}

pub fn png(data: &str, path: &Path) -> Result<(), String> {
    let code =
        QrCode::with_error_correction_level(data.as_bytes(), EcLevel::M).map_err(too_long)?;
    let image = code
        .render::<image::Luma<u8>>()
        .min_dimensions(512, 512)
        .quiet_zone(true)
        .build();
    image.save(path).map_err(|e| {
        let path = path.display();
        tf!({ "scrittura di {path} fallita: {e}" }, {
            "writing {path} failed: {e}"
        })
    })
}

/// Reads [`ascii`] back into modules, `true` = light, quiet zone included:
/// the window paints squares instead of trusting a font's block glyphs.
/// A drawn glyph half is a light module (colours are inverted).
pub fn modules_from_ascii(text: &str) -> Vec<Vec<bool>> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let (top, bottom): (Vec<bool>, Vec<bool>) = line
            .chars()
            .map(|c| (matches!(c, '█' | '▀'), matches!(c, '█' | '▄')))
            .unzip();
        rows.push(top);
        rows.push(bottom);
    }
    // QR codes are square with an odd side: the last text line has no
    // bottom half.
    let side = rows.first().map_or(0, Vec::len);
    rows.truncate(side);
    rows
}

/// Beyond this, refuse rather than print an unreadable QR.
pub const MAX_QR_BYTES: usize = 1_200;

pub fn fits(data: &str) -> bool {
    data.len() <= MAX_QR_BYTES
}

/// Largest QR file we open. A phone photo is well below.
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Decoded-pixel memory cap, checked before allocating (decompression
/// bombs).
const MAX_PIXEL_BYTES: u64 = 256 * 1024 * 1024;

/// Text in a QR image. Callers must parse it as if typed: no extra trust.
pub fn decode_file(path: &Path) -> Result<String, String> {
    let name = path.display();

    let size = std::fs::metadata(path)
        .map_err(|e| {
            tf!({ "non riesco a leggere {name}: {e}" }, {
                "cannot read {name}: {e}"
            })
        })?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(tf!(
            { "{name} è troppo grande per essere l'immagine di un QR" },
            { "{name} is too large to be a QR image" }
        ));
    }

    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_PIXEL_BYTES);

    let mut reader = image::ImageReader::open(path)
        .map_err(|e| {
            tf!({ "non riesco ad aprire {name}: {e}" }, {
                "cannot open {name}: {e}"
            })
        })?
        .with_guessed_format()
        .map_err(|e| {
            tf!({ "non riesco a leggere {name}: {e}" }, {
                "cannot read {name}: {e}"
            })
        })?;
    reader.limits(limits);

    let image = reader.decode().map_err(|e| {
        tf!(
            { "{name} non è un'immagine che so leggere (servono PNG o JPEG): {e}" },
            { "{name} is not an image I can read (PNG or JPEG needed): {e}" }
        )
    })?;

    let gray = image.to_luma8();
    let mut prepared = rqrr::PreparedImage::prepare(gray);

    let mut found: Vec<String> = Vec::new();
    for grid in prepared.detect_grids() {
        // A grid that does not decode is skipped; another may.
        let Ok((_meta, text)) = grid.decode() else {
            continue;
        };
        let text = text.trim().to_string();
        if !text.is_empty() && !found.contains(&text) {
            found.push(text);
        }
    }

    match found.len() {
        0 => Err(tf!({ "in {name} non ho trovato nessun QR leggibile" }, {
            "no readable QR found in {name}"
        })),
        1 => Ok(found.remove(0)),
        // Two different QR codes: choosing would choose who gets paid.
        count => Err(tf!(
            {
                "in {name} ci sono {count} codici diversi: ritaglia l'immagine \
               lasciando solo quello giusto, o scrivi l'indirizzo a mano"
            },
            {
                "{name} holds {count} different codes: crop the image down to the \
               right one, or type the address by hand"
            }
        )),
    }
}

pub fn looks_like_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Temp;

    /// Every module read back from the text matches the code: dark stays
    /// dark, the quiet zone is light.
    #[test]
    fn the_text_qr_reads_back_into_the_same_modules() {
        let data = "0200000001abcdef";
        let code = QrCode::with_error_correction_level(data.as_bytes(), EcLevel::M).unwrap();
        let width = code.width();
        let grid = modules_from_ascii(&ascii(data).unwrap());
        let quiet = 4;
        for y in 0..width {
            for x in 0..width {
                let dark = code[(x, y)] == qrcode::Color::Dark;
                assert_eq!(grid[y + quiet][x + quiet], !dark, "modulo ({x},{y})");
            }
        }
        assert!(grid[0].iter().all(|light| *light), "margine chiaro");
        assert_eq!(grid.len(), width + 2 * quiet, "quadrato");
    }

    /// What the window paints (4 px per module, light = white) is read back
    /// by a QR decoder as the same data.
    #[test]
    fn the_painted_grid_scans_back_to_the_data() {
        let data = "02000000000101abcdef0123456789abcdef0123456789abcdef0123456789";
        let grid = modules_from_ascii(&ascii(data).unwrap());
        let scale = 4;
        let side = (grid.len() * scale) as u32;
        let image = image::GrayImage::from_fn(side, side, |x, y| {
            let light = grid[y as usize / scale][x as usize / scale];
            image::Luma([if light { 255 } else { 0 }])
        });
        let mut prepared = rqrr::PreparedImage::prepare(image);
        let grids = prepared.detect_grids();
        assert_eq!(grids.len(), 1);
        let (_, content) = grids[0].decode().unwrap();
        assert_eq!(content, data);
    }

    #[test]
    fn what_i_write_i_read_back() {
        let dir = Temp::new("qr");
        let file = dir.path.join("indirizzo.png");

        let expected = "bitcoin:tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx?amount=0.001";
        png(expected, &file).unwrap();

        assert_eq!(decode_file(&file).unwrap(), expected);
    }

    #[test]
    fn a_file_that_is_not_an_image_says_so() {
        let dir = Temp::new("qr-no");
        let file = dir.write("appunti.txt", b"non sono un'immagine");

        assert!(decode_file(&file).is_err());
    }

    #[test]
    fn an_image_without_a_qr_says_so() {
        let dir = Temp::new("qr-vuoto");
        let file = dir.path.join("bianco.png");
        image::GrayImage::from_pixel(64, 64, image::Luma([255u8]))
            .save(&file)
            .unwrap();

        let error = decode_file(&file).unwrap_err();
        assert!(error.contains("QR"), "{error}");
    }

    #[test]
    fn the_files_worth_offering_are_recognized() {
        assert!(looks_like_image(Path::new("/media/chiavetta/qr.PNG")));
        assert!(looks_like_image(Path::new("foto.jpeg")));
        assert!(!looks_like_image(Path::new("wallet.json")));
    }
}
