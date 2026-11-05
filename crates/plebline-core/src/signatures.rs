//! Cryptographic signature verification from the transaction bytes alone:
//! BIP143 sighash recomputed, ECDSA checked against the witness key.
//!
//! The sighash commits to input amounts, recipients, sequence and locktime,
//! so a valid signature also proves the declared amounts and fee.
//!
//! Only native P2WPKH (what BIP84 produces) is verified; anything else is
//! reported as unsupported, never as valid.

use bdk_wallet::bitcoin::hashes::Hash;
use bdk_wallet::bitcoin::secp256k1::{Message, Secp256k1, VerifyOnly};
use bdk_wallet::bitcoin::sighash::{EcdsaSighashType, SighashCache};
use bdk_wallet::bitcoin::{ecdsa, Amount, PublicKey, ScriptBuf, Transaction};

use crate::lang::t;
use crate::tf;

/// Outcome of verifying a single input.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// Signature valid for the key in the witness and for the declared amount.
    Valid,
    /// A signature that does not verify.
    Invalid { reason: String },
    /// An input type this check cannot verify.
    Unsupported { reason: String },
}

#[derive(Debug, Clone)]
pub struct InputCheck {
    pub index: usize,
    pub outcome: Outcome,
}

impl InputCheck {
    pub fn is_valid(&self) -> bool {
        matches!(self.outcome, Outcome::Valid)
    }
}

/// Verifies every input of `tx`. `values` are the input amounts in order;
/// inputs without one are reported unsupported, not valid.
pub fn check(tx: &Transaction, values: &[Amount]) -> Vec<InputCheck> {
    let secp = Secp256k1::verification_only();
    // BIP143 intermediate digests are shared by all inputs.
    let mut cache = SighashCache::new(tx);

    (0..tx.input.len())
        .map(|index| InputCheck {
            index,
            outcome: check_input(&secp, &mut cache, tx, index, values.get(index).copied()),
        })
        .collect()
}

