//! The dice/cards guide, for users without a seed yet.
//!
//! It generates no randomness: a person rolls the 128 bits; the program
//! provides the text, the printable list and the checksum arithmetic.
//!
//! Twelve words: eleven rolled in full (11 bits each), the twelfth carries
//! 7 rolled bits plus 4 checksum bits (part of a SHA256), so 16 candidate
//! words remain and exactly one is valid. [`checksum_step`] says which,
//! offline, instead of a browser tool. 24 words would leave 256 candidates,
//! impractical by hand.

use plebline::ask::{Ask, Question};
use plebline::project::Project;
use plebline::{ui, wizard};
use plebline_core::bdk_wallet::keys::bip39::Language;
use plebline_core::lang::t;
use plebline_core::{tf, Error, Result};

/// Defined in `project`, so deleting a directory need not know the guide.
pub use plebline::project::WORDLIST;

const MARKS_PER_WORD: usize = 11;

const BY_HAND: usize = 11;

const WORDS: usize = 12;

/// Rolled marks of the last word; four more bits are checksum.
const MARKS_LAST_WORD: usize = 7;

/// 2^4 candidates for the last word.
const CANDIDATES: usize = 16;

/// The guide's example; its row and word are computed, never hand-written.
const EXAMPLE: [u8; MARKS_PER_WORD] = [1, 0, 1, 0, 0, 0, 1, 1, 0, 1, 1];

const EXAMPLE_LAST: [u8; MARKS_LAST_WORD] = [1, 0, 1, 0, 0, 0, 1];

fn word_list() -> &'static [&'static str; 2048] {
    Language::English.word_list()
}

/// Row number from binary marks, plus one: the printed list starts at 1
/// (BIP39 counts from 0).
fn row(marks: &[u8]) -> usize {
    let value = marks
        .iter()
        .fold(0usize, |acc, mark| acc * 2 + usize::from(*mark == 1));
    value + 1
}

fn word(row: usize) -> &'static str {
    word_list()[row - 1]
}

/// The sixteen rows (inclusive, printed numbering) for a last-word group.
fn candidate_rows(group: usize) -> (usize, usize) {
    let first = group * CANDIDATES + 1;
    (first, first + CANDIDATES - 1)
}

/// Asks whether the user has the words. `false` stops the ceremony: someone
/// going to fetch dice has nothing to type. Cancelling inside the guide
/// comes back to this question; cancelling the question goes further back.
pub fn step_wallet_origin(ceremony: &Project, ask: &mut dyn Ask) -> Result<bool> {
    let already_have = t(
        "Ce le ho già: sono scritte su un foglio",
        "I already have them: they are written on a sheet",
    );
    let make_them = t(
        "Non ce le ho: me le faccio adesso, a mano",
        "I do not have them: I will make them now, by hand",
    );

    loop {
        ui::title(t("Il tuo portafoglio", "Your wallet"));
        ui::note(t(
            "Il tuo portafoglio sta tutto in dodici o ventiquattro parole: chi le ha può spendere \
             le monete da qualunque macchina. Per questo la prima domanda è se ce le hai già.",
            "Your whole wallet comes down to twelve or twenty-four words: whoever has them can \
             spend the coins from any machine. That is why the first question is whether you \
             already have them.",
        ));

        let choice = ask.select(
            Question::new(t(
                "Le parole del tuo portafoglio ce le hai già?",
                "Do you already have your wallet's words?",
            )),
            &[already_have, make_them],
            None,
        )?;

        if choice == 0 {
            return Ok(true);
        }

        match guide(ceremony, ask) {
            Err(Error::Cancelled) => {}
            other => return other,
        }
    }
}

