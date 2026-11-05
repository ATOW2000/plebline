//! Ceremony output: the signed group and addresses for next time.

use plebline_core::lang::t;
use plebline_core::{tf, Error, Result};

use plebline::ask::{Ask, Question};
use plebline::project::{self, Project};
use plebline::{qr, ui, wizard};

use crate::close::{copy_into, shellexpand};
use crate::{exists, NETWORK};

/// Takes the payments away: onto the phone as on-screen QR codes, or onto a
/// stick. No keys: signed transactions whose recipient and amount cannot
/// change. Returns `true` only if they really left, since closing may then
/// delete the working directory without asking about them.
pub(crate) fn step_export(ceremony: &Project, ask: &mut dyn Ask) -> Result<bool> {
    ui::title(t(
        "Le tue ricariche, da portare via",
        "Your refills, to take away",
    ));

    let phone = t(
        "Sul telefono: te le mostro come QR, una alla volta",
        "On the phone: I show them as QR codes, one at a time",
    );
    let stick = t(
        "Su una chiavetta o in un'altra cartella",
        "On a USB stick or in another folder",
    );
    let how = ask.select(
        Question::new(t("Dove le porti?", "Where are you taking them?")),
        &[phone, stick],
        None,
    )?;
    if how == 0 {
        if to_phone(ceremony, ask)? {
            step_receive(ceremony, None, ask)?;
            return Ok(true);
        }
        ui::blank();
        ui::warning(t(
            "Allora copiale anche su una chiavetta: alla chiusura ti chiedo se cancellare la \
             cartella di lavoro, e lì dentro c'è l'unica copia.",
            "Then copy them onto a stick too: when you close, I ask whether to delete the \
             working folder, and it holds the only copy.",
        ));
    }

    ui::blank();
    ui::note(t(
        "Adesso le copio dove vuoi tu: una chiavetta, una scheda SD, un'altra cartella. Sono \
         transazioni già firmate: chi le ha può mandarle in rete, ma non può cambiare a chi \
         vanno né quanto valgono.",
        "Now I copy them wherever you want: a USB stick, an SD card, another folder. They are \
         already-signed transactions: whoever has them can send them to the network, but cannot \
         change who they go to or how much they are worth.",
    ));
    ui::blank();
    ui::note(t(
        "Di ogni ricarica scrivo due file: l'hex, che serve a trasmetterla, e un QR da \
         fotografare col telefono. Insieme a batch.json, che dice cosa è stato firmato.",
        "For each refill I write two files: the hex, used to broadcast it, and a QR to \
         photograph with your phone. Along with batch.json, which says what was signed.",
    ));

    ui::blank();
    let typed = ask.text(
        Question::new(t("Dove le copio?", "Where do I copy them?")).help(t(
            "il percorso di una cartella, per esempio /media/amnesia/CHIAVETTA. Invio per saltare",
            "the path of a folder, for example /media/amnesia/USBSTICK. Enter to skip",
        )),
        None,
    )?;

    let typed = typed.trim();
    if typed.is_empty() {
        ui::blank();
        let dir = project::display(&ceremony.dir);
        ui::warning(&tf!(
            {
                "Salto la copia. Le ricariche sono soltanto in {dir}, e alla chiusura ti chiedo \
                 se cancellare quella cartella: se la cancelli senza averle portate via, le \
                 perdi."
            },
            {
                "Skipping the copy. The refills are only in {dir}, and when you close I ask \
                 whether to delete that folder: if you delete it without taking them away, you \
                 lose them."
            }
        ));
        // Addresses are shown even without a stick.
        step_receive(ceremony, None, ask)?;
        return Ok(false);
    }

    let dest = std::path::PathBuf::from(shellexpand(typed)).join("plebline-ricariche");
    std::fs::create_dir_all(&dest).map_err(Error::io(&dest))?;

    let batch = plebline_core::verify::load(&ceremony.path(project::BATCH))?;
    let mut copied = 0usize;

    // The whole batch, for third-party verification.
    copy_into(&ceremony.path(project::BATCH), &dest)?;
    copied += 1;

    // Each hex alone is enough to broadcast, anywhere.
    if let Ok(entries) = std::fs::read_dir(&ceremony.dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if wizard::is_per_tx_name(&name) {
                copy_into(&entry.path(), &dest)?;
                copied += 1;
            }
        }
    }

    let content = ask_qr_content(ask)?;
    wizard::write_qr_files(&batch, &dest, content)?;
    copied += batch.txs.len();

    ui::blank();
    ui::success(&tf!(
        { "{copied} file copiati in {}", dest.display() },
        { "{copied} files copied to {}", dest.display() }
    ));
    ui::blank();
    ui::note(t(
        "Controlla che ci siano davvero, poi togli la chiavetta e mettila via. Da qui in avanti \
         quei file sono le tue ricariche: quando scegli «Chiudi e cancella», la cartella di \
         lavoro sparisce e quella sulla chiavetta resta l'unica copia.",
        "Check they are really there, then remove the stick and put it away. From here on those \
         files are your refills: when you choose \"Close and delete\", the working folder goes \
         and the copy on the stick is the only one left.",
    ));

    step_receive(ceremony, Some(&dest), ask)?;
    Ok(true)
}

