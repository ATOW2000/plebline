//! `nLockTime` is always zero. BDK would otherwise write the known tip
//! height (anti fee sniping), marking payments broadcast months later as
//! presigned. These tests give the wallet a tip, so the zero must come from
//! the code.

use plebline_core::bdk_wallet::chain::BlockId;
use plebline_core::bdk_wallet::{KeychainKind, Update, Wallet};
use plebline_core::bitcoin::hashes::Hash;
use plebline_core::bitcoin::{absolute::LockTime, BlockHash};
use plebline_core::plan::{self, BatchConfig, BatchEntry};
use plebline_core::split::{self, SplitConfig};
use plebline_core::utxos;

mod common;
use common::NETWORK;

const TIP: u32 = 800_000;

/// A wallet with three coins that believes the chain is at [`TIP`].
fn wallet_with_a_tip() -> Wallet {
    let mut wallet = common::wallet(common::SEED, None);

    let funding: Vec<(u32, u64, u8)> = (0..3u32).map(|i| (i, 500_000, i as u8 + 1)).collect();
    let mut set = common::utxo_set(&wallet, &funding);
    set.tip_height = TIP;
    utxos::validate(set, NETWORK, "utxos.json")
        .expect("set valido")
        .hydrate(&mut wallet)
        .expect("idratazione riuscita");

    let tip = wallet
        .latest_checkpoint()
        .push(BlockId {
            height: TIP,
            hash: BlockHash::from_byte_array([7; 32]),
        })
        .expect("altezza sopra la genesi");
    wallet
        .apply_update(Update {
            chain: Some(tip),
            ..Default::default()
        })
        .expect("punta collegabile");
    assert_eq!(wallet.latest_checkpoint().height(), TIP);
    wallet
}

fn destination(index: u32) -> String {
    common::wallet("zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong", None)
        .peek_address(KeychainKind::External, index)
        .address
        .to_string()
}

#[test]
fn every_payment_of_the_batch_has_zero_locktime() {
    let mut wallet = wallet_with_a_tip();
    let config = BatchConfig {
        network: NETWORK.to_string(),
        fee_rate_sat_vb: 2.0,
        entries: (0..3)
            .map(|i| BatchEntry {
                recipient: destination(i),
                amount_sat: 100_000,
                label: None,
            })
            .collect(),
    };

    let built = plan::build(&mut wallet, &config, []).expect("batch costruibile");

    assert_eq!(built.len(), 3);
    for tx in &built {
        assert_eq!(
            tx.psbt.unsigned_tx.lock_time,
            LockTime::ZERO,
            "transazione #{}: nLockTime diverso da zero",
            tx.index
        );
    }
}

#[test]
fn the_split_has_zero_locktime() {
    let mut wallet = wallet_with_a_tip();
    let config = SplitConfig {
        network: NETWORK.to_string(),
        outputs: 5,
        fee_rate_sat_vb: 2.0,
    };

    let plan = split::build(&mut wallet, &config, []).expect("divisione costruibile");

    assert_eq!(plan.psbt.unsigned_tx.lock_time, LockTime::ZERO);
}