/// The guide itself, ending with the checksum arithmetic.
fn guide(ceremony: &Project, ask: &mut dyn Ask) -> Result<bool> {
    let die = t(
        "Un dado a sei facce (il modo più semplice)",
        "A six-sided die (the simplest way)",
    );
    let cards = t("Un mazzo da 52 carte", "A deck of 52 cards");

    let tool = ask.select(
        Question::new(t("Con cosa le facciamo?", "What shall we make them with?")),
        &[die, cards],
        None,
    )?;
    let with_dice = tool == 0;

    why_by_hand();
    materials(with_dice, ceremony, ask)?;
    how_to_make_a_word(with_dice);
    the_twelfth(with_dice);
    where_they_go();

    if !checksum_step(ask)? {
        return Ok(false);
    }

    ui::blank();
    let ready = ask.confirm(
        Question::new(t(
            "Le hai scritte tutte e dodici? Passo a chiedertele.",
            "Have you written down all twelve? I will ask you for them now.",
        ))
        .help(t(
            "Non c'è nessuna fretta: se devi ancora tirarle, rispondi no e riapri il programma \
             quando hai finito.",
            "There is no hurry: if you still have to roll them, answer no and open the program \
             again when you are done.",
        )),
        true,
    )?;

    if !ready {
        close_and_return();
        return Ok(false);
    }
    Ok(true)
}

/// Nothing to type yet: close and reopen later.
fn close_and_return() {
    ui::blank();
    ui::note(t(
        "Va bene: chiudi pure il programma. Quando hai finito, riaprilo e ricomincia da qui. \
         Non hai perso niente, perché non è ancora stato scritto niente.",
        "That is fine: you can close the program. When you are done, open it again and start \
         from here. Nothing is lost, because nothing has been written yet.",
    ));
}

fn why_by_hand() {
    ui::title(t(
        "Le parole te le fai da solo",
        "You make the words yourself",
    ));
    ui::note(t(
        "Le dodici parole bastano per spendere le tue monete, e chi riesce a indovinarle può \
         spenderle al posto tuo. L'unica protezione è che siano state scelte davvero a caso.",
        "The twelve words are all it takes to spend your coins, and whoever manages to guess \
         them can spend them instead of you. The only protection is that they were picked \
         truly at random.",
    ));
    ui::note(t(
        "Se le sceglie un programma, devi fidarti del programma, della macchina su cui gira e \
         di chi te l'ha data. Un dado sul tavolo invece lo vedi cadere. Questa guida non tira \
         niente al posto tuo: ti spiega come fare, e io vedo le parole per la prima volta \
         quando le scrivi.",
        "If a program picks them, you have to trust the program, the machine it runs on and \
         whoever gave it to you. A die on the table you can watch as it falls. This guide rolls \
         nothing for you: it explains how, and I see the words for the first time when you \
         type them.",
    ));
}

/// What to have on the table, plus the list to print.
fn materials(with_dice: bool, ceremony: &Project, ask: &mut dyn Ask) -> Result<()> {
    ui::blank();
    ui::info(t("Cosa ti serve", "What you need"));
    ui::success(if with_dice {
        t(
            "Un dado a sei facce, di quelli normali, non scheggiato o smussato da una parte. \
             Tiralo su un piano duro: un dado che tende a cadere sempre sulla stessa faccia dà \
             numeri che si indovinano.",
            "One ordinary six-sided die, not chipped or worn on one side. Roll it on a hard \
             surface: a die that tends to land on the same face gives numbers that can be \
             guessed.",
        )
    } else {
        t(
            "Un mazzo da 52 carte, mescolato bene: almeno sette mescolate a riffle.",
            "A deck of 52 cards, well shuffled: at least seven riffle shuffles.",
        )
    });
    ui::success(t(
        "Carta e penna, non il telefono né un file, perché quello che scrivi è il portafoglio.",
        "Paper and pen, not your phone or a file, because what you write down is the wallet.",
    ));
    ui::success(t(
        "L'elenco numerato delle 2048 parole, che è l'unico elenco ammesso: le parole di un \
         seed si prendono da lì e non si inventano.",
        "The numbered list of the 2048 words, which is the only list allowed: a seed's words \
         come from there and are never invented.",
    ));
    ui::success(t(
        "Un posto dove nessuno ti vede, senza telecamere e senza finestre di fronte. Chi vede \
         il foglio può prendere le tue monete.",
        "A place where nobody can see you, with no cameras and no window facing you. Whoever \
         sees the sheet can take your coins.",
    ));

    ui::blank();
    let wants = ask.confirm(
        Question::new(t(
            "Ti scrivo l'elenco delle 2048 parole in un file, così lo stampi?",
            "Shall I write the list of the 2048 words to a file, so you can print it?",
        ))
        .help(t(
            "È l'elenco pubblico di BIP39, uguale per tutti i portafogli del mondo: non c'è \
             niente di tuo dentro.",
            "It is the public BIP39 list, the same for every wallet in the world: there is \
             nothing of yours in it.",
        )),
        true,
    )?;

    if wants {
        let path = ceremony.path(WORDLIST);
        wizard::write_file(&path, &wordlist_file())?;
        let shown_path = plebline::project::display(&path);
        ui::blank();
        ui::success(&tf!({ "L'elenco è qui: {shown_path}" }, {
            "The list is here: {shown_path}"
        }));
        ui::note(t(
            "Stampalo, o tienilo aperto in un'altra finestra. Se questo computer dimentica \
             tutto allo spegnimento, come fa Tails, quel file sparisce con il resto. Non è un \
             problema: si riscrive in un secondo.",
            "Print it, or keep it open in another window. If this computer forgets everything \
             when it shuts down, as Tails does, that file goes with the rest. That is fine: it \
             takes a second to write it again.",
        ));
    }
    Ok(())
}