/// One QR per payment on screen, each confirmed as saved, then all of them.
/// Nothing enters this machine: the phone only reads the screen with its
/// camera.
fn to_phone(ceremony: &Project, ask: &mut dyn Ask) -> Result<bool> {
    let batch = plebline_core::verify::load(&ceremony.path(project::BATCH))?;
    let total = batch.txs.len();

    ui::blank();
    ui::note(t(
        "Per ogni ricarica ti mostro un QR. Inquadralo con il wallet del telefono che userai \
         per mandarlo, o con la fotocamera, e salvalo. Chi lo vede può mandarlo in rete, ma \
         non può cambiare a chi va né quanto vale: tienilo per te come terresti una ricevuta.",
        "For each refill I show a QR code. Scan it with the phone wallet you will send it \
         from, or with the camera, and save it. Whoever sees it can send it to the network, but \
         cannot change who it goes to or how much it is worth: keep it to yourself as you would \
         a receipt.",
    ));
    let content = ask_qr_content(ask)?;
    ui::note(t(
        "Se il QR esce tagliato, allarga la finestra.",
        "If the QR code is cut off, make the window larger.",
    ));

    for (position, tx) in batch.txs.iter().enumerate() {
        let (payload, _) = content.payload(tx)?;
        if !qr::fits(&payload) {
            let index = plebline_core::batch::number(tx.index);
            ui::warning(&tf!(
                { "La ricarica #{index} è troppo grande per un QR solo." },
                { "Refill #{index} is too large for a single QR code." }
            ));
            return Ok(false);
        }
        let drawn = qr::ascii(&payload).map_err(Error::Refused)?;
        loop {
            ui::blank();
            ui::info(&describe(tx, position + 1, total));
            ui::qr(&drawn);
            let saved = ask.confirm(
                Question::new(t(
                    "L'hai salvato sul telefono?",
                    "Have you saved it on the phone?",
                )),
                false,
            )?;
            if saved {
                break;
            }
            ui::note(t("Te lo mostro di nuovo.", "Here it is again."));
        }
    }

    ui::blank();
    Ok(ask.confirm(
        Question::new(&tf!({ "Sul telefono hai tutte le {total} ricariche?" }, {
            "Does the phone hold all {total} refills?"
        })),
        false,
    )?)
}

/// `Ricarica 1 di 3: 50.000 sat a bc1q…xyz`, plus `(#3)` when a payment
/// taken out of the group leaves the numbers apart from the order.
fn describe(tx: &plebline_core::batch::SignedTx, position: usize, total: usize) -> String {
    let number = plebline_core::batch::number(tx.index);
    let paid: Vec<String> = tx
        .outputs
        .iter()
        .filter(|o| !o.is_change)
        .map(|o| {
            format!(
                "{} → {}",
                ui::sats(o.value_sat),
                ui::short_address(&o.address)
            )
        })
        .collect();
    let paid = paid.join(", ");
    if number == position {
        return tf!({ "Ricarica {position} di {total}: {paid}" }, {
            "Refill {position} of {total}: {paid}"
        });
    }
    tf!({ "Ricarica {position} di {total} (#{number}): {paid}" }, {
        "Refill {position} of {total} (#{number}): {paid}"
    })
}

/// Asked: only the user knows what will read the QR codes. Hex first.
fn ask_qr_content(ask: &mut dyn Ask) -> Result<wizard::QrContent> {
    let hex = t("QR code dell'hex", "QR code of the hex");
    let psbt = t("QR code del PSBT", "QR code of the PSBT");
    let index = ask.select(
        Question::new(t(
            "Cosa metto nel QR di ogni ricarica?",
            "What do I put in each refill's QR code?",
        ))
        .help(t(
            "l'hex lo accettano quasi tutti; il PSBT serve ai wallet che importano le \
                 ricariche, come BlueWallet. L'hex di ogni ricarica lo copio comunque",
            "the hex is accepted almost everywhere; the PSBT is for wallets that import \
                 refills, such as BlueWallet. I copy each refill's hex anyway",
        )),
        &[hex, psbt],
        None,
    )?;
    Ok(if index == 1 {
        wizard::QrContent::Psbt
    } else {
        wizard::QrContent::Hex
    })
}

