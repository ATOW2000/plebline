//! Wallet construction, from the seed or from public descriptors.
//! The seed is never written, logged or returned.

use bdk_wallet::bitcoin::bip32::{ChildNumber, DerivationPath};
use bdk_wallet::bitcoin::Network;
use bdk_wallet::descriptor::ExtendedDescriptor;
use bdk_wallet::keys::bip39::{Language, Mnemonic};
use bdk_wallet::miniscript::descriptor::DescriptorPublicKey;
use bdk_wallet::miniscript::ForEachKey;
use bdk_wallet::template::Bip84;
use bdk_wallet::{AddressInfo, KeychainKind, Wallet};
use zeroize::{Zeroize, Zeroizing};

use crate::error::{Error, Result};
use crate::tf;

/// Seed words, zeroed on drop. Built only from interactive input, never
/// from argv or the environment.
pub struct SeedWords(String);

impl SeedWords {
    /// The caller zeroes `words` right afterwards.
    pub fn from_interactive_input(words: &str) -> Result<Self> {
        let normalized = words.split_whitespace().collect::<Vec<_>>().join(" ");
        let count = normalized.split(' ').count();
        if !matches!(count, 12 | 15 | 18 | 21 | 24) {
            let mut n = normalized;
            n.zeroize();
            return Err(Error::Seed(tf!(
                { "{count} parole: un seed BIP39 ne ha 12, 15, 18, 21 o 24" },
                { "{count} words: a BIP39 seed has 12, 15, 18, 21 or 24" }
            )));
        }
        Ok(Self(normalized))
    }

    fn mnemonic(&self) -> Result<Mnemonic> {
        Mnemonic::parse_in(Language::English, &self.0).map_err(|e| Error::Seed(e.to_string()))
    }

    /// Checks the BIP39 checksum. Called right after typing, so a wrong word
    /// can be fixed while the written copy is still at hand.
    pub fn check(&self) -> Result<()> {
        self.mnemonic().map(|_| ())
    }

    /// Recomputes the checksum by changing only the last word; returns its BIP39
    /// index.
    ///
    /// For seeds made by hand (dice, cards): entropy bits are kept as rolled,
    /// only the check bits are recomputed. Never for "fixing" an existing seed:
    /// an earlier typo would still yield a valid wallet that is not yours.
    pub fn with_valid_last_word(&self) -> Result<(Self, usize)> {
        // Each of these holds the seed in another form, and is zeroed when
        // dropped, on every exit path.
        let mut indexes = Zeroizing::new(Vec::<usize>::with_capacity(24));
        let mut bit = Zeroizing::new(Vec::<bool>::with_capacity(24 * 11));

        for (position, word) in self.0.split(' ').enumerate() {
            match Language::English.find_word(word) {
                Some(index) => indexes.push(index as usize),
                None => {
                    // Never name the word: this line outlives the program in the terminal
                    // scrollback.
                    let number = position + 1;
                    return Err(Error::Seed(tf!(
                        { "la parola numero {number} non è dell'elenco BIP39" },
                        { "word number {number} is not from the BIP39 list" }
                    )));
                }
            }
        }

        for index in indexes.iter() {
            for position in (0..11).rev() {
                bit.push((index >> position) & 1 == 1);
            }
        }

        // 32 of every 33 bits are entropy; the rest is checksum, recomputed.
        let entropy_bits = bit.len() * 32 / 33;
        let mut entropy = Zeroizing::new(vec![0u8; entropy_bits / 8]);
        for (position, on) in bit.iter().take(entropy_bits).enumerate() {
            if *on {
                entropy[position / 8] |= 1 << (7 - position % 8);
            }
        }

        let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy)
            .map_err(|e| Error::Seed(e.to_string()))?;
        let last = mnemonic
            .word_indices()
            .last()
            .expect("a mnemonic has at least twelve words");
        let words = Zeroizing::new(mnemonic.words().collect::<Vec<_>>().join(" "));
        let seed = Self::from_interactive_input(&words)?;
        Ok((seed, last))
    }
}

impl Drop for SeedWords {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for SeedWords {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SeedWords(<hidden>)")
    }
}

/// BIP84 signing wallet derived from the seed.
///
/// Not zeroed: the passphrase copies handed to BDK, and the derived private
/// key, which stays in RAM for the session. The real protection is an
/// amnesic machine without swap.
pub fn from_seed(seed: &SeedWords, passphrase: Option<&str>, network: Network) -> Result<Wallet> {
    let key = (seed.mnemonic()?, passphrase.map(str::to_owned));
    Wallet::create(
        Bip84(key.clone(), KeychainKind::External),
        Bip84(key, KeychainKind::Internal),
    )
    .network(network)
    .create_wallet_no_persist()
    .map_err(|e| Error::Descriptor(e.to_string()))
}

