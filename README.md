# PlebLine

*[Versione italiana](README.it.md).*

PlebLine lets you spend from Bitcoin cold storage without opening it every
time. In one sitting you open the cold storage, sign the next few months of
**refills** (presigned payments, usually to your everyday wallet) and take them
away. Later you broadcast them one at a time, in any order.

It is a window for [Tails](https://tails.net). It holds no keys for you and
does not replace your wallet. Version 0.1, tested on signet only.

## How a ceremony goes

1. **Words.** You type your seed words. The program never generates them; if
   you have none yet, it explains how to make them by hand with dice or cards.
2. **Coins.** It asks a server, through Tor or your own node, which coins the
   wallet holds. Only the public half of the wallet is used for this.
3. **Wallet page.** Every coin with its state, and the wallet's fingerprint, so
   words typed for the wrong wallet are noticed at once. From here: new
   refills, split the coins, replace a refill, close and delete.
4. **Addresses.** The recipients' addresses arrive from your phone through
   OnionShare, pasted, or read from a QR code image. Each one is shown back for
   comparison before you confirm.
5. **Signing.** You unplug the network. The program signs one transaction per
   refill, then checks them again from the transaction bytes, with no keys and
   no network.
6. **Taking them away.** Each refill leaves as a QR code on screen, for your
   phone, or as a file on a USB stick.
7. **Later.** You broadcast each refill when you need it, from any wallet or
   node. The cold storage stays closed.

## What it guarantees

- **No two refills signed in the same session share a coin.** If two did,
  broadcasting one would cancel the other for good. The coin selector never
  sees a coin already committed, and the whole batch is checked again before
  anything is written. Each opening starts from zero: before the first
  signature PlebLine reminds you to broadcast or discard the refills you
  signed in earlier sessions.
- **Every output goes to a recipient you chose or back to your wallet**,
  checked on the signed bytes (`crates/plebline-core/src/verify.rs`).
- **The seed is never written to disk** and is entered only interactively.
- **The network goes through Tor or to your own node.** A public server reached
  in the clear is refused before any packet leaves.

The full threat model, and what PlebLine deliberately does not protect
against, is in [`SECURITY.md`](SECURITY.md).

## Broadcasting without PlebLine

Signing leaves one file per refill in `~/PlebLine/cassaforte/`: `tx-0.hex` is
refill #1, `tx-1.hex` refill #2, and so on. The hex is all the network needs:

```sh
bitcoin-cli sendrawtransaction "$(cat tx-0.hex)"
curl --socks5-hostname 127.0.0.1:9050 -sX POST -d "$(cat tx-0.hex)" \
  https://mempool.space/api/tx    # through Tor
```

`batch.json`, next to them, describes what was signed.

## Trying it

Use **signet**, where coins are worth nothing.
`sh scripts/build-app.sh signet` builds the test package in
`dist/PlebLine-prova`; the `LEGGIMI.txt` inside it (Italian) explains how to
install it on Tails. It has its own name, folder and menu entry, so it never
touches the real package. First check:

```sh
sh ~/.plebline-prova/avvia --version   # must say: PlebLine 0.1.0 (signet)
```

Coins come from [signetfaucet.com](https://signetfaucet.com): ask for a
separate payment to each address, since one coin is one refill. The test
that matters is the last: broadcast #3, then #1, then #2, hours or days
apart. All must go through; `bad-txns-inputs-missingorspent` would mean two
refills spent the same coin.

## Limits

- Not reviewed by a third party. Written with the help of an AI (Claude, by
  Anthropic); the maintainer decides what it does and checks the result.
- The fee is fixed at signing. If it is too low months later, raising it means
  opening the cold storage again; only the recipient can speed up a stuck
  refill on their own.
- Coins coming straight from a coinbase are not supported.
- The key that signs the packages has no published fingerprint yet.

## Building

Rust 1.94.1, pinned in `rust-toolchain.toml`.

```sh
cargo test --workspace                           # mainnet build
cargo test -p plebline-guided --features signet  # the guided path, signet build
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo tree -p plebline-core --edges normal       # must contain no network crate
cargo audit --deny warnings
cargo run -p plebline-gui --features signet      # the window, on signet
```

The network is chosen at compile time: without `--features signet` the build
is for mainnet. Tests never read the real `~/PlebLine`. CI runs all of the
above except the window, plus `scripts/prova-tails-app.sh` (the Tails
installer, without Tails); on every push it builds the signet package and
publishes its hashes. `scripts/build-app.sh signet|bitcoin` builds the Tails
package; [`docs/REPRODUCIBLE.md`](docs/REPRODUCIBLE.md) explains how to
rebuild it and compare.

For reviewers: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) says where
things are, [`SECURITY.md`](SECURITY.md) what must hold and what is still
open, [`docs/DECISIONS.md`](docs/DECISIONS.md) why.

## License

MIT.