/// The common gap limit: beyond it a rescan may not find the funds.
pub(crate) const MAX_ADDRESSES: usize = 20;

/// Addresses to top up the cold storage without reopening it. Derived from
/// the public half; no secrets involved.
pub(crate) fn step_receive(
    ceremony: &Project,
    dest: Option<&std::path::Path>,
    ask: &mut dyn Ask,
) -> Result<()> {
    // No public half yet (half-done project): nothing to derive.
    if !ceremony.has(project::WALLET) {
        return Ok(());
    }

    ui::title(t(
        "Gli indirizzi per la prossima volta",
        "The addresses for next time",
    ));
    ui::note(t(
        "Prima o poi vorrai mettere altre monete qui dentro. Per farlo serve un indirizzo di \
         questo wallet, e per averlo bisognerebbe riaprirlo, cioè ribattere le tue parole. Te \
         ne do qualcuno adesso, così non serve.",
        "Sooner or later you will want to put more coins in here. To do that you need an address \
         of this wallet, and getting one would mean opening it again, that is, typing your \
         words back in. I give you a few now, so you will not have to.",
    ));
    ui::blank();
    ui::note(t(
        "Sono indirizzi pubblici e servono solo a ricevere: chi li ha può mandarti dei soldi ma \
         non prenderteli. Usane uno per volta, senza riusarli: non è obbligatorio, ma chi \
         osserva la catena collega più facilmente i pagamenti che arrivano sullo stesso \
         indirizzo.",
        "They are public addresses and only serve to receive: whoever has them can send you \
         money but not take it. Use one at a time without reusing them: it is not required, \
         but whoever watches the chain links payments to the same address more easily.",
    ));

    ui::blank();
    let count = ask.range_usize(
        Question::new(t(
            "Quanti indirizzi ti preparo?",
            "How many addresses shall I prepare?",
        ))
        .help(t(
            "uno per ogni volta che pensi di aggiungere monete, da qui alla prossima \
                 cerimonia",
            "one for each time you expect to add coins between now and the next ceremony",
        )),
        1..=MAX_ADDRESSES,
        Some(5),
    )?;

    let file = plebline_core::watch::load(&ceremony.path(project::WALLET), NETWORK)?;
    let wallet = plebline_core::wallet::from_descriptors(&file.external, &file.internal, NETWORK)?;

    // Start after the last index with history (not the last spendable one,
    // which would reuse seen addresses). No snapshot: zero.
    let first = next_free_index(ceremony);

    let addresses: Vec<(u32, String)> = (0..count)
        .map(|n| {
            let index = first + n as u32;
            let address = wallet
                .peek_address(plebline_core::bdk_wallet::KeychainKind::External, index)
                .address
                .to_string();
            (index, address)
        })
        .collect();

    ui::blank();
    for (index, address) in &addresses {
        ui::item(&format!("#{index}"), address);
    }

    match dest {
        Some(dir) => write_addresses(&addresses, dir)?,
        // On screen only: a QR each, and say it is the only copy.
        None => {
            for (index, address) in &addresses {
                ui::blank();
                ui::info(&format!("#{index}"));
                if let Ok(drawn) = qr::ascii(address) {
                    ui::qr(&drawn);
                }
            }
            ui::blank();
            ui::warning(t(
                "Non li ho scritti da nessuna parte: sono solo qui a schermo. Inquadrali col \
                 telefono o copiali a mano prima di chiudere, o dovrai riaprire il wallet per \
                 riaverli.",
                "I have not written them anywhere: they are only here on screen. Scan them with \
                 the phone or copy them by hand before closing, or you will have to open the \
                 wallet again to get them back.",
            ));
        }
    }
    Ok(())
}

/// First never-used receiving index; zero whenever nothing is known.
pub(crate) fn next_free_index(ceremony: &Project) -> u32 {
    let Some(path) = exists(ceremony, project::UTXOS) else {
        return 0;
    };
    let Ok(set) = plebline_core::utxos::load(&path, NETWORK) else {
        return 0;
    };
    set.last_active_indices
        .get(&plebline_core::bdk_wallet::KeychainKind::External)
        .map(|last| last + 1)
        .unwrap_or(0)
}