/// How to make one word, one action per line; the example comes from the
/// same functions. Dice and cards differ only in the first and last step.
fn how_to_make_a_word(with_dice: bool) {
    ui::blank();
    ui::info(t("Come si fa una parola", "How one word is made"));
    ui::note(if with_dice {
        t(
            "Undici tiri fanno una parola. Sono sempre gli stessi sei passaggi.",
            "Eleven rolls make one word. It is always the same six steps.",
        )
    } else {
        t(
            "Undici carte fanno una parola. Sono sempre gli stessi sei passaggi.",
            "Eleven cards make one word. It is always the same six steps.",
        )
    });
    ui::blank();

    ui::note(if with_dice {
        t(
            "1. Tira il dado undici volte. Ogni volta scrivi 0 se esce 1, 2 o 3, e 1 se esce 4, \
             5 o 6.",
            "1. Roll the die eleven times. Each time write 0 if it shows 1, 2 or 3, and 1 if it \
             shows 4, 5 or 6.",
        )
    } else {
        t(
            "1. Mescola il mazzo e scopri undici carte. Per ognuna scrivi 0 se è rossa (cuori o \
             quadri), 1 se è nera (picche o fiori). Il numero sulla carta non conta.",
            "1. Shuffle the deck and turn over eleven cards. For each one write 0 if it is red \
             (hearts or diamonds), 1 if it is black (spades or clubs). The number on the card \
             does not matter.",
        )
    });

    let digits = show(&EXAMPLE);
    ui::note(t(
        "2. Adesso hai una riga di undici cifre. Per esempio:",
        "2. Now you have a row of eleven digits. For example:",
    ));
    ui::item("", &digits);

    ui::note(t(
        "3. Sotto ogni cifra scrivi il numero che le tocca. Sono sempre questi, in quest'ordine:",
        "3. Under each digit write the number that belongs to it. Always these, in this order:",
    ));
    ui::item("", "1024  512  256  128  64  32  16  8  4  2  1");

    let addends = sum_written(&EXAMPLE);
    let example_row = row(&EXAMPLE);
    let sum = example_row - 1;
    ui::note(t(
        "4. Somma solo i numeri che stanno sotto un 1, e lascia stare quelli sotto uno 0:",
        "4. Add up only the numbers sitting under a 1, and leave out those under a 0:",
    ));
    ui::item("", &format!("{addends} = {sum}"));

    let example_word = word(example_row);
    ui::note(t(
        "5. Aggiungi 1: quello è un numero di riga. Vai a quella riga dell'elenco, e la parola \
         che ci trovi è la tua. Scrivila.",
        "5. Add 1: that is a row number. Go to that row of the list, and the word you find there \
         is yours. Write it down.",
    ));
    ui::item(
        "",
        &tf!({ "{sum} + 1 = riga {example_row} → {example_word}" }, {
            "{sum} + 1 = row {example_row} → {example_word}"
        }),
    );

    ui::note(if with_dice {
        t(
            "6. Ricomincia dal punto 1. Di parole così ne servono undici, numerate da 1 a 11: \
             l'ordine è parte del portafoglio quanto le parole.",
            "6. Start again from step 1. You need eleven words like this, numbered 1 to 11: the \
             order is as much part of the wallet as the words are.",
        )
    } else {
        t(
            "6. Rimescola tutto il mazzo e ricomincia dal punto 1. Di parole così ne servono \
             undici, numerate da 1 a 11: l'ordine è parte del portafoglio quanto le parole.",
            "6. Shuffle the whole deck again and start over from step 1. You need eleven words \
             like this, numbered 1 to 11: the order is as much part of the wallet as the words \
             are.",
        )
    });

    if !with_dice {
        ui::blank();
        ui::note(t(
            "Rimescola ogni volta, perché in un mazzo ci sono 26 carte rosse e 26 nere: più ne \
             scopri di fila, più le ultime si indovinano. Undici su cinquantadue vanno bene, \
             quaranta no.",
            "Shuffle every time, because a deck holds 26 red cards and 26 black ones: the more \
             you turn over in a row, the easier the last ones are to guess. Eleven out of \
             fifty-two is fine, forty is not.",
        ));
    }
}