/// Watch-only wallet from explicit descriptors, checksum included.
pub fn from_descriptors(external: &str, internal: &str, network: Network) -> Result<Wallet> {
    Wallet::create(external.to_string(), internal.to_string())
        .network(network)
        .create_wallet_no_persist()
        .map_err(|e| Error::Descriptor(e.to_string()))
}

/// `true` if the change descriptor is the receiving one's sibling: same
/// script shape and, key by key, same origin and xpub, the path differing
/// only where receiving has branch 0 and change branch 1. A change
/// descriptor from another wallet would make a payment to someone else look
/// like change returning to this wallet.
pub fn one_account(wallet: &Wallet) -> bool {
    let external = wallet.public_descriptor(KeychainKind::External);
    let internal = wallet.public_descriptor(KeychainKind::Internal);
    let (external_keys, internal_keys) = (keys_of(external), keys_of(internal));
    if external_keys.is_empty() || external_keys.len() != internal_keys.len() {
        return false;
    }
    let siblings = external_keys
        .iter()
        .zip(&internal_keys)
        .all(|(e, i)| sibling_keys(e, i));
    let shape = skeleton(external, &external_keys);
    siblings && shape.is_some() && shape == skeleton(internal, &internal_keys)
}

fn keys_of(descriptor: &ExtendedDescriptor) -> Vec<DescriptorPublicKey> {
    let mut keys = Vec::new();
    descriptor.for_each_key(|key| {
        keys.push(key.clone());
        true
    });
    keys
}

fn sibling_keys(external: &DescriptorPublicKey, internal: &DescriptorPublicKey) -> bool {
    match (external, internal) {
        (DescriptorPublicKey::XPub(e), DescriptorPublicKey::XPub(i)) => {
            e.origin == i.origin
                && e.xkey == i.xkey
                && e.wildcard == i.wildcard
                && receive_and_change(&e.derivation_path, &i.derivation_path)
        }
        // A single key has no branches; multipath keys are not used here.
        _ => false,
    }
}

/// Equal paths but for one step: 0 for receiving, 1 for change.
fn receive_and_change(external: &DerivationPath, internal: &DerivationPath) -> bool {
    let (e, i): (&[ChildNumber], &[ChildNumber]) = (external.as_ref(), internal.as_ref());
    if e.len() != i.len() {
        return false;
    }
    let mut apart = e.iter().zip(i).filter(|(a, b)| a != b);
    matches!(
        (apart.next(), apart.next()),
        (
            Some((
                ChildNumber::Normal { index: 0 },
                ChildNumber::Normal { index: 1 }
            )),
            None
        )
    )
}

/// The descriptor text with its keys, in order, replaced by their position:
/// equal skeletons mean the same script shape.
fn skeleton(descriptor: &ExtendedDescriptor, keys: &[DescriptorPublicKey]) -> Option<String> {
    let text = descriptor.to_string();
    let mut rest = text
        .rsplit_once('#')
        .map_or(text.as_str(), |(body, _)| body);
    let mut shape = String::new();
    for (position, key) in keys.iter().enumerate() {
        let shown = key.to_string();
        let at = rest.find(&shown)?;
        shape.push_str(&rest[..at]);
        shape.push_str(&format!("@{position}"));
        rest = &rest[at + shown.len()..];
    }
    shape.push_str(rest);
    Some(shape)
}

/// Public descriptors with checksum, for watch-only use online.
pub fn public_descriptors(wallet: &Wallet) -> (String, String) {
    // `Display` already appends the checksum.
    let render = |keychain| wallet.public_descriptor(keychain).to_string();
    (
        render(KeychainKind::External),
        render(KeychainKind::Internal),
    )
}