/// A plain text file (readable anywhere in six months) plus QR codes.
pub(crate) fn write_addresses(addresses: &[(u32, String)], dir: &std::path::Path) -> Result<()> {
    let mut sheet = String::new();
    sheet.push_str(t(
        "Indirizzi di ricezione del tuo cold storage.\n\
         Servono solo a RICEVERE: chi li ha può mandarti monete, non prenderle.\n\
         Usane uno per volta, e mai due volte lo stesso.\n\n",
        "Receive addresses of your cold storage.\n\
         They only serve to RECEIVE: whoever has them can send you coins, not take them.\n\
         Use one at a time, and never the same one twice.\n\n",
    ));
    for (index, address) in addresses {
        sheet.push_str(&format!("#{index}\t{address}\n"));
    }
    wizard::write_file(&dir.join("indirizzi-ricezione.txt"), &sheet)?;

    // A QR failure is not fatal: the text file is there.
    let mut written = 0usize;
    for (index, address) in addresses {
        let file = dir.join(format!("ricevi-{index}.png"));
        match qr::png(address, &file) {
            Ok(()) => written += 1,
            Err(reason) => ui::warning(&reason),
        }
    }

    ui::blank();
    let count = addresses.len();
    let dir = dir.display();
    ui::success(&tf!(
        { "{count} indirizzi scritti in {dir}, con {written} QR da inquadrare" },
        { "{count} addresses written to {dir}, with {written} QR codes to scan" }
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::*;
    use plebline::ask::{Reply, Scripted};
    use plebline::out::{Kind, Mono, Recorder};

    fn qr_lines(notebook: &Recorder) -> Vec<String> {
        notebook
            .lines()
            .into_iter()
            .filter(|l| matches!(l.kind, Kind::Mono(Mono::Qr)))
            .map(|l| l.text)
            .collect()
    }

    /// Counted from 1, as the person counts. A payment taken out leaves a
    /// gap: then the number is said too, or the second QR would claim to be
    /// payment 2 of a group whose second payment is gone.
    #[test]
    fn the_qr_titles_count_from_one() {
        in_italian();
        let temp = Temp::new("numeri");
        let ceremony = temp.ceremony();
        write_batch(&ceremony, &[0, 2]);
        let batch = plebline_core::verify::load(&ceremony.path(project::BATCH)).expect("batch");

        let first = describe(&batch.txs[0], 1, 2);
        let second = describe(&batch.txs[1], 2, 2);
        assert!(first.starts_with("Ricarica 1 di 2:"), "{first}");
        assert!(second.starts_with("Ricarica 2 di 2 (#3):"), "{second}");
        assert!(!first.contains("#0") && !second.contains("#0"));
    }

    /// Each payment's QR is its hex, shown again until saved; all saved
    /// counts as taken away.
    #[test]
    fn the_payments_go_to_the_phone_one_qr_at_a_time() {
        in_italian();
        let temp = Temp::new("telefono");
        let ceremony = temp.ceremony();
        write_batch(&ceremony, &[0, 1]);

        let notebook = Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));
        let mut ask = Scripted::new([
            Reply::like("Sul telefono"),
            Reply::like("hex"),
            // First one not saved: shown again.
            Reply::No,
            Reply::Yes,
            Reply::Yes,
            // All of them on the phone.
            Reply::Yes,
        ]);
        let taken = step_export(&ceremony, &mut ask).expect("esportazione");
        plebline::out::take();

        assert!(taken);
        assert_eq!(ask.remaining(), 0);
        let shown = qr_lines(&notebook);
        let hex = |i: usize| plebline::testing::spending_hex(&format!("{i:064}:0"));
        let first = qr::ascii(&hex(0)).unwrap();
        let second = qr::ascii(&hex(1)).unwrap();
        assert_eq!(shown, [first.clone(), first, second]);
    }

    /// "Not all on the phone" is not taken away: the stick is offered.
    #[test]
    fn not_all_on_the_phone_falls_back_to_the_stick() {
        in_italian();
        let temp = Temp::new("telefono-no");
        let ceremony = Project {
            name: "prova".to_string(),
            dir: temp.path.join("cerimonia"),
        };
        std::fs::create_dir_all(&ceremony.dir).unwrap();
        write_batch(&ceremony, &[0]);
        let stick = temp.path.join("chiavetta");
        std::fs::create_dir_all(&stick).unwrap();

        let mut ask = Scripted::new([
            Reply::like("Sul telefono"),
            Reply::like("hex"),
            Reply::Yes,
            Reply::No,
            Reply::text(stick.display().to_string()),
            Reply::like("hex"),
        ]);
        let taken = step_export(&ceremony, &mut ask).expect("esportazione");

        assert!(taken);
        assert_eq!(ask.remaining(), 0);
        assert!(stick.join("plebline-ricariche/batch.json").exists());
    }
}