/// The last word, only partly rolled.
fn the_twelfth(with_dice: bool) {
    ui::blank();
    ui::info(t("La dodicesima parola", "The twelfth word"));
    ui::note(t(
        "L'ultima è diversa dalle altre undici. Dentro ci sta un numero di controllo: serve a \
         far accorgere qualsiasi programma se hai copiato male una parola, invece di aprirti in \
         silenzio un portafoglio vuoto che non è il tuo.",
        "The last one is different from the other eleven. Inside it sits a check number: it \
         makes any program notice that you copied a word wrong, instead of silently opening an \
         empty wallet that is not yours.",
    ));
    ui::note(t(
        "Quel numero si calcola, e a mano non si può fare: lo faccio io, qui in fondo. A te \
         tocca solo trovare il punto di partenza.",
        "That number is computed, and you cannot do it by hand: I do it for you, at the end \
         here. All you have to find is the starting point.",
    ));
    ui::blank();

    ui::note(if with_dice {
        t(
            "1. Tira il dado sette volte, non undici. Stessa regola di prima: 1, 2, 3 fanno 0, \
             mentre 4, 5, 6 fanno 1.",
            "1. Roll the die seven times, not eleven. Same rule as before: 1, 2, 3 make 0, and \
             4, 5, 6 make 1.",
        )
    } else {
        t(
            "1. Mescola, e scopri sette carte, non undici. Stessa regola di prima: rossa 0, \
             nera 1.",
            "1. Shuffle, and turn over seven cards, not eleven. Same rule as before: red 0, \
             black 1.",
        )
    });

    let digits = show(&EXAMPLE_LAST);
    let addends = sum_written(&EXAMPLE_LAST);
    let group = row(&EXAMPLE_LAST) - 1;
    ui::note(t(
        "2. Sotto le sette cifre scrivi 64 32 16 8 4 2 1, e somma come prima:",
        "2. Under the seven digits write 64 32 16 8 4 2 1, and add up as before:",
    ));
    ui::item("", &format!("{digits}   →   {addends} = {group}"));

    let (first, last) = candidate_rows(group);
    let first_word = word(first);
    ui::note(&tf!(
        {
            "3. Il totale moltiplicalo per {CANDIDATES} e aggiungi 1: quella è la riga da cui si \
             parte. Vai a quella riga e scrivi quella parola come dodicesima."
        },
        {
            "3. Multiply the total by {CANDIDATES} and add 1: that is the row to start from. Go \
             to that row and write that word down as your twelfth."
        }
    ));
    ui::item(
        "",
        &tf!(
            { "{group} × {CANDIDATES} + 1 = riga {first} → {first_word}" },
            { "{group} × {CANDIDATES} + 1 = row {first} → {first_word}" }
        ),
    );

    ui::note(&tf!(
        {
            "4. Quella parola però non è ancora definitiva: è solo il punto di partenza. La tua \
             è una delle {CANDIDATES} righe che cominciano da lì (nell'esempio, dalla {first} \
             alla {last}), e qual è te lo dico io qui sotto."
        },
        {
            "4. That word is not final yet: it is only the starting point. Yours is one of the \
             {CANDIDATES} rows beginning there (in the example, from {first} to {last}), and I \
             tell you which one below."
        }
    ));
}