/// `count` fresh receiving addresses, never revealed before.
pub fn fresh_addresses(wallet: &mut Wallet, count: u32) -> Vec<AddressInfo> {
    (0..count)
        .map(|_| wallet.reveal_next_address(KeychainKind::External))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &str =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn a_wrong_word_count_is_refused() {
        let err = SeedWords::from_interactive_input("abandon abandon").unwrap_err();
        assert!(matches!(err, Error::Seed(_)));
    }

    #[test]
    fn words_outside_the_wordlist_are_refused() {
        let seed = SeedWords::from_interactive_input(
            "zzzz abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        assert!(from_seed(&seed, None, Network::Regtest).is_err());
    }

    /// Without the checksum, a changed word would open a different, empty
    /// wallet.
    #[test]
    fn the_checksum_rejects_a_changed_word() {
        SeedWords::from_interactive_input(SEED)
            .unwrap()
            .check()
            .expect("il seed dei vettori BIP39 torna");

        let broken = SEED.replace("about", "zoo");
        let err = SeedWords::from_interactive_input(&broken)
            .unwrap()
            .check()
            .unwrap_err();
        assert!(matches!(err, Error::Seed(_)), "{err}");
    }

    #[test]
    fn the_last_word_is_redone_and_the_others_are_not() {
        let rolled = SEED.replace("about", "abstract");
        let seed = SeedWords::from_interactive_input(&rolled).unwrap();
        assert!(seed.check().is_err(), "così com'è non deve tornare");

        let (fixed, last) = seed.with_valid_last_word().unwrap();
        fixed.check().expect("dopo il conto deve tornare");

        let words: Vec<&str> = fixed.0.split(' ').collect();
        assert_eq!(
            words[..11],
            rolled.split(' ').collect::<Vec<_>>()[..11],
            "le parole tirate a mano non si toccano"
        );
        assert_eq!(words[11], "about", "è la dodicesima che torna");
        assert_eq!(Language::English.word_list()[last], "about");

        // Same group of sixteen: same rolled bits.
        let before = Language::English.find_word("abstract").unwrap() as usize;
        assert_eq!(before / 16, last / 16);
    }

    #[test]
    fn a_seed_that_already_checks_out_stays_as_it_is() {
        let seed = SeedWords::from_interactive_input(SEED).unwrap();
        let (same, _) = seed.with_valid_last_word().unwrap();
        assert_eq!(same.0, SEED);
    }

    /// 24 words: eight check bits, nothing hardcoded.
    #[test]
    fn the_same_holds_for_twenty_four_words() {
        const SEED24: &str = "abandon abandon abandon abandon abandon abandon abandon abandon \
                              abandon abandon abandon abandon abandon abandon abandon abandon \
                              abandon abandon abandon abandon abandon abandon abandon art";
        // "art" swapped for another word of its group of 256.
        let art = Language::English.find_word("art").unwrap() as usize;
        let group = art / 256;
        let other = Language::English.word_list()[group * 256];
        let broken = SEED24.replace(" art", &format!(" {other}"));

        let (fixed, last) = SeedWords::from_interactive_input(&broken)
            .unwrap()
            .with_valid_last_word()
            .unwrap();
        assert_eq!(fixed.0, SEED24);
        assert_eq!(last, art);
    }

    #[test]
    fn a_word_outside_the_list_is_not_repeated_in_the_error() {
        let with_typo = SEED.replace("about", "abuot");
        let error = SeedWords::from_interactive_input(&with_typo)
            .unwrap()
            .with_valid_last_word()
            .expect_err("«abuot» non è dell'elenco");

        let text = error.to_string();
        assert!(!text.contains("abuot"), "l'errore ripete la parola: {text}");
        assert!(text.contains("12"), "e non dice quale posizione: {text}");
    }

    #[test]
    fn multiple_spaces_are_normalized() {
        let seed = SeedWords::from_interactive_input(&SEED.replace(' ', "   ")).unwrap();
        assert!(from_seed(&seed, None, Network::Regtest).is_ok());
    }

    #[test]
    fn the_passphrase_changes_the_wallet() {
        let seed = SeedWords::from_interactive_input(SEED).unwrap();
        let a = from_seed(&seed, None, Network::Regtest).unwrap();
        let b = from_seed(&seed, Some("segreta"), Network::Regtest).unwrap();
        assert_ne!(
            a.peek_address(KeychainKind::External, 0).address,
            b.peek_address(KeychainKind::External, 0).address
        );
    }

    #[test]
    fn fresh_addresses_are_all_different() {
        let seed = SeedWords::from_interactive_input(SEED).unwrap();
        let mut wallet = from_seed(&seed, None, Network::Regtest).unwrap();
        let addresses = fresh_addresses(&mut wallet, 5);
        let unique: std::collections::BTreeSet<_> =
            addresses.iter().map(|a| a.address.to_string()).collect();
        assert_eq!(unique.len(), 5);
    }
}
