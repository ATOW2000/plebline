# Security

PlebLine is at version 0.1 and has not been reviewed by a third party. What
is still open is at the end of this file.

## Reporting a vulnerability

Use GitHub's private vulnerability reporting (the "Security" tab, "Report a
vulnerability"), not a public issue, for anything that could cost a user
money or privacy. Say which commit or package, which network, and the steps
or the file that reproduce it.

## Threat model

**The user** holds a single-signature BIP84 cold storage (BIP39 words,
optionally a passphrase), is not necessarily technical, and opens it once
every few months to presign the next refills.

**The machine** is Tails: amnesic, home in RAM, and a system firewall that
lets nothing out except through Tor. The program speaks SOCKS5 to Tor
itself (DNS included) or connects directly to a node on the user's own
network; anything else is refused before a packet leaves
(`plebline::tor::route`). Unplugging the network before signing is an
instruction the user confirms, not something the program can check.

**Assets**: the seed and passphrase; the signed refills, which are bearer
instruments (anyone holding one can broadcast it, to the recipient and
amount fixed at signing); the recipient addresses (a wrong one is a valid
payment to a stranger); and the wallet's public files and chain queries,
which reveal every address, the balance and the history.

**Attackers considered**: the Esplora server or the network path (lies about
coins, confirmations and broadcasts; stalls); hostile files from the disk, a
USB stick or the phone; whoever takes the stick or phone holding the
refills; another process of the same user; a modified package; the user's
own mistakes (another wallet's words, a wrong address or network, a low fee,
two ceremonies at once, deleting too early).

**Not considered**: a compromised operating system, firmware or hardware on
the signing machine, a compromised Tor, a compromised BDK or Rust toolchain.

## Invariants

1. **No two presigned refills share a coin**, within a batch and within a
   session: broadcasting one would invalidate the other forever. The coin
   selector never sees a committed coin (`plebline_core::coin_select`); the
   same coins are marked unspendable; the inputs each transaction actually
   spends are committed after building; `plan::check_disjoint` checks the
   whole batch again. Committed coins are read from the signed bytes of the
   signed files themselves, never from a separate registry
   (`wizard::committed_outpoints`). **Between sessions this does not hold, by
   choice** ([`docs/DECISIONS.md`](docs/DECISIONS.md)): every opening sets
   earlier signed files aside unread, and before the first signature the
   program reminds the user to check that the old refills have all been sent.
2. **Every output goes to an intended recipient or back to the wallet.**
   `plebline_core::verify` recomputes everything from the transaction bytes,
   with no keys and no network: txids, inputs, BIP143 signatures (P2WPKH and
   SIGHASH_ALL only), amounts, fees, addresses and disjointness. With the
   public descriptors and the refills' definition, every output must be an
   expected recipient or derive from the wallet; an output marked as change
   must always derive from the wallet. It runs in full before any signed
   file is written. Before a refill is sent it runs again on what cannot
   have changed since signing (signatures, disjointness, change), without
   the definition and the coin list, which a new round or a fresh read of
   the chain legitimately replace.
3. **`nLockTime` is always 0.** BDK writes the tip height against fee
   sniping, but a height months old at broadcast would mark the refill as
   presigned, and the same height on a whole group would link the refills;
   the right height cannot be known months earlier. Zero is also what many
   wallets write.
4. **The program never generates seed words, and the seed is never written
   to disk.** It is entered only interactively. The words are zeroed on
   drop; the keys BDK derives from them are not. A test searches the JSON
   files built from the wallet (`wallet`, `utxos`, `plan`, `batch` with its
   PSBTs, `split`) for the words and for private keys
   (`plebline-core/tests/no_secrets_on_disk.rs`).
5. **Every irreversible confirmation defaults to no**: signing, broadcasting,
   deleting signed files, withdrawing a refill, accepting the phone's address
   list. On mainnet sending a refill also requires typing the amount; a
   split, where only the fee leaves the wallet, asks yes or no.
6. **Signed files are deleted only after the user says yes.** Refills can be
   sent whenever one likes, so some are normally still unsent at the end of
   a ceremony: "Close and delete" names the files not taken away and asks.
   Opening a new session only sets them aside, and a network error never
   counts as "sent" or "never sent".

## Other protections

- **Process**: no core dumps; on Linux the process is non-dumpable, so other
  processes of the same user cannot read its memory (`plebline::harden`).
