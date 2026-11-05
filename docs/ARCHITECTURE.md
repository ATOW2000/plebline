# Architecture

PlebLine is a cold-storage ceremony tool: in one sitting the user signs N
refills that share no coin, and broadcasts them one at a time over the
following months. It runs on Tails, as a window. The guarantees are listed
in [`SECURITY.md`](../SECURITY.md); this document says where things are.

For funds safety, start with `coin_select`, `plan`, `batch`, `verify` and
`utxos` in the core, then `plebline::wizard`, `project` and `replace`, then
the network rule in `tor` and `chain`.

## Crates

```
plebline-gui ──► plebline-guided ──► plebline ──────► plebline-core ──► bdk_wallet
egui window      the guided path     ceremony steps,  formats, coin
                 (run, home)         connectors       selection, plan,
                                     (network code    sign, verify, split
                                     in chain.rs)
```

| crate | holds |
|---|---|
| `plebline-core` | File formats, disjoint coin selection, planning, signing, verification, split. No UI, `#![forbid(unsafe_code)]`, no network crate in its tree (CI checks it). |
| `plebline` | The ceremony steps (`wizard`, `online`, `replace`, `project`) and the connectors (`ask`, `chain`, `out`, `tor`). |
| `plebline-guided` | The single guided path. The window runs it; the tests run it with scripted answers. |
| `plebline-gui` | The window (`egui`/`eframe`, OpenGL). No ceremony logic: it answers questions and draws lines. |

### `plebline-core`

| module | role |
|---|---|
| `coin_select` | `DisjointCoinSelection`, a BDK `CoinSelectionAlgorithm` that never sees a committed coin. The order is total (value, then outpoint), so plan and sign pick the same coins. |
| `plan` | `build`: one transaction per refill, each selector seeded with everything committed so far; `unspendable`, `utxos::refuse_unlisted`, the inputs actually spent committed after `finish()`, `check_disjoint`; `nLockTime` 0; `predicted_vsize`. |
| `batch` | `SignedBatch` (`batch.json`); `sign` with `SignOptions::default()` (no `trust_witness_utxo`), the txid unchanged by signing; `shown_address`, `number` (shown numbers start at 1, file indexes at 0). |
| `verify` | Keyless, networkless re-check of a signed batch, everything read from its bytes; a signature it cannot check is a failure. |
| `signatures` | BIP143 sighash and ECDSA verification for P2WPKH, SIGHASH_ALL only. |
| `split` | One self-transfer that turns the free coins into more coins: committed and locked coins excluded, every output the wallet's. |
| `utxos` | `utxos.json`, validated against each coin's source transaction; `hydrate` loads it into an offline wallet and locks the outputs the file does not list. |
| `watch`, `wallet` | `wallet.json` (public descriptors, one account only); `SeedWords`, zeroed on drop; the BIP84 wallet. |
| `address` | Recipient parsing (address or BIP 21 URI), network-checked. |
| `error`, `lang` | Errors with a printable remedy; bilingual strings, `t("it", "en")` and `tf!`. |

### `plebline`

| module | role |
|---|---|
| `wizard` | Signing, splitting and verification flows (verify, then archive, then write); commitments (`committed_outpoints`, `coins_spent`); atomic writes and archiving. |
| `online` | The chain scan (`scan`, `export_utxos`), `broadcast` as `prepare`, `confirm`, `send`, the default server list. No private key reachable from here. |
| `project` | The ceremony folder and its fixed file names; which signed files commit coins (`signed_files`, `all_signed_files`, `set_aside`); `wipe`. |
| `replace` | Withdrawing one never-sent refill to free its coin. |
| `inbox`, `destination`, `qr` | Addresses from OnionShare, typed input, URIs and QR images: with `plebline_core::address`, everything that comes from outside the process. |
| `harden` | No core dumps; on Linux, a non-dumpable process. |
| `ask`, `chain`, `out`, `tor` | The connectors, below. |

The code also builds on macOS, for development; where the Mac differs (where
OnionShare saves, the Tor remedy) is marked `cfg!(target_os = "macos")`.

## Connectors

They keep the ceremony apart from the keyboard, the network and the screen,
so the whole guided path runs in tests with prepared answers, a fake chain
and a recorded screen.

- **`ask::Ask`**, passed as `&mut dyn Ask`, carries every question. The
  window's `bridge::Window` answers it; `Scripted` answers in tests. Choices
  return an index, never the text, and an address is its own question type.
- **`chain::Chain`** carries every network request: `reach` (tip height, before
  the words), `scan`, `tx_status` (status of a txid), `send`. `Esplora` is
  the real server, `Fake` a prepared one for tests. `chain.rs` is the only
  file that names the network libraries.
