# Names and texts that differ between the two networks. Both `build-app.sh`
# and `installa` read them from here, because two copies could drift apart and
# one of them would then name the wrong network.
#
#     . reti.sh
#     descrivi_rete signet || echo "rete sconosciuta"
#
# Each network has its own directories and menu entry, so installing one does
# not remove the other.

descrivi_rete() {
    case "$1" in
        signet)
            RETE=signet
            CARTELLA=PlebLine-prova
            NASCOSTA=.plebline-prova
            VOCE=plebline-prova.desktop
            NOME="PlebLine (prova su signet)"
            COMMENTO="Prepara più ricariche in una volta sola, offline. Monete di prova."
            MONETE="Sono di signet e non valgono niente, quindi qui un errore non costa nulla. Si chiedono gratis a signetfaucet.com"
            ;;
        bitcoin)
            RETE=bitcoin
            CARTELLA=PlebLine
            NASCOSTA=.plebline
            VOCE=plebline.desktop
            NOME="PlebLine"
            COMMENTO="Prepara più ricariche in una volta sola, offline."
            MONETE="Sono monete vere. Una ricarica mandata in rete non si può annullare, né da te né da nessun altro."
            ;;
        *)
            return 1
            ;;
    esac
}