- **Hostile input**: every file reader returns an error instead of
  panicking, and refuses impossible amounts, overflowing sums, duplicate
  refill numbers and derivation indexes above 100 000
  (`plebline-core/tests/hostile_files.rs`). QR images are decoded in pure
  Rust with size caps; phone messages are read as text only, and one bad
  line rejects the whole list.
- **Addresses**: every source goes through one parser
  (`plebline_core::address::parse`) that checks the network; each address is
  shown back in groups of four characters before it is accepted.
- **Network**: no client can be built without the permit the Tor rule
  returns; how the rule decides is in
  [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md#connectors).

## Signed packages

`installa` copies the Tails package into a private directory and checks
every file against `IMPRONTE` before installing anything: a missing list, a
file that does not match, or a file not on the list stops it. If the list is
signed, it verifies the signature and shows the signing key's fingerprint,
to be compared with one published here; the default answer is no. The
mainnet package installs only if signed. **The project's signing key has no
published fingerprint yet.** Builds are reproducible on one machine; see
[`docs/REPRODUCIBLE.md`](docs/REPRODUCIBLE.md).

## Open findings and accepted limits

| severity | finding |
|---|---|
| High | The phone is both the source of the recipient addresses and the place they are checked: a compromised phone can supply wrong addresses that pass every check. OnionShare in public mode accepts a list from anyone with the address; a single message is taken without asking which; a BIP 21 amount is pre-filled (both kept, see `docs/DECISIONS.md`). |
| High | Refills are bearer instruments and usually sit on one phone or stick: whoever takes it can broadcast them all. The only way to cancel one is to spend its coin first, which needs the cold storage. By design. |
| High | Receive addresses written to the stick for later deposits can be rewritten, with nothing to compare them against. |
| High | On Tails with persistence, the installed program is not re-verified at each start. |
| Medium | One Esplora server at a time: it can hide coins, misreport confirmations or refuse a broadcast, and it learns every address of the wallet. It cannot misstate a real coin's value, which is bound to its source transaction. A node of one's own removes this third party. |
| Medium | A tampered mainnet package can declare itself signet and install unsigned with a warning; an older package signed with the same key installs; revocations are not seen. |
| Medium | `verify` does not compare amounts with the batch definition. |
| Medium | Anything answering on port 9050 is taken for Tor; the `check.torproject.org` proof is shown, not required. Matters where no firewall forces Tor. |
| Medium | The seed is typed with the network still up, to rebuild the wallet for the scan, and the keys BDK derives stay in RAM for the session; egui's undo history may keep the words. The protection is an amnesic machine. |
| Medium | Two copies open at once can put one coin in two refills. The Tails launcher refuses a second copy; started by hand, nothing stops it. |
| Medium | The USB stick folder is fixed: the next ceremony overwrites `batch.json` and same-numbered `tx-K.hex` there. |
| Medium | With Dotfiles persistence, an atomic write replaces the persistent link with a file in RAM. |
| Medium | After an Esc at "plug back in", the session may still believe the network is unplugged. |
| Low | Destinations anyone can spend (witness v2+, P2A) are accepted; a `utxos.json` dated in the future stays fresh; offline change can reuse addresses; `broadcast` shows the JSON, not the bytes; `socks5`/`socks4` proxies (local DNS) are accepted; a home node given by name is looked up by the system resolver, outside Tor (on Tails the firewall stops the lookup); status checks reveal unsent refills to the server; the passphrase is not asked twice; receive-address QR images are not `0600`; in a split, change below dust goes to fee. |

Out of scope: multisig, hardware wallets, Lightning, timelocks, complex
descriptors.

## What review would help most

1. **Disjointness within a session**: two signed refills sharing a coin, by
   any route.
2. **What `verify` proves** to someone holding only `batch.json`, and to the
   user holding the definition, the coin list and `wallet.json`.
3. **The network gate**: `tor::route` and the client, DNS leaks, anything
   that reaches a clearnet host.
4. **Secrets in memory**: the seed from entry to drop, across the window's
   thread boundary, and what BDK keeps.
5. **Hostile input**: the JSON readers, the OnionShare inbox, QR decoding,
   in the process that holds the seed.
6. **Supply chain and packaging**: dependencies, reproducible builds, the
   Tails install and start path.
7. **Wording and defaults** where a hurried user could do the wrong thing.