- **`out`** is the screen, with one sink per thread: the window's ceremony
  thread installs its own.
  `out::Place` tells the window where the ceremony stands; control characters
  from files are shown as text.
- **`tor::route`** is the network rule, in one function: an own node
  (loopback, private addresses, `localhost`, `.local`, `.lan`, `.home`,
  `.internal`, or a name without dots, which must resolve only to local
  addresses and is reached at those) is reached directly; anything else
  through Tor (`socks5h`, DNS inside Tor) if Tor answers, or refused before
  any packet. The host is read with the parser the HTTP client connects
  with. `chain::Esplora` cannot build a client without the `Permit` that
  only the rule makes; each client gets its own Tor circuit, and the route is
  proved on screen before the words are typed.

## The ceremony

`plebline_guided::run` uses `~/PlebLine/cassaforte` and calls `run_in`, which
holds the order up to the wallet page; the page loop is `home::run`.
`home::Session` is the only place that changes what the program believes
about the network and the words.

```
run_in
  language → "do you already have your words?" (no → how to make them by hand)
  → network check (server choice, Chain::reach), before the words
  → words and passphrase (BIP39 checksum checked at once)
      → wallet.json rewritten from the words
      → signed files of earlier sessions set aside in precedenti/<n>/, unread
      → chain scan → utxos.json
home::run (until "Close and delete")
  coins with their state, wallet fingerprint
  ├─ New refills: addresses from the phone → how many → unplug → sign
  │               → verify → take away (QR to the phone, or a USB stick)
  ├─ Split the coins: unplug → sign → plug back → broadcast the split
  ├─ Replace a refill: the chain says it was never sent → withdraw it
  ├─ Send a signed refill: verify → plug back → broadcast
  ├─ Read the chain again
  └─ Close and delete (asks, default no, if a signed file was not taken away)
```

`wizard::sign`: validate `utxos.json` → read commitments → open the wallet
from the words and check it against `utxos.json` → `hydrate` → `plan::build`
→ show every transaction → confirm (default no) → `batch::sign` → `verify`
→ only if it passes, archive the previous `batch.json` and write the new one
and one `tx-K.hex` per refill.

**The window.** `plebline-gui` runs the ceremony on its own thread, with
`bridge::ToWindow` as its screen and `bridge::Window` as its `Ask`. Questions
cross a channel; the drawing thread never blocks. The words cross to the
ceremony thread by move and stay there; the window's text fields are zeroed
when they are handed over, and on cancel and close. The window keeps nothing
on disk and has no accessibility bridge.

## Files on disk

All in `~/PlebLine/cassaforte/`, under fixed names, `0600` (folders `0700`),
written atomically (temporary file, sync, rename): a crash leaves the old
file or the new one, never half a signed batch whose coins would then look
free.

| file | content |
|---|---|
| `wallet.json` | Public descriptors and their checksum, the wallet's name. Watch-only, but it reveals every address. |
| `utxos.json` | Every coin with its full source transaction, the chain height and the wallet's name. |
| `batch-config.json` | Recipients, amounts and fee rate of the refills to sign. |
| `plan.json` | The transactions as built, before signing. |
| `batch.json` | The signed refills: hex, PSBT, inputs with values, outputs with address and change flag. |
| `split.json` | The split, as a one-transaction batch. |
| `tx-K.hex` | Refill `K` (from 0), ready to broadcast from anywhere. |
| `batch-N.json`, `split-N.json`, `tx-K-N.hex` | Earlier versions in the same session, archived with a hard link, never overwritten. |

Every reader treats these files as hostile. `utxos.json` is re-derived from
each source transaction; `wallet.json` must match its own checksum, and its
change descriptor must be the receive descriptor's sibling.

Committed coins are read by `wizard::committed_outpoints` from the signed
bytes of the files `project::all_signed_files` lists. Why there is no
registry, why they last one session, and why the network (`signet` feature)
is fixed at compile time: [`DECISIONS.md`](DECISIONS.md).

## Tests

Integration tests of the core are in `crates/plebline-core/tests/`; unit
tests sit next to the code. End-to-end tests of the guided path run `run_in`
in a temporary folder with `Scripted`, `chain::Fake` and `out::Recorder`
(the `test-support` feature), using coins whose source transactions really
pay the test wallet, so validation, signing and verification are the real
ones. Each test thread has its own home and language.

## Languages

Code, comments and test names are in English; what people read is bilingual
(`t("it", "en")`). Test assertions and the Tails installer are in Italian.
