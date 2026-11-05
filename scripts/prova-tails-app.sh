#!/usr/bin/env sh
# Tests the Tails installer and launcher without Tails.
#
#     sh scripts/prova-tails-app.sh
#
# Covers where files land, the menu entry, which network reaches the program,
# a package with no network or with a file missing, and installing over an
# earlier version: after a reboot (home files are links into the Dotfiles),
# with a working copy that cannot be rewritten, and with the old version still
# open. Then the fingerprints: missing, wrong, malformed, incomplete, a file
# they do not name, and a stick whose content changes after the check. Then
# the signature, with throwaway keys: confirmed, declined, bad, missing, made
# by another key, in the terminal and in a fake dialog box. The binaries are
# fake and record their arguments.
#
# Left for a test on Tails itself: that a binary linked from the Dotfiles
# runs, that the window opens, that Linux reports "Text file busy" where this
# test uses a write-protected file, and that the menu picks up a new entry
# without a reboot.

set -eu

QUI=$(cd "$(dirname "$0")/.." && pwd)
# A template, or macOS ignores $TMPDIR.
BANCO=$(mktemp -d "${TMPDIR:-/tmp}/prova-tails.XXXXXX")
CHIAVI="$BANCO/chiavi"

# Also stops any gpg-agent started for a throwaway key.
pulisci_banco() {
    if command -v gpgconf >/dev/null 2>&1; then
        for casa in "$CHIAVI"/*/; do
            [ -d "$casa" ] && GNUPGHOME="$casa" gpgconf --kill all >/dev/null 2>&1 || true
        done
    fi
    rm -rf "$BANCO"
}
trap pulisci_banco EXIT

FALLITE=0
SALTATE=0

# `salta N ETICHETTA`: counts N checks that cannot run here and prints them as
# skipped.
salta() {
    printf '  --   %s (saltata)\n' "$2"
    SALTATE=$((SALTATE + $1))
}

esito() {
    if [ "$1" = 0 ]; then
        printf '  ok   %s\n' "$2"
    else
        printf '  NO   %s\n' "$2"
        FALLITE=$((FALLITE + 1))
    fi
}

# `contiene FILE TESTO ETICHETTA`
contiene() {
    if [ -f "$1" ] && grep -qF -- "$2" "$1"; then esito 0 "$3"; else esito 1 "$3"; fi
}

non_contiene() {
    if [ -f "$1" ] && grep -qF -- "$2" "$1"; then esito 1 "$3"; else esito 0 "$3"; fi
}

esiste() {
    if [ -e "$1" ]; then esito 0 "$2"; else esito 1 "$2"; fi
}

non_esiste() {
    if [ -e "$1" ]; then esito 1 "$2"; else esito 0 "$2"; fi
}

# A real file, not a link.
vero_file() {
    if [ -f "$1" ] && [ ! -L "$1" ]; then esito 0 "$2"; else esito 1 "$2"; fi
}

# Writes IMPRONTE with the same script `build-app.sh` uses.
impronte() { sh "$QUI/scripts/impronte.sh" "$1"; }

# A package shaped like `build-app.sh`'s, with fake binaries. The third
# argument is a version written inside every file, to tell old from new.
pacchetto() {
    rete=$1
    dove=$2
    versione=${3:-1}
    mkdir -p "$dove/programma" "$dove/senza-terminale"
    cp "$QUI/scripts/tails-app/installa" "$dove/installa"
    printf 'leggimi finto, versione %s\n' "$versione" > "$dove/LEGGIMI.txt"
    printf '[Desktop Entry]\n' > "$dove/senza-terminale/voce.desktop"
    cp "$QUI/scripts/tails-app/avvia" "$dove/programma/avvia"
    cp "$QUI/scripts/tails-app/voce.desktop" "$dove/programma/voce.desktop"
    cp "$QUI/scripts/tails-app/reti.sh" "$dove/programma/reti.sh"
    printf '%s\n' "$rete" > "$dove/programma/RETE"
    printf 'png finto, versione %s\n' "$versione" > "$dove/programma/plebline.png"
    cat > "$dove/programma/plebline-gui" <<FINE
#!/bin/sh
# versione $versione
printf 'plebline-gui %s\n' "\$*" >> "\$HOME/eseguito"
echo "stampato da plebline-gui"
FINE
    chmod +x "$dove/programma/plebline-gui"
    chmod +x "$dove/installa" "$dove/programma/avvia"
    impronte "$dove"
}

# An empty home, and a Dotfiles directory only when asked for.
casa_nuova() {
    nome=$1
    CASA_FINTA="$BANCO/$nome"
    rm -rf "$CASA_FINTA"
    mkdir -p "$CASA_FINTA"
    DOTFILES_FINTI="$BANCO/$nome-dotfiles"
    rm -rf "$DOTFILES_FINTI"
}

dotfiles_accesi() { mkdir -p "$DOTFILES_FINTI"; }

# What Tails does at boot: the home starts empty (it is RAM), then every file
# in the Dotfiles comes back as a link, its directories as real ones.
riavvio() {
    rm -rf "$CASA_FINTA"
    mkdir -p "$CASA_FINTA"
    ( cd "$DOTFILES_FINTI" && find . -type f ) | while IFS= read -r f; do
        f=${f#./}
        mkdir -p "$CASA_FINTA/$(dirname "$f")"
        ln -s "$DOTFILES_FINTI/$f" "$CASA_FINTA/$f"
    done
}

# Leftovers of a copy made beside the file and renamed over it.
nessun_avanzo() {
    if [ -z "$(find "$@" -name '*.nuovo' 2>/dev/null)" ]; then
        esito 0 "nessuna copia a metà lasciata in giro"
    else
        esito 1 "nessuna copia a metà lasciata in giro"
    fi
}

# Stdin is empty, so every question gets no answer, which counts as no.
installa() {
    HOME="$CASA_FINTA" PLEBLINE_DOTFILES="$DOTFILES_FINTI" \
        sh "$1/installa" </dev/null >"$BANCO/uscita" 2>&1
}

# `installa_dicendo RISPOSTA DIR`: RISPOSTA is what gets typed at the question.
installa_dicendo() {
    printf '%s\n' "$1" | HOME="$CASA_FINTA" PLEBLINE_DOTFILES="$DOTFILES_FINTI" \
        sh "$2/installa" >"$BANCO/uscita" 2>&1
}

# Nothing of either network in the home.
niente_installato() {
    if [ -e "$CASA_FINTA/.plebline" ] || [ -e "$CASA_FINTA/.plebline-prova" ]; then
        esito 1 "$1"
    else
        esito 0 "$1"
    fi
}

# `rifiuta DIR TESTO ETICHETTA [RISPOSTA]`: stops, says why, installs
# nothing. In a home of its own, so one wrong install does not spill into the
# next case.
rifiuta() {
    casa_nuova rifiuto
    if [ $# -ge 4 ]; then
        installa_dicendo "$4" "$1" && fermato=no || fermato=si
    else
        installa "$1" && fermato=no || fermato=si
    fi
    if [ "$fermato" = si ]; then contiene "$BANCO/uscita" "$2" "$3"; else esito 1 "$3"; fi
    niente_installato "    e non installa niente"
}

# ------------------------------------------- throwaway keys, never the real one
#
# Made and checked with the real gpg, as `installa` does.
FIRMA_CON=gpg

# `nuova_chiave NOME`: its fingerprint goes in $CHIAVI/NOME.fpr.
nuova_chiave() {
    case "$FIRMA_CON" in
        gpg)
            mkdir -m 700 "$CHIAVI/$1" || return 1
            GNUPGHOME="$CHIAVI/$1" gpg --batch --pinentry-mode loopback --passphrase '' \
                --quick-gen-key "Prova $1 <$1@prova.invalid>" ed25519 sign 1d >/dev/null 2>&1 \
                || return 1
            GNUPGHOME="$CHIAVI/$1" gpg --batch --with-colons --list-keys 2>/dev/null \
                | awk -F: '$1 == "fpr" { print $10; exit }' > "$CHIAVI/$1.fpr"
            ;;
        *) return 1 ;;
    esac
    [ -s "$CHIAVI/$1.fpr" ]
}

# `firma PACCHETTO NOME`: with gpg, through firma-pacchetto.sh, the way a real
# package is signed.
firma() {
    case "$FIRMA_CON" in
        gpg)
            GNUPGHOME="$CHIAVI/$2" sh "$QUI/scripts/firma-pacchetto.sh" "$1" \
                "$(cat "$CHIAVI/$2.fpr")" >"$BANCO/firmato" 2>&1
            ;;
        *) return 1 ;;
    esac
}

# As `installa` shows it: ten groups of four.
a_gruppi() { sed 's/..../& /g; s/ $//' < "$CHIAVI/$1.fpr"; }

mkdir -m 700 "$CHIAVI"
if ! command -v gpg >/dev/null 2>&1; then
    FIRMA_CON=niente
elif ! nuova_chiave vera; then
    FIRMA_CON=niente
fi
[ "$FIRMA_CON" = niente ] || nuova_chiave altra || FIRMA_CON=niente
case "$FIRMA_CON" in
    gpg)    echo "     (le firme di prova le fa gpg, con firma-pacchetto.sh)" ;;
    *)      echo "     (gpg qui non riesce a firmare: le prove sulle firme saltano)" ;;
esac

# ------------------------------------------------------ 1. the test package
echo
echo "1. il pacchetto di prova, con l'archiviazione persistente accesa"
pacchetto signet "$BANCO/prova"
casa_nuova casa1
dotfiles_accesi
installa "$BANCO/prova"

esiste "$CASA_FINTA/.plebline-prova/plebline-gui" "la finestra è in casa"
esiste "$CASA_FINTA/.plebline-prova/RETE" "la rete dichiarata sta accanto ai programmi"
VOCE1="$CASA_FINTA/.local/share/applications/plebline-prova.desktop"
contiene "$VOCE1" "Name=PlebLine (prova su signet)" "la voce di menu dice che è la prova"
contiene "$VOCE1" "Exec=sh $CASA_FINTA/.plebline-prova/avvia" "la voce di menu apre la finestra"
contiene "$VOCE1" "Terminal=false" "la voce di menu non apre un terminale"
esiste "$DOTFILES_FINTI/.plebline-prova/plebline-gui" "la finestra è anche nell'archiviazione"
esiste "$DOTFILES_FINTI/.local/share/applications/plebline-prova.desktop" \
    "la voce di menu è anche nell'archiviazione"
contiene "$BANCO/uscita" "ci sarà a ogni avvio" "lo dice a chi ha installato"

# ----------------------------------------------- 2. with no persistence
echo
echo "2. lo stesso, senza archiviazione persistente"
casa_nuova casa2
installa "$BANCO/prova"
esiste "$CASA_FINTA/.plebline-prova/avvia" "il programma c'è lo stesso"
contiene "$BANCO/uscita" "Allo spegnimento sparirà" "e avvisa che sparirà allo spegnimento"

# ---------------------------------------------- 3. the two networks coexist
echo
echo "3. il pacchetto vero, accanto a quello di prova"
pacchetto bitcoin "$BANCO/vero"
casa_nuova casa3
dotfiles_accesi
installa "$BANCO/prova"
# The mainnet package installs only when signed and with the fingerprint
# confirmed.
if firma "$BANCO/vero" vera; then
    installa_dicendo s "$BANCO/vero" || true
    esiste "$CASA_FINTA/.plebline/plebline-gui" "il vero sta in una cartella sua"
    esiste "$CASA_FINTA/.plebline-prova/plebline-gui" "e quello di prova è ancora lì"
    VOCE3="$CASA_FINTA/.local/share/applications/plebline.desktop"
    contiene "$VOCE3" "Name=PlebLine" "due voci di menu, con due nomi"
    if grep -qF "signet" "$VOCE3"; then esito 1 "il vero non nomina signet"; else esito 0 "il vero non nomina signet"; fi
else
    salta 4 "il pacchetto vero si installa solo firmato, e qui non c'è come firmarlo"
fi

# ----------------------------------------------- 4. the missing network
echo
echo "4. un pacchetto che non dichiara la rete"
pacchetto signet "$BANCO/muto"
rm "$BANCO/muto/programma/RETE"
impronte "$BANCO/muto"
casa_nuova casa4
if installa "$BANCO/muto"; then esito 1 "si ferma invece di indovinare"; else esito 0 "si ferma invece di indovinare"; fi
non_esiste "$CASA_FINTA/.plebline" "e non ha installato niente"
non_esiste "$CASA_FINTA/.plebline-prova" "né sotto l'altro nome"

# ---------------------------------------------------- 5. a missing piece
echo
echo "5. un pacchetto a cui manca la finestra"
pacchetto signet "$BANCO/monco"
rm "$BANCO/monco/programma/plebline-gui"
impronte "$BANCO/monco"
casa_nuova casa5
if installa "$BANCO/monco"; then esito 1 "si ferma e dice quale file manca"; else
    contiene "$BANCO/uscita" "plebline-gui" "si ferma e dice quale file manca"
fi

# -------------------------------------------------- 6. the launcher
echo
echo "6. il lanciatore: cosa esegue, e con quale rete"
casa_nuova casa6
dotfiles_accesi
installa "$BANCO/prova"
LANCIA="$CASA_FINTA/.plebline-prova/avvia"

HOME="$CASA_FINTA" sh "$LANCIA" >/dev/null 2>&1 || true
contiene "$CASA_FINTA/eseguito" "plebline-gui" "esegue la finestra"

HOME="$CASA_FINTA" sh "$LANCIA" --version >"$BANCO/uscita" 2>/dev/null || true
contiene "$CASA_FINTA/eseguito" "plebline-gui --version" "e le passa le opzioni"
contiene "$BANCO/uscita" "stampato da plebline-gui" "quello che stampa arriva al terminale"

# Run from a copy in the home: the persistent disk may be noexec.
esiste "$CASA_FINTA/.plebline-avvio/signet/plebline-gui" "esegue una copia fatta in casa"

# Started from the menu there is no terminal: the reason must reach the user.
cat > "$CASA_FINTA/.plebline-prova/plebline-gui" <<'FINE'
#!/bin/sh
echo "niente OpenGL" >&2
exit 1
FINE
if HOME="$CASA_FINTA" sh "$LANCIA" >"$BANCO/uscita" 2>&1; then
    esito 1 "una finestra che non si apre è un errore"
else
    esito 0 "una finestra che non si apre è un errore"
fi
contiene "$BANCO/uscita" "niente OpenGL" "e il motivo arriva a chi l'ha aperta"

echo
echo "7. il lanciatore con la rete illeggibile"
casa_nuova casa7
installa "$BANCO/prova"
printf 'regtest\n' > "$CASA_FINTA/.plebline-prova/RETE"
if HOME="$CASA_FINTA" sh "$CASA_FINTA/.plebline-prova/avvia" >/dev/null 2>&1; then
    esito 1 "non parte con una rete che non conosce"
else
    esito 0 "non parte con una rete che non conosce"
fi

# -------------------------------- 8. over an earlier version, after a reboot
echo
echo "8. una versione nuova sopra quella di prima, dopo un riavvio"
pacchetto signet "$BANCO/prova2" 2
casa_nuova casa8
dotfiles_accesi
installa "$BANCO/prova"
# Programs that earlier packages shipped and this one does not.
printf 'vecchio\n' > "$DOTFILES_FINTI/.plebline-prova/plebline"
printf 'vecchio\n' > "$DOTFILES_FINTI/.plebline-prova/plebline-guided"
riavvio
if [ -L "$CASA_FINTA/.plebline-prova/plebline-gui" ]; then
    esito 0 "dopo il riavvio i file in casa sono collegamenti, come su Tails"
else
    esito 1 "dopo il riavvio i file in casa sono collegamenti, come su Tails"
fi

if installa "$BANCO/prova2"; then esito 0 "arriva in fondo"; else esito 1 "arriva in fondo"; fi
contiene "$BANCO/uscita" "ci sarà a ogni avvio" "e lo dice a chi ha installato"
contiene "$DOTFILES_FINTI/.plebline-prova/plebline-gui" "versione 2" \
    "l'archiviazione ha la versione nuova"
vero_file "$DOTFILES_FINTI/.plebline-prova/avvia" "e il lanciatore lì è un file vero"
non_esiste "$DOTFILES_FINTI/.plebline-prova/plebline-guided" "i programmi dei pacchetti vecchi spariscono dall'archiviazione"
non_esiste "$CASA_FINTA/.plebline-prova/plebline" "e dalla casa"
contiene "$CASA_FINTA/.plebline-prova/plebline-gui" "versione 2" "la casa ha la versione nuova"
VOCE8="$CASA_FINTA/.local/share/applications/plebline-prova.desktop"
# Writing through the link would change the file in the Dotfiles directory,
# and the menu, which watches this directory, would not notice.
vero_file "$VOCE8" "la voce di menu è riscritta nella sua cartella, dove il menu guarda"
contiene "$VOCE8" "Exec=sh $CASA_FINTA/.plebline-prova/avvia" "e apre ancora la finestra"
nessun_avanzo "$CASA_FINTA" "$DOTFILES_FINTI"

riavvio
HOME="$CASA_FINTA" sh "$CASA_FINTA/.plebline-prova/avvia" >/dev/null 2>&1 || true
contiene "$CASA_FINTA/.plebline-avvio/signet/plebline-gui" "versione 2" \
    "al riavvio seguente parte la versione nuova"

# ------------------------- 9. a working copy that cannot be rewritten in place
#
# On Linux a running program cannot be opened for writing ("Text file busy").
# Not reproducible here; a write-protected file fails the same way for a copy
# made in place, and can be replaced by rename just the same.
echo
echo "9. il lanciatore, con la copia di lavoro che non si lascia riscrivere"
casa_nuova casa9
installa "$BANCO/prova"
LAVORO9="$CASA_FINTA/.plebline-avvio/signet"
HOME="$CASA_FINTA" sh "$CASA_FINTA/.plebline-prova/avvia" >/dev/null 2>&1 || true
chmod a-w "$LAVORO9/plebline-gui"
installa "$BANCO/prova2"
rm -f "$CASA_FINTA/eseguito"
if HOME="$CASA_FINTA" sh "$CASA_FINTA/.plebline-prova/avvia" >"$BANCO/uscita" 2>&1; then
    esito 0 "la finestra parte lo stesso"
else
    esito 1 "la finestra parte lo stesso"
fi
contiene "$CASA_FINTA/eseguito" "plebline-gui" "ed è stata eseguita davvero"
contiene "$LAVORO9/plebline-gui" "versione 2" "dalla versione nuova"
nessun_avanzo "$LAVORO9"

# --------------------------------------- 10. the old version still open
echo
echo "10. con la versione di prima ancora aperta"

# Where pgrep cannot list processes (some sandboxes), a stand-in applies the
# same pattern to the command lines this test starts.
PATH_PROVA=$PATH
if command -v pgrep >/dev/null 2>&1 && pgrep -f . >/dev/null 2>&1; then
    echo "     (pgrep vero)"
    PGREP_VERO=si
else
    echo "     (pgrep non vede i processi qui: lo sostituisce una lista di righe di comando)"
    PGREP_VERO=no
    mkdir -p "$BANCO/bin"
    cat > "$BANCO/bin/pgrep" <<FINE
#!/bin/sh
[ "\$1" = -f ] || exit 2
grep -Eq -- "\$2" "$BANCO/processi" 2>/dev/null
FINE
    chmod +x "$BANCO/bin/pgrep"
    PATH_PROVA="$BANCO/bin:$PATH"
fi
: > "$BANCO/processi"

APERTA="$BANCO/aperta"

# A program left running from a working copy until $APERTA goes away. Its
# HOME is the fake one: whatever it runs stays in the test.
lascia_aperto() {
    cat > "$1" <<FINE
#!/bin/sh
# versione 1, ancora aperta
while [ -f "$APERTA" ]; do sleep 1; done
exit 0
FINE
    HOME="$CASA_FINTA" sh "$1" &
    printf 'sh %s\n' "$1" >> "$BANCO/processi"
    if [ "$PGREP_VERO" = si ]; then
        for _ in 1 2 3 4 5; do
            pgrep -f "$1( |\$)" >/dev/null 2>&1 && break
            sleep 1
        done
    fi
}

finestra10() {
    rm -f "$CASA_FINTA/eseguito"
    HOME="$CASA_FINTA" PATH="$PATH_PROVA" \
        sh "$CASA_FINTA/.plebline-prova/avvia" >"$BANCO/uscita" 2>&1
}

casa_nuova casa10
dotfiles_accesi
installa "$BANCO/prova"
LAVORO10="$CASA_FINTA/.plebline-avvio/signet"
finestra10 || true
: > "$APERTA"

# Earlier packages also had the ceremony in the terminal, on the same folder.
lascia_aperto "$LAVORO10/plebline-guided"
if finestra10; then esito 1 "nemmeno accanto alla cerimonia nel terminale di un pacchetto vecchio"; else
    esito 0 "nemmeno accanto alla cerimonia nel terminale di un pacchetto vecchio"
fi
rm -f "$APERTA"
wait
: > "$BANCO/processi"
: > "$APERTA"

lascia_aperto "$LAVORO10/plebline-gui"
if installa "$BANCO/prova2"; then
    esito 0 "l'installazione arriva in fondo lo stesso"
else
    esito 1 "l'installazione arriva in fondo lo stesso"
fi
contiene "$DOTFILES_FINTI/.plebline-prova/plebline-gui" "versione 2" \
    "e mette la versione nuova nell'archiviazione"

if finestra10; then esito 1 "non apre una seconda finestra accanto a quella vecchia"; else
    esito 0 "non apre una seconda finestra accanto a quella vecchia"
fi
contiene "$BANCO/uscita" "già aperto" "e dice perché"
non_esiste "$CASA_FINTA/eseguito" "senza eseguire niente"
contiene "$LAVORO10/plebline-gui" "ancora aperta" "né tocca la copia aperta"

rm -f "$APERTA"
wait
: > "$BANCO/processi"
finestra10 || true
contiene "$LAVORO10/plebline-gui" "versione 2" "chiusa la vecchia, parte la nuova"

# ---------------------------------------------------- 11. the fingerprints
echo
echo "11. le impronte"

pacchetto signet "$BANCO/senza-elenco"
rm "$BANCO/senza-elenco/IMPRONTE"
rifiuta "$BANCO/senza-elenco" "Manca il file «IMPRONTE»" \
    "senza IMPRONTE si ferma, invece di saltare il controllo"

pacchetto signet "$BANCO/cambiato"
printf 'un byte in più\n' >> "$BANCO/cambiato/programma/plebline-gui"
rifiuta "$BANCO/cambiato" "«programma/plebline-gui»" "un file cambiato: si ferma e dice quale"

pacchetto signet "$BANCO/storto"
printf 'questa non è un impronta\n' >> "$BANCO/storto/IMPRONTE"
rifiuta "$BANCO/storto" "una riga che non capisco" "una riga storta: si ferma, non la salta"

pacchetto signet "$BANCO/esce"
printf '%064d  programma/../../fuori\n' 0 >> "$BANCO/esce/IMPRONTE"
rifiuta "$BANCO/esce" "una riga che non capisco" "un percorso che esce dalla cartella: rifiutato"

pacchetto signet "$BANCO/doppio"
ULTIMA=$(tail -n 1 "$BANCO/doppio/IMPRONTE")
printf '%s\n' "$ULTIMA" >> "$BANCO/doppio/IMPRONTE"
rifiuta "$BANCO/doppio" "nomina due volte" "un file elencato due volte: si ferma"

pacchetto signet "$BANCO/aggiunto"
printf '#!/bin/sh\necho aggiunto\n' > "$BANCO/aggiunto/programma/aiutante"
chmod +x "$BANCO/aggiunto/programma/aiutante"
rifiuta "$BANCO/aggiunto" "«programma/aiutante»" \
    "un file in più, che l'elenco non nomina: si ferma e dice quale"

pacchetto signet "$BANCO/omesso"
grep -v ' programma/reti.sh$' "$BANCO/omesso/IMPRONTE" > "$BANCO/omesso/elenco"
mv "$BANCO/omesso/elenco" "$BANCO/omesso/IMPRONTE"
rifiuta "$BANCO/omesso" "non nomina «programma/reti.sh»" \
    "un file che serve, tolto dall'elenco: si ferma invece di non controllarlo"

# A changed `reti.sh` must be caught before `installa` sources it.
pacchetto signet "$BANCO/trappola"
cat > "$BANCO/trappola/programma/reti.sh" <<'FINE'
: > "$HOME/reti-eseguito"
FINE
rifiuta "$BANCO/trappola" "«programma/reti.sh»" "un reti.sh cambiato: si ferma"
non_esiste "$CASA_FINTA/reti-eseguito" "    senza averlo eseguito"

pacchetto signet "$BANCO/mac"
printf 'x' > "$BANCO/mac/._installa"
printf 'x' > "$BANCO/mac/programma/._plebline-gui"
printf 'x' > "$BANCO/mac/.DS_Store"
casa_nuova casa11
if installa "$BANCO/mac"; then esito 0 "i file che macOS lascia sulle chiavette non fermano niente"; else
    esito 1 "i file che macOS lascia sulle chiavette non fermano niente"
fi

# A stick that returns different bytes on a second read: the file changes
# right after the check, and what gets installed must be the checked copy.
pacchetto signet "$BANCO/cambia"
mkdir -p "$BANCO/bin-cambia" "$BANCO/tmp-cambia"
VERO_SHA=$(command -v sha256sum)
cat > "$BANCO/bin-cambia/sha256sum" <<FINE
#!/bin/sh
ls -ld .. | cut -c1-10 > "$BANCO/permessi"
"$VERO_SHA" "\$@"
esito=\$?
printf 'cambiato dopo il controllo\n' >> "$BANCO/cambia/programma/plebline-gui"
exit \$esito
FINE
chmod +x "$BANCO/bin-cambia/sha256sum"
casa_nuova casa11c
HOME="$CASA_FINTA" PLEBLINE_DOTFILES="$DOTFILES_FINTI" PATH="$BANCO/bin-cambia:$PATH" \
    TMPDIR="$BANCO/tmp-cambia" sh "$BANCO/cambia/installa" </dev/null >"$BANCO/uscita" 2>&1 || true
contiene "$CASA_FINTA/.plebline-prova/plebline-gui" "versione 1" \
    "la chiavetta cambia dopo il controllo: installa la copia controllata"
if grep -qF "cambiato dopo il controllo" "$CASA_FINTA/.plebline-prova/plebline-gui" 2>/dev/null; then
    esito 1 "    non quello che la chiavetta dice dopo"
else
    esito 0 "    non quello che la chiavetta dice dopo"
fi
contiene "$BANCO/permessi" "drwx------" "la copia controllata sta in una cartella solo sua"
if [ -z "$(ls -A "$BANCO/tmp-cambia")" ]; then esito 0 "    che alla fine non c'è più"; else
    esito 1 "    che alla fine non c'è più"
fi

# ------------------------------------------------------- 12. the signature
echo
echo "12. la firma"

# These cases stop before signing, so they need no key. They still use a
# throwaway keyring, never the user's.
mkdir -p "$BANCO/non-pacchetto"
mkdir -m 700 "$BANCO/nessuna-chiave"
if GNUPGHOME="$BANCO/nessuna-chiave" sh "$QUI/scripts/firma-pacchetto.sh" "$BANCO/non-pacchetto" ABCD \
        >"$BANCO/firmato" 2>&1; then
    esito 1 "firma-pacchetto.sh: una cartella che non è un pacchetto non la firma"
else
    contiene "$BANCO/firmato" "non c'è un pacchetto da firmare" \
        "firma-pacchetto.sh: una cartella che non è un pacchetto non la firma"
fi
pacchetto signet "$BANCO/da-firmare-rotto"
printf 'cambiato dopo\n' >> "$BANCO/da-firmare-rotto/programma/avvia"
if GNUPGHOME="$BANCO/nessuna-chiave" sh "$QUI/scripts/firma-pacchetto.sh" "$BANCO/da-firmare-rotto" ABCD \
        >"$BANCO/firmato" 2>&1; then
    esito 1 "firma-pacchetto.sh: file che non tornano con IMPRONTE non li firma"
else
    contiene "$BANCO/firmato" "non firmo" "firma-pacchetto.sh: file che non tornano con IMPRONTE non li firma"
fi
non_esiste "$BANCO/da-firmare-rotto/IMPRONTE.asc" "    e non lascia una firma"
pacchetto signet "$BANCO/da-firmare"
if GNUPGHOME="$BANCO/nessuna-chiave" sh "$QUI/scripts/firma-pacchetto.sh" "$BANCO/da-firmare" \
        0123456789ABCDEF0123456789ABCDEF01234567 >"$BANCO/firmato" 2>&1; then
    esito 1 "firma-pacchetto.sh: con una chiave che non c'è si ferma"
else
    contiene "$BANCO/firmato" "non indica una chiave segreta sola" \
        "firma-pacchetto.sh: con una chiave che non c'è si ferma"
fi

pacchetto bitcoin "$BANCO/vero-senza-firma"
rifiuta "$BANCO/vero-senza-firma" "si installa solo firmato" "il pacchetto vero senza firma: si ferma"

pacchetto signet "$BANCO/prova-senza-firma"
casa_nuova casa12
if installa "$BANCO/prova-senza-firma"; then
    esito 0 "il pacchetto di prova senza firma: installa lo stesso"
else
    esito 1 "il pacchetto di prova senza firma: installa lo stesso"
fi
contiene "$BANCO/uscita" "non è firmato" "    ma lo dice"
contiene "$BANCO/uscita" "NON scriverci le parole" "    e dice cosa non farci"

prove_con_la_firma() {
    pacchetto signet "$BANCO/firmato-pac"
    firma "$BANCO/firmato-pac" vera
    contiene "$BANCO/firmato" "$(a_gruppi vera)" "firma-pacchetto.sh dice l'impronta, a gruppi di quattro"

    casa_nuova casa12
    if installa_dicendo s "$BANCO/firmato-pac"; then
        esito 0 "firmato, e l'impronta è la stessa: installa"
    else
        esito 1 "firmato, e l'impronta è la stessa: installa"
    fi
    esiste "$CASA_FINTA/.plebline-prova/plebline-gui" "    ed è installato davvero"
    contiene "$BANCO/uscita" "$(a_gruppi vera)" "    dopo aver mostrato l'impronta intera, a gruppi di quattro"
    non_contiene "$BANCO/uscita" "non è firmato" "    senza dire che non è firmato"

    rifiuta "$BANCO/firmato-pac" "Non installo" "l'impronta non è la stessa (no): si ferma" n
    rifiuta "$BANCO/firmato-pac" "Non installo" "solo Invio vuol dire no" ""
    rifiuta "$BANCO/firmato-pac" "Non installo" "nessuna risposta vuol dire no"

    pacchetto signet "$BANCO/rifatto"
    firma "$BANCO/rifatto" vera
    printf 'cambiato dopo la firma\n' >> "$BANCO/rifatto/programma/plebline-gui"
    impronte "$BANCO/rifatto"
    rifiuta "$BANCO/rifatto" "NON è valida" \
        "cambiato dopo la firma, con le impronte rifatte: la firma non è valida" s
    non_contiene "$BANCO/uscita" "È la stessa" "    e non chiede nemmeno dell'impronta"

    pacchetto signet "$BANCO/firma-rotta"
    firma "$BANCO/firma-rotta" vera
    printf 'questa non è una firma\n' > "$BANCO/firma-rotta/IMPRONTE.asc"
    rifiuta "$BANCO/firma-rotta" "NON è valida" "una firma illeggibile: si ferma" s

    pacchetto signet "$BANCO/senza-chiave"
    firma "$BANCO/senza-chiave" vera
    rm "$BANCO/senza-chiave/CHIAVE-PUBBLICA.asc"
    rifiuta "$BANCO/senza-chiave" "manca la chiave" "la firma senza la sua chiave: si ferma" s

    # Someone who alters the package can re-sign it with their own key. The
    # signature is then valid, but the fingerprint is not the published one.
    pacchetto signet "$BANCO/altra-chiave"
    firma "$BANCO/altra-chiave" altra
    rifiuta "$BANCO/altra-chiave" "$(a_gruppi altra)" \
        "firmato con un'altra chiave: mostra l'impronta di quella" n
    non_contiene "$BANCO/uscita" "$(a_gruppi vera)" "    non quella giusta, così il confronto la scopre"

    # On Tails the question is a dialog box, like the other messages.
    mkdir -p "$BANCO/bin-zenity"
    cat > "$BANCO/bin-zenity/zenity" <<FINE
#!/bin/sh
printf '%s\n' "\$*" >> "$BANCO/zenity-registro"
case "\$*" in
    *--question*impronta*) exit "\$(cat "$BANCO/zenity-risposta")" ;;
    *--question*) exit 1 ;;
esac
exit 0
FINE
    chmod +x "$BANCO/bin-zenity/zenity"
    # `con_zenity USCITA_DI_ZENITY RISPOSTA_NEL_TERMINALE DIR`
    con_zenity() {
        printf '%s\n' "$1" > "$BANCO/zenity-risposta"
        : > "$BANCO/zenity-registro"
        printf '%s\n' "$2" | HOME="$CASA_FINTA" PLEBLINE_DOTFILES="$DOTFILES_FINTI" \
            PATH="$BANCO/bin-zenity:$PATH" sh "$3/installa" >"$BANCO/uscita" 2>&1
    }

    casa_nuova casa12z
    if con_zenity 1 s "$BANCO/firmato-pac"; then esito 1 "nella finestra, «No»: si ferma"; else
        esito 0 "nella finestra, «No»: si ferma"
    fi
    niente_installato "    e non installa niente, anche se nel terminale c'era un sì"
    contiene "$BANCO/zenity-registro" "--default-cancel" "    la risposta già pronta è «No»"
    contiene "$BANCO/zenity-registro" "$(a_gruppi vera)" "    e l'impronta è nella finestra"
    if con_zenity 0 "" "$BANCO/firmato-pac"; then esito 0 "nella finestra, «Sì, è la stessa»: installa"; else
        esito 1 "nella finestra, «Sì, è la stessa»: installa"
    fi
    casa_nuova casa12z
    if con_zenity 255 s "$BANCO/firmato-pac"; then
        esito 0 "se la finestra non si apre, chiede nel terminale"
    else
        esito 1 "se la finestra non si apre, chiede nel terminale"
    fi
    contiene "$BANCO/uscita" "[s/N]" "    con il no già pronto"
}

if [ "$FIRMA_CON" = niente ]; then
    salta 27 "le prove con una firma: qui non c'è niente con cui firmare"
else
    prove_con_la_firma
fi

echo
if [ "$SALTATE" -gt 0 ]; then
    echo "$SALTATE prove saltate, non fallite: vanno fatte dove si può firmare"
fi
if [ "$FALLITE" -eq 0 ]; then
    echo "tutte le prove sono passate"
else
    echo "$FALLITE prove fallite"
    exit 1
fi
