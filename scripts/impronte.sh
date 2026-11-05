#!/usr/bin/env sh
# Writes `IMPRONTE` in a Tails package: one SHA-256 per file, in the only
# format `installa` accepts. Shared by `build-app.sh` and
# `prova-tails-app.sh`, so what the test checks is what the build writes.
#
#     sh scripts/impronte.sh dist/PlebLine-prova

set -eu

if [ $# -ne 1 ] || [ ! -d "$1" ]; then
    echo "uso: sh scripts/impronte.sh <cartella del pacchetto>" >&2
    exit 2
fi

if command -v sha256sum >/dev/null 2>&1; then
    impronta() { sha256sum "$1"; }
else
    impronta() { shasum -a 256 "$1"; }
fi

cd "$1"
find installa LEGGIMI.txt programma senza-terminale -type f | sort | while IFS= read -r f; do
    h=$(impronta "$f" | cut -d' ' -f1)
    [ "${#h}" -eq 64 ] || { echo "non riesco a leggere $f" >&2; exit 1; }
    printf '%s  %s\n' "$h" "$f"
done > IMPRONTE