fn check_input(
    secp: &Secp256k1<VerifyOnly>,
    cache: &mut SighashCache<&Transaction>,
    tx: &Transaction,
    index: usize,
    value: Option<Amount>,
) -> Outcome {
    let input = &tx.input[index];

    if input.witness.is_empty() {
        return Outcome::Invalid {
            reason: t(
                "nessun testimone: l'input non è firmato",
                "no witness: the input is not signed",
            )
            .into(),
        };
    }

    // Legacy or P2SH-wrapped inputs are not produced by this wallet and are
    // not verified.
    if !input.script_sig.is_empty() {
        return Outcome::Unsupported {
            reason: t(
                "input non nativo SegWit (scriptSig non vuoto)",
                "input is not native SegWit (scriptSig is not empty)",
            )
            .into(),
        };
    }

    if input.witness.len() != 2 {
        let parts = input.witness.len();
        return Outcome::Unsupported {
            reason: tf!(
                { "testimone di {parts} elementi: non è un P2WPKH, che ne ha esattamente due" },
                { "witness with {parts} items: not a P2WPKH, which has exactly two" }
            ),
        };
    }

    let Some(value) = value else {
        return Outcome::Unsupported {
            reason: t(
                "importo dell'input non disponibile: senza non si può calcolare il sighash",
                "input amount not available: without it the sighash cannot be computed",
            )
            .into(),
        };
    };

    let signature_bytes = &input.witness[0];
    let pubkey_bytes = &input.witness[1];

    let pubkey = match PublicKey::from_slice(pubkey_bytes) {
        Ok(pubkey) => pubkey,
        Err(e) => {
            return Outcome::Invalid {
                reason: tf!({ "la chiave pubblica nel testimone non è valida: {e}" }, {
                    "the public key in the witness is not valid: {e}"
                }),
            }
        }
    };

    // P2WPKH requires compressed keys.
    let Ok(wpubkey_hash) = pubkey.wpubkey_hash() else {
        return Outcome::Invalid {
            reason: t(
                "la chiave pubblica non è compressa: P2WPKH non lo consente",
                "the public key is not compressed: P2WPKH does not allow it",
            )
            .into(),
        };
    };
    let script_pubkey = ScriptBuf::new_p2wpkh(&wpubkey_hash);

    let signature = match ecdsa::Signature::from_slice(signature_bytes) {
        Ok(signature) => signature,
        Err(e) => {
            return Outcome::Invalid {
                reason: tf!({ "la firma non è leggibile: {e}" }, {
                    "the signature cannot be read: {e}"
                }),
            }
        }
    };

    // Only SIGHASH_ALL commits to the whole transaction: anything else lets
    // the file holder change outputs or add inputs.
    if signature.sighash_type != EcdsaSighashType::All {
        let found = signature.sighash_type;
        return Outcome::Invalid {
            reason: tf!(
                {
                    "tipo di firma {found}, non SIGHASH_ALL: la transazione resterebbe \
                     modificabile dopo la firma"
                },
                {
                    "signature type {found}, not SIGHASH_ALL: the transaction would stay \
                     modifiable after signing"
                }
            ),
        };
    }

    let sighash =
        match cache.p2wpkh_signature_hash(index, &script_pubkey, value, signature.sighash_type) {
            Ok(sighash) => sighash,
            Err(e) => {
                return Outcome::Invalid {
                    reason: tf!({ "sighash non calcolabile: {e}" }, {
                        "sighash cannot be computed: {e}"
                    }),
                }
            }
        };

    let message = Message::from_digest(sighash.to_byte_array());
    match secp.verify_ecdsa(&message, &signature.signature, &pubkey.inner) {
        Ok(()) => Outcome::Valid,
        Err(_) => Outcome::Invalid {
            // Most likely cause first: a wrong amount changes the sighash.
            reason: tf!(
                {
                    "firma non valida per questa transazione. Se i byte non sono stati alterati, \
                     l'importo dichiarato per questo input ({value}) non è quello vero: il \
                     sighash di SegWit lo comprende, quindi un valore diverso produce una firma \
                     che non torna"
                },
                {
                    "signature not valid for this transaction. If the bytes were not altered, \
                     the amount declared for this input ({value}) is not the real one: the \
                     SegWit sighash includes it, so a different value gives a signature that \
                     does not match"
                }
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bdk_wallet::bitcoin::absolute::LockTime;
    use bdk_wallet::bitcoin::transaction::Version;
    use bdk_wallet::bitcoin::{Network, Sequence, TxIn, TxOut, Witness};
    use bdk_wallet::{KeychainKind, SignOptions, TxOrdering, Wallet};

    const SEED: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon \
                        abandon abandon about";
    const NETWORK: Network = Network::Regtest;
    const FUNDING: u64 = 200_000;

    fn wallet() -> Wallet {
        let seed = crate::wallet::SeedWords::from_interactive_input(SEED).expect("seed valido");
        crate::wallet::from_seed(&seed, None, NETWORK).expect("wallet derivabile")
    }

    fn funding_tx(wallet: &Wallet, index: u32, nonce: u8) -> Transaction {
        let spk = wallet
            .peek_address(KeychainKind::External, index)
            .address
            .script_pubkey();
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: crate::testing::outpoint(nonce),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(FUNDING),
                script_pubkey: spk,
            }],
        }
    }

    fn signed() -> (Transaction, Vec<Amount>) {
        let mut wallet = wallet();
        let funding = funding_tx(&wallet, 0, 7);
        let _ = wallet
            .reveal_addresses_to(KeychainKind::External, 5)
            .count();
        let _ = wallet
            .reveal_addresses_to(KeychainKind::Internal, 5)
            .count();
        wallet.apply_unconfirmed_txs([(funding.clone(), 0u64)]);

        let recipient = wallet
            .peek_address(KeychainKind::External, 40)
            .address
            .script_pubkey();
        let mut psbt = {
            let mut builder = wallet.build_tx();
            builder
                .ordering(TxOrdering::Untouched)
                .add_recipient(recipient, Amount::from_sat(50_000));
            builder.finish().expect("transazione costruibile")
        };
        assert!(
            wallet
                .sign(&mut psbt, SignOptions::default())
                .expect("firma riuscita"),
            "la PSBT doveva risultare completa"
        );
        let tx = psbt.extract_tx().expect("transazione estraibile");
        let values = tx
            .input
            .iter()
            .map(|i| {
                wallet
                    .get_utxo(i.previous_output)
                    .map(|u| u.txout.value)
                    .expect("input noto al wallet")
            })
            .collect();
        (tx, values)
    }

    #[test]
    fn a_real_signature_verifies() {
        let (tx, values) = signed();
        let checks = check(&tx, &values);
        assert!(!checks.is_empty(), "la transazione ha almeno un input");
        for c in &checks {
            assert!(
                c.is_valid(),
                "input {} non verificato: {:?}",
                c.index,
                c.outcome
            );
        }
    }

    #[test]
    fn a_forged_amount_breaks_the_signature() {
        let (tx, values) = signed();
        let lying: Vec<Amount> = values.iter().map(|v| *v + Amount::from_sat(1)).collect();
        let checks = check(&tx, &lying);
        assert!(
            checks.iter().all(|c| !c.is_valid()),
            "un importo diverso deve far fallire la verifica"
        );
    }

    #[test]
    fn changing_an_output_invalidates_the_signature() {
        let (mut tx, values) = signed();
        tx.output[0].value += Amount::from_sat(1);
        let checks = check(&tx, &values);
        assert!(checks.iter().any(|c| !c.is_valid()));
    }

    #[test]
    fn a_corrupt_signature_does_not_pass() {
        let (tx, values) = signed();
        let mut tx = tx;
        let mut witness = tx.input[0].witness.to_vec();
        witness[0][10] ^= 0xff;
        tx.input[0].witness = Witness::from_slice(&witness);
        let checks = check(&tx, &values);
        assert!(!checks[0].is_valid(), "{:?}", checks[0].outcome);
    }

    #[test]
    fn an_input_without_a_witness_is_invalid() {
        let (mut tx, values) = signed();
        tx.input[0].witness = Witness::new();
        let checks = check(&tx, &values);
        assert!(matches!(checks[0].outcome, Outcome::Invalid { .. }));
    }

    /// Without the amount the sighash cannot be computed, so the input is
    /// reported as unsupported.
    #[test]
    fn without_the_amount_nothing_verifies() {
        let (tx, _) = signed();
        let checks = check(&tx, &[]);
        assert!(matches!(checks[0].outcome, Outcome::Unsupported { .. }));
    }
}