/// From the eleven words and the seven rolled marks, the one valid twelfth
/// of sixteen. Guide only: for someone who already has words, a miscopied
/// one would yield a valid wallet that is not theirs. Prints the row number
/// and not the word, which would stay in the scrollback. Nothing is kept;
/// the next step reads the words again from the user's paper.
fn checksum_step(ask: &mut dyn Ask) -> Result<bool> {
    ui::blank();
    ui::title(t(
        "L'ultima parola: il conto lo faccio io",
        "The last word: I do the sum",
    ));
    ui::note(t(
        "È l'ultimo passo, e l'unico che a mano non si fa: il numero di controllo è un pezzo di \
         uno SHA256, e serve una macchina. Lo faccio qui, senza rete e senza browser: le tue \
         parole non escono da questo programma.",
        "This is the last step, and the only one you cannot do by hand: the check number is a \
         piece of a SHA256, and that needs a machine. I do it here, with no network and no \
         browser: your words do not leave this program.",
    ));
    ui::blank();
    let ready = ask.confirm(
        Question::new(t(
            "Hai le tue undici parole e la dodicesima di partenza? Te la calcolo.",
            "Do you have your eleven words and the twelfth to start from? I will work it out.",
        ))
        .help(t(
            "se devi ancora tirarli, rispondi no: si riprende da qui quando riapri.",
            "if you still have to roll them, answer no: we pick up here when you come back.",
        )),
        true,
    )?;
    if !ready {
        close_and_return();
        return Ok(false);
    }

    ui::blank();
    let others = CANDIDATES - 1;
    ui::note(&tf!(
        {
            "Scrivimi le tue {BY_HAND} parole e, come dodicesima, la parola della prima riga del \
             tuo gruppo, cioè quella del punto 3 qui sopra. Le altre {others} non ti servono: te \
             la dico io quale è."
        },
        {
            "Type your {BY_HAND} words and, as the twelfth, the word on the first row of your \
             group, the one from step 3 above. You do not need the other {others}: I tell you \
             which one it is."
        }
    ));

    let rolled = ask.seed(WORDS)?;

    let (correct, index) = rolled.with_valid_last_word()?;
    // Only needed to find the row.
    drop(correct);
    drop(rolled);

    let row = index + 1;
    let position = index % CANDIDATES + 1;

    ui::blank();
    ui::success(&tf!(
        {
            "La tua dodicesima parola è quella della riga {row}: la {position}ª delle \
             {CANDIDATES} del tuo gruppo."
        },
        {
            "Your twelfth word is the one on row {row}: number {position} of the {CANDIDATES} \
             in your group."
        }
    ));
    ui::note(t(
        "La parola non te la scrivo qui: leggila tu dall'elenco stampato. Quello che compare a \
         schermo può restare nella cronologia del terminale, mentre un numero di riga da solo \
         non è un pezzo del tuo portafoglio.",
        "I do not write the word here: read it off the printed list yourself. What appears on \
         the screen can stay in the terminal's scrollback, while a row number on its own is \
         not a piece of your wallet.",
    ));
    ui::blank();
    ui::warning(t(
        "Scrivila adesso sul foglio, come dodicesima. Da questo momento il portafoglio esiste \
         solo su quel foglio: qui dentro non ne resta niente.",
        "Write it on the sheet now, as the twelfth word. From this moment the wallet exists \
         only on that sheet: nothing of it stays in here.",
    ));
    Ok(true)
}

fn where_they_go() {
    ui::blank();
    ui::info(t("Quando le hai tutte", "Once you have them all"));
    ui::note(t(
        "Ricopiale in bella, numerate, e rileggile una per una confrontandole con l'elenco: \
         una lettera sbagliata è una parola diversa. Poi mettile in due posti lontani fra \
         loro, perché un incendio o un furto sono più probabili di qualcuno che le indovina.",
        "Copy them out neatly, numbered, and read them back one by one against the list: one \
         wrong letter is a different word. Then keep them in two places far apart, because a \
         fire or a burglary is more likely than someone guessing them.",
    ));
    ui::warning(t(
        "Non fotografarle, non salvarle in un file e non scriverle su una tastiera collegata a \
         Internet. Nessuno te le chiede per aiutarti: chi te le chiede vuole rubarti le monete.",
        "Do not photograph them, save them in a file or type them on a keyboard connected to \
         the internet. Nobody asks for them to help you: whoever asks wants to steal your \
         coins.",
    ));
}

