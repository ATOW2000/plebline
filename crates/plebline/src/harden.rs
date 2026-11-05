//! Disables core dumps and, on Linux, marks the process non-dumpable.
//! Best effort: returns what was granted. The key still lives in RAM; see
//! `plebline_core::wallet::from_seed`. The core crate stays
//! `forbid(unsafe_code)`.

use plebline_core::lang::t;

/// The protections the system actually granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Hardened {
    /// A crash leaves no memory image on disk.
    pub no_core_dump: bool,
    /// Linux: `PR_SET_DUMPABLE` (closes `/proc/<pid>/mem` and ptrace to the
    /// same user). macOS: comes from the hardened-runtime signature, so `false`.
    pub not_readable_by_others: bool,
}

pub fn at_startup() -> Hardened {
    Hardened {
        no_core_dump: disable_core_dumps(),
        not_readable_by_others: hide_from_other_processes(),
    }
}

impl Hardened {
    /// What the user should be told: a protection this system can grant but
    /// refused. On macOS memory reads are closed by the signature, not here.
    pub fn shortfall(&self) -> Option<&'static str> {
        let closed_here = cfg!(target_os = "linux");
        match (
            self.no_core_dump || !cfg!(unix),
            self.not_readable_by_others || !closed_here,
        ) {
            (true, true) => None,
            (false, _) => Some(t(
                "Il sistema non ha disattivato i core dump: se il programma si chiudesse di \
                 colpo, la sua memoria, parole comprese, potrebbe finire su disco.",
                "The system did not disable core dumps: if the program crashed, its memory, \
                 words included, could end up on disk.",
            )),
            (true, false) => Some(t(
                "Il sistema non ha chiuso la memoria di questo programma agli altri programmi \
                 dello stesso utente.",
                "The system did not close this program's memory to other programs of the same \
                 user.",
            )),
        }
    }
}

/// Zero core limit regardless of the inherited one.
#[cfg(unix)]
fn disable_core_dumps() -> bool {
    let zero = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `setrlimit` reads the structure it is given and writes nothing
    // of ours. The structure is fully initialized just above.
    unsafe { libc::setrlimit(libc::RLIMIT_CORE, &zero) == 0 }
}

#[cfg(not(unix))]
fn disable_core_dumps() -> bool {
    false
}

/// Non-dumpable: `/proc` entries become root-owned, no ptrace by same user.
#[cfg(target_os = "linux")]
fn hide_from_other_processes() -> bool {
    // SAFETY: `prctl` with `PR_SET_DUMPABLE` takes an integer and touches no
    // memory of ours.
    unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0) == 0 }
}

/// Not available here; on macOS see [`Hardened::not_readable_by_others`].
#[cfg(not(target_os = "linux"))]
fn hide_from_other_processes() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read back with `getrlimit`, not trusting the return value.
    #[test]
    #[cfg(unix)]
    fn after_asking_no_core_dump_is_left() {
        assert!(at_startup().no_core_dump);

        let mut limit = libc::rlimit {
            rlim_cur: 1,
            rlim_max: 1,
        };
        // SAFETY: `getrlimit` writes into the structure we give it, which is
        // initialized and lives for the whole call.
        let read = unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut limit) };
        assert_eq!(read, 0, "getrlimit non risponde");
        assert_eq!(limit.rlim_cur, 0, "il limite non è a zero");
    }
}
