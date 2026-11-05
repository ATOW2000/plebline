#!/usr/bin/env sh
# Builds the same commit twice, in two directories with different names and
# lengths, and compares the two Tails packages byte for byte.
#
#     sh scripts/verifica-riproducibile.sh                 # HEAD, signet
#     sh scripts/verifica-riproducibile.sh bitcoin 1a2b3c4
#
# Runs two full builds one after the other, which takes a while. On a Mac with
# 8 GB, prefix CARGO_BUILD_JOBS=2. It shows only that this machine's paths and
# clock do not reach the binaries; whether another machine gets the same bytes
# is covered in docs/REPRODUCIBLE.md.

set -eu

QUI=$(cd "$(dirname "$0")/.." && pwd -P)
RETE=${1:-signet}
case "$RETE" in
    signet | bitcoin) ;;
    *)
        echo "uso: sh scripts/verifica-riproducibile.sh [signet|bitcoin] [commit]" >&2
        exit 2
        ;;
esac
COMMIT=$(git -C "$QUI" rev-parse --verify "${2:-HEAD}^{commit}")
# The copies are not git checkouts, so build-app.sh takes the commit date from
# this variable.
SOURCE_DATE_EPOCH=$(git -C "$QUI" log -1 --format=%ct "$COMMIT")
export SOURCE_DATE_EPOCH

if command -v sha256sum >/dev/null 2>&1; then
    impronta() { sha256sum "$1" | cut -d' ' -f1; }
else
    impronta() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi

BANCO=$(mktemp -d "${TMPDIR:-/tmp}/riproducibile.XXXXXX")
BANCO_VERO=$(cd "$BANCO" && pwd -P)
# A path that slipped into a binary shows up as a difference only if the two
# paths differ, length included.
UNO="$BANCO/a/PlebLine"
DUE="$BANCO/una-cartella-dal-nome-molto-piu-lungo/e-ancora/PlebLine"

echo "==> commit $COMMIT, rete $RETE"
if [ -n "$(git -C "$QUI" status --porcelain --untracked-files=no)" ]; then
    echo "    (le modifiche non salvate non entrano: si costruisce il commit)"
fi

for COPIA in "$UNO" "$DUE"; do
    mkdir -p "$COPIA"
    git -C "$QUI" archive --format=tar "$COMMIT" | tar -xf - -C "$COPIA"
    echo "==> costruisco in $COPIA"
    INIZIO=$(date +%s)
    if ! sh "$COPIA/scripts/build-app.sh" "$RETE" >"$COPIA.log" 2>&1; then
        tail -n 30 "$COPIA.log" >&2
        echo >&2
        echo "La costruzione non è riuscita. Il log completo è in $COPIA.log" >&2
        exit 1
    fi
    ORA=$(date +%s)
    echo "    fatta in $(( (ORA - INIZIO) / 60 )) minuti"
done

. "$UNO/scripts/tails-app/reti.sh"
descrivi_rete "$RETE"
A="$UNO/dist/$CARTELLA"
B="$DUE/dist/$CARTELLA"

echo
echo "==> confronto (SHA-256)"
DIVERSI=0
for FILE in programma/plebline-gui IMPRONTE; do
    HA=$(impronta "$A/$FILE")
    HB=$(impronta "$B/$FILE")
    echo "    $FILE"
    echo "        prima:   $HA"
    echo "        seconda: $HB"
    if [ "$HA" = "$HB" ]; then
        echo "        uguali"
    else
        echo "        DIVERSI"
        DIVERSI=$((DIVERSI + 1))
        case "$FILE" in
            programma/*) python3 "$QUI/scripts/elf.py" confronta "$A/$FILE" "$B/$FILE" ;;
            *) diff "$A/$FILE" "$B/$FILE" | sed 's/^/        /' || true ;;
        esac
    fi
done

# Two identical builds can still both contain the same path of this machine.
echo
echo "==> percorsi di questa macchina dentro i binari"
TRACCE=0
for FILE in programma/plebline-gui; do
    for CERCA in "$BANCO" "$BANCO_VERO" "$HOME" "${CARGO_HOME:-$HOME/.cargo}"; do
        if grep -qaF -- "$CERCA" "$A/$FILE"; then
            echo "    $FILE contiene $CERCA"
            TRACCE=$((TRACCE + 1))
        fi
    done
done
[ "$TRACCE" -eq 0 ] && echo "    nessuno"

echo
if [ "$DIVERSI" -eq 0 ] && [ "$TRACCE" -eq 0 ]; then
    rm -rf "$BANCO"
    echo "==> riproducibile: due costruzioni, byte per byte uguali"
else
    echo "==> NON riproducibile: $DIVERSI file diversi, $TRACCE percorsi rimasti dentro"
    echo "    Le due copie restano per guardarci dentro: $BANCO"
    exit 1
fi
