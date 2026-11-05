//! Interface language: global, because the window's thread and the
//! ceremony's must speak the same one. Italian by default. Tests run in
//! parallel and match on translated text, so each keeps its own with
//! [`set_for_this_thread`].

use std::cell::Cell;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Lang {
    #[default]
    It,
    En,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::It => "it",
            Lang::En => "en",
        }
    }
}

impl std::str::FromStr for Lang {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "it" | "ita" | "italiano" | "italian" => Ok(Lang::It),
            "en" | "eng" | "inglese" | "english" => Ok(Lang::En),
            other => Err(crate::tf!(
                { "lingua sconosciuta '{other}': usa 'it' o 'en'" },
                { "unknown language '{other}': use 'it' or 'en'" }
            )),
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static THIS_THREAD: Cell<Option<Lang>> = const { Cell::new(None) };
}

/// On a thread with its own language, changes only that one.
pub fn set(lang: Lang) {
    if THIS_THREAD.with(Cell::get).is_some() {
        THIS_THREAD.with(|own| own.set(Some(lang)));
    } else {
        CURRENT.store(lang as u8, Ordering::Relaxed);
    }
}

/// For tests: a language for this thread alone.
pub fn set_for_this_thread(lang: Lang) {
    THIS_THREAD.with(|own| own.set(Some(lang)));
}

pub fn current() -> Lang {
    if let Some(own) = THIS_THREAD.with(Cell::get) {
        return own;
    }
    match CURRENT.load(Ordering::Relaxed) {
        0 => Lang::It,
        _ => Lang::En,
    }
}

/// Both versions of a sentence, inline, next to the code it explains.
pub fn t(it: &'static str, en: &'static str) -> &'static str {
    match current() {
        Lang::It => it,
        Lang::En => en,
    }
}

/// [`t`] with format arguments.
///
/// ```ignore
/// tf!({"versione {}", v}, {"version {}", v})
/// ```
#[macro_export]
macro_rules! tf {
    ({$($it:tt)*}, {$($en:tt)*}) => {
        match $crate::lang::current() {
            $crate::lang::Lang::It => format!($($it)*),
            $crate::lang::Lang::En => format!($($en)*),
        }
    };
}
