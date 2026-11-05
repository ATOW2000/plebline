# PlebLine

*[English version](README.md).*

PlebLine serve a spendere dal cold storage di Bitcoin senza aprirlo ogni volta.
In una seduta apri il cold storage, firmi le **ricariche** dei mesi successivi
(pagamenti prefirmati, di solito verso il wallet di tutti i giorni) e le porti
via. Poi le mandi in rete una alla volta, in qualsiasi ordine.

È una finestra per [Tails](https://tails.net). Non custodisce le chiavi al
posto tuo e non sostituisce il tuo wallet. Versione 0.1, provata solo su
signet.

## Come va una cerimonia

1. **Parole.** Scrivi le parole del seed. Il programma non le genera mai; se
   non le hai ancora, spiega come fartele a mano, con un dado o con le carte.
2. **Monete.** Chiede a un server, via Tor o al tuo nodo, quali monete ha il
   wallet. Per questo usa solo la metà pubblica del wallet.
3. **Pagina del wallet.** Ogni moneta con il suo stato, e l'impronta del
   wallet, così parole scritte per il wallet sbagliato si notano subito. Da qui:
   nuove ricariche, dividere le monete, sostituire una ricarica, chiudere e
   cancellare.
4. **Indirizzi.** Gli indirizzi di chi riceve arrivano dal telefono con
   OnionShare, incollati, o letti dall'immagine di un QR. Ognuno ti viene
   mostrato per il confronto prima della conferma.
5. **Firma.** Stacchi la rete. Il programma firma una transazione per ricarica,
   poi le ricontrolla dai byte delle transazioni, senza chiavi e senza rete.
6. **Portarle via.** Ogni ricarica esce come QR a schermo, per il telefono,
   oppure come file su una chiavetta.
7. **Dopo.** Mandi in rete ogni ricarica quando ti serve, da qualunque wallet
   o nodo. Il cold storage resta chiuso.

## Cosa garantisce

- **Due ricariche firmate nella stessa seduta non condividono mai una
  moneta.** Se succedesse, mandarne una annullerebbe l'altra per sempre. Il
  selettore non vede le monete già impegnate, e l'intero gruppo viene
  ricontrollato prima di scrivere qualsiasi file. Ogni apertura riparte da
  zero: prima della prima firma PlebLine ricorda di mandare in rete o
  scartare le ricariche firmate le volte scorse.
- **Ogni uscita va a un destinatario che hai scelto o torna al tuo wallet**,
  controllato sui byte firmati (`crates/plebline-core/src/verify.rs`).
- **Il seed non finisce mai su disco** e si inserisce solo a mano.
- **La rete passa da Tor o dal tuo nodo.** Un server pubblico raggiunto in
  chiaro viene rifiutato prima che parta un solo pacchetto.

Il modello delle minacce completo, e ciò da cui PlebLine di proposito non
protegge, è in [`SECURITY.md`](SECURITY.md).

## Mandare in rete senza PlebLine

La firma lascia un file per ricarica in `~/PlebLine/cassaforte/`: `tx-0.hex` è
la ricarica n. 1, `tx-1.hex` la n. 2, e così via. Alla rete basta l'esadecimale:

```sh
bitcoin-cli sendrawtransaction "$(cat tx-0.hex)"
curl --socks5-hostname 127.0.0.1:9050 -sX POST -d "$(cat tx-0.hex)" \
  https://mempool.space/api/tx    # passando da Tor
```

Accanto, `batch.json` descrive cosa è stato firmato.

## Provarlo

Usa **signet**, dove le monete non valgono niente.
`sh scripts/build-app.sh signet` costruisce il pacchetto di prova in
`dist/PlebLine-prova`; il `LEGGIMI.txt` che c'è dentro spiega come
installarlo su Tails. Ha nome, cartella e voce di menu sue, quindi non tocca
mai il pacchetto vero. Il primo controllo:

```sh
sh ~/.plebline-prova/avvia --version   # deve dire: PlebLine 0.1.0 (signet)
```

Le monete si prendono da [signetfaucet.com](https://signetfaucet.com):
chiedi un pagamento separato per ogni indirizzo, perché una moneta è una
ricarica. La prova che conta è l'ultima: manda la #3, poi la #1, poi la #2,
a distanza di ore o giorni. Devono passare tutte;
`bad-txns-inputs-missingorspent` vorrebbe dire che due ricariche spendevano
la stessa moneta.

## Limiti

- Nessuna revisione da parte di terzi. Scritto con l'aiuto di un'IA (Claude, di
  Anthropic); chi dirige il progetto decide cosa fa e controlla il risultato.
- La commissione si fissa alla firma. Se fra qualche mese non basta, alzarla
  vuol dire riaprire il cold storage; solo chi riceve può sbloccare da sé una
  ricarica ferma.
- Le monete che arrivano direttamente da una coinbase non sono gestite.
- La chiave che firma i pacchetti non ha ancora un'impronta pubblicata.

## Costruirlo

Rust 1.94.1, fissato in `rust-toolchain.toml`.

```sh
cargo test --workspace                           # versione mainnet
cargo test -p plebline-guided --features signet  # il percorso guidato, versione signet
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo tree -p plebline-core --edges normal       # nessun crate di rete
cargo audit --deny warnings
cargo run -p plebline-gui --features signet      # la finestra, su signet
```

La rete si sceglie quando si compila: senza `--features signet` la versione è
per mainnet. Le prove non leggono mai il vero `~/PlebLine`. La CI esegue
tutto questo tranne la finestra, più `scripts/prova-tails-app.sh`
(l'installatore per Tails, senza Tails); a ogni push costruisce il pacchetto
signet e ne pubblica le impronte. `scripts/build-app.sh signet|bitcoin`
costruisce il pacchetto per Tails; [`docs/REPRODUCIBLE.md`](docs/REPRODUCIBLE.md)
spiega come ricostruirlo e confrontarlo.

Per chi rivede il codice: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) dice
dove stanno le cose, [`SECURITY.md`](SECURITY.md) cosa deve valere e cosa è
ancora aperto, [`docs/DECISIONS.md`](docs/DECISIONS.md) perché.

## Licenza

MIT.