fn show(marks: &[u8]) -> String {
    marks
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `1024 + 256 + …`, computed from the marks.
fn sum_written(marks: &[u8]) -> String {
    let last = marks.len() - 1;
    let addends: Vec<String> = marks
        .iter()
        .enumerate()
        .filter(|(_, mark)| **mark == 1)
        .map(|(position, _)| (1usize << (last - position)).to_string())
        .collect();
    addends.join(" + ")
}

fn wordlist_file() -> String {
    let mut out = String::with_capacity(24 * 1024);
    out.push_str(t(
        "Elenco BIP39 in inglese: 2048 parole, numerate da 1 a 2048.\n\
         È lo stesso elenco per tutti i portafogli del mondo, e non c'è niente di tuo qui \
         dentro.\n\
         Ogni parola del tuo seed è una riga di questo elenco.\n\n",
        "BIP39 English list: 2048 words, numbered 1 to 2048.\n\
         It is the same list for every wallet in the world, and there is nothing of yours in \
         it.\n\
         Each word of your seed is one row of this list.\n\n",
    ));
    for (i, word) in word_list().iter().enumerate() {
        out.push_str(&format!("{:>4}  {word}\n", i + 1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::*;
    use plebline::ask::{Reply, Scripted};
    use plebline_core::wallet::SeedWords;

    /// The guide's arithmetic at both ends and on the example.
    #[test]
    fn the_row_number_comes_out_of_the_marks() {
        assert_eq!(row(&[0; MARKS_PER_WORD]), 1);
        assert_eq!(row(&[1; MARKS_PER_WORD]), 2048);
        assert_eq!(word(1), "abandon");
        assert_eq!(word(2048), "zoo");

        assert_eq!(row(&EXAMPLE), 1308);
        assert_eq!(word(1308), "pet");
    }

    #[test]
    fn the_examples_sum_is_the_sum_of_its_marks() {
        assert_eq!(sum_written(&EXAMPLE), "1024 + 256 + 16 + 8 + 2 + 1");
        assert_eq!(sum_written(&EXAMPLE_LAST), "64 + 16 + 1");
        assert_eq!(sum_written(&[0; MARKS_PER_WORD]), "");
        assert_eq!(sum_written(&[1, 1]), "2 + 1");
    }

    /// Candidate rows for the example and both extremes, within the list.
    #[test]
    fn the_candidates_are_sixteen_rows_in_a_row() {
        assert_eq!(candidate_rows(0), (1, 16));
        assert_eq!(candidate_rows(127), (2033, 2048));

        let group = row(&EXAMPLE_LAST) - 1;
        assert_eq!(group, 81);
        assert_eq!(candidate_rows(group), (1297, 1312));
        assert_eq!(word(1297), "pear");
        assert_eq!(word(1312), "physical");
    }

    #[test]
    fn only_one_of_the_sixteen_candidates_checks_out() {
        // Fixed generator: same round every run.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next_value = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        for _ in 0..25 {
            let mut words: Vec<&str> = (0..BY_HAND)
                .map(|_| word((next_value() % 2048) as usize + 1))
                .collect();
            let group = (next_value() % 128) as usize;
            let (first, last) = candidate_rows(group);

            let mut good: Vec<&str> = Vec::new();
            for candidate in (first..=last).map(word) {
                words.push(candidate);
                let phrase = words.join(" ");
                words.pop();
                let matches = SeedWords::from_interactive_input(&phrase)
                    .and_then(|seed| seed.check())
                    .is_ok();
                if matches {
                    good.push(candidate);
                }
            }

            assert_eq!(
                good.len(),
                1,
                "delle {CANDIDATES} candidate del gruppo {group} ne tornano {}: {good:?} \
                 (prime {BY_HAND}: {words:?})",
                good.len()
            );
        }
    }

    #[test]
    fn the_printed_list_is_the_bip39_one() {
        let file = wordlist_file();
        let lines: Vec<&str> = file.lines().collect();
        assert!(lines.iter().any(|r| r.ends_with("  abandon")));
        assert!(lines.first().expect("intestazione").contains("2048"));

        let numbered: Vec<&&str> = lines
            .iter()
            .filter(|r| r.starts_with(' ') || r.starts_with(char::is_numeric))
            .collect();
        assert_eq!(numbered.len(), 2048, "una riga per parola");
        assert_eq!(numbered[0].trim(), "1  abandon");
        assert_eq!(numbered[2047].trim(), "2048  zoo");
    }

    /// The guide writes the word list where it says, and nothing else.
    #[test]
    fn without_words_the_guide_runs_and_writes_the_list() {
        in_italian();
        let temp = Temp::new("guida");
        let ceremony = temp.ceremony();

        let mut ask = Scripted::new([
            Reply::like("Non ce le ho"),
            Reply::like("dado"),
            Reply::Yes,
            // Yes to the arithmetic; eleven rolled words plus the group's first row.
            Reply::Yes,
            Reply::seed(SEED12.replace("about", "abandon")),
            Reply::Yes,
        ]);
        let proceed = super::step_wallet_origin(&ceremony, &mut ask).expect("la guida gira");

        assert!(proceed, "chi ha finito prosegue con l'inserimento");
        assert!(
            ceremony.has(super::WORDLIST),
            "l'elenco delle 2048 parole deve essere sul disco"
        );
        assert!(
            !ceremony.has(plebline::project::WALLET),
            "la guida non crea nessun portafoglio: le parole le fa una persona"
        );
    }

    /// Eleven `abandon` plus `abandon` (row 1 of group 0) gives row 4,
    /// `about`: the BIP39 test vector. Only the row is printed.
    #[test]
    fn at_the_end_of_the_guide_the_program_says_which_the_twelfth_is() {
        in_italian();
        let temp = Temp::new("conto");
        let ceremony = temp.ceremony();
        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));

        let mut ask = Scripted::new([
            Reply::like("Non ce le ho"),
            Reply::like("dado"),
            Reply::No,
            Reply::Yes,
            Reply::seed(SEED12.replace("about", "abandon")),
            Reply::Yes,
        ]);
        let proceed = super::step_wallet_origin(&ceremony, &mut ask).expect("la guida gira");
        plebline::out::take();

        assert!(proceed);
        assert_eq!(ask.remaining(), 0);

        let said = notebook.text();
        assert!(
            said.contains("riga 4"),
            "deve dire la riga della parola giusta: {said}"
        );
        assert!(
            said.contains("4ª delle 16"),
            "e la sua posizione fra le sedici: {said}"
        );
        assert!(
            !said.contains("about"),
            "la parola non si stampa: resterebbe sullo schermo"
        );
        assert!(
            !ceremony.has(plebline::project::WALLET),
            "il conto non crea niente: è un conto"
        );
    }

    #[test]
    fn having_the_words_already_skips_the_arithmetic() {
        in_italian();
        let temp = Temp::new("niente-conto");
        let ceremony = temp.ceremony();
        let notebook = plebline::out::Recorder::new();
        plebline::out::to(Box::new(notebook.clone()));

        let mut ask = Scripted::new([Reply::like("Ce le ho già")]);
        let proceed = super::step_wallet_origin(&ceremony, &mut ask).expect("si prosegue");

        // Bad checksum: `step_seed` offers no twelfth word.
        let mut ask = Scripted::new([
            Reply::like("12"),
            Reply::seed(SEED12.replace("about", "abandon")),
            Reply::Yes,
            Reply::seed(SEED12),
            Reply::text(""),
        ]);
        crate::seed::step_seed(&mut ask).expect("riscritte, tornano");
        plebline::out::take();

        assert!(proceed);
        let said = notebook.text();
        assert!(
            !said.contains("riga 4"),
            "a chi ha già le sue parole non si dice quale metterci: {said}"
        );
        assert!(
            said.contains("non è possibile dire quale"),
            "si dice solo che il conto non torna: {said}"
        );
    }
}
