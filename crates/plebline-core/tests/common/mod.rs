//! Shared by the integration tests: a regtest wallet and coins whose source
//! transactions are real, validated like any `utxos.json`.

// Each test file is its own crate and uses only part of this.
#![allow(dead_code)]

use plebline_core::bdk_wallet::{KeychainKind, Wallet};
use plebline_core::bitcoin::consensus::encode::serialize_hex;
use plebline_core::bitcoin::hashes::Hash;
use plebline_core::bitcoin::{
    absolute::LockTime, transaction::Version, Amount, Network, OutPoint, ScriptBuf, Sequence,
    Transaction, TxIn, TxOut, Txid, Witness,
};
use plebline_core::utxos::{self, UtxoEntry, UtxoSet, ValidatedUtxoSet};
use plebline_core::wallet::{from_seed, SeedWords};

pub const NETWORK: Network = Network::Regtest;

/// BIP39 test vector; no funds anywhere.
pub const SEED: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

/// Another wallet, for destinations outside the cold storage.
pub const OTHER_SEED: &str =
    "legal winner thank year wave sausage worth useful legal winner thank yellow";

pub fn wallet(seed: &str, passphrase: Option<&str>) -> Wallet {
    let seed = SeedWords::from_interactive_input(seed).expect("seed valido");
    from_seed(&seed, passphrase, NETWORK).expect("wallet derivabile")
}

/// A transaction paying `sats` to receiving address `index`. The input is
/// fake: all the wallet cares about is the output.
pub fn funding_tx(wallet: &Wallet, index: u32, sats: u64, nonce: u8) -> Transaction {
    let spk = wallet
        .peek_address(KeychainKind::External, index)
        .address
        .script_pubkey();
    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            // Not `OutPoint::null()`: that would make it a coinbase.
            previous_output: outpoint(nonce),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(sats),
            script_pubkey: spk,
        }],
    }
}

/// A made-up outpoint: `n` in the first byte of the txid.
pub fn outpoint(n: u8) -> OutPoint {
    let mut bytes = [0u8; 32];
    bytes[0] = n;
    OutPoint::new(Txid::from_byte_array(bytes), 0)
}

/// `utxos.json` for coins `(index, sats, nonce)`, at tip 300.
pub fn utxo_set(wallet: &Wallet, funding: &[(u32, u64, u8)]) -> UtxoSet {
    let utxos = funding
        .iter()
        .map(|(index, sats, nonce)| {
            let tx = funding_tx(wallet, *index, *sats, *nonce);
            UtxoEntry {
                outpoint: format!("{}:0", tx.compute_txid()),
                value_sat: *sats,
                script_pubkey: tx.output[0].script_pubkey.to_hex_string(),
                keychain: "external".into(),
                derivation_index: *index,
                confirmations: 144,
                prev_tx_hex: serialize_hex(&tx),
            }
        })
        .collect();

    UtxoSet {
        version: 1,
        network: NETWORK.to_string(),
        generated_at: 1_700_000_000,
        tip_height: 300,
        descriptor_checksum: Some(wallet.descriptor_checksum(KeychainKind::External)),
        last_active_indices: [("external".to_string(), 10)].into_iter().collect(),
        utxos,
    }
}

/// The wallet holding those coins, through the same validation as a file.
pub fn hydrated(mut wallet: Wallet, funding: &[(u32, u64, u8)]) -> (Wallet, ValidatedUtxoSet) {
    let set = utxos::validate(utxo_set(&wallet, funding), NETWORK, "utxos.json")
        .expect("il set sintetico è coerente");
    set.hydrate(&mut wallet).expect("idratazione riuscita");
    (wallet, set)
}
