#!/usr/bin/env sh
# Signs a Tails package made by `build-app.sh`, with the project's key:
#
#     sh scripts/firma-pacchetto.sh dist/PlebLine <impronta della chiave>
#
# Writes `IMPRONTE.asc`, a detached signature of `IMPRONTE` (which holds the
# hash of every other file), and `CHIAVE-PUBBLICA.asc`. `installa` checks the
# first with the second, then shows the key's fingerprint for the user to
# compare with the published one, because whoever alters the package can also
# replace the key in it.

set -eu

fermo() {
    printf '\n%s\n\n' "$1" >&2
    exit 1
}

if [ $# -ne 2 ]; then
    echo "uso: sh scripts/firma-pacchetto.sh <cartella del pacchetto> <impronta della chiave>" >&2
    exit 2
fi
PACCHETTO=$1
CHIAVE=$2

[ -f "$PACCHETTO/IMPRONTE" ] && [ -f "$PACCHETTO/programma/RETE" ] \
    || fermo "In «${PACCHETTO}» non c'è un pacchetto da firmare: mancano IMPRONTE o programma/RETE. Prima costruiscilo con: sh scripts/build-app.sh bitcoin (o signet)."
command -v gpg >/dev/null 2>&1 \
    || fermo "Manca gpg. Sul Mac c'è con GPG Suite, oppure: brew install gnupg"
RETE=$(tr -d ' \t\n\r' < "$PACCHETTO/programma/RETE")

# The files are checked against IMPRONTE first, so that a package changed
# after the build does not get a valid signature over the wrong files.
if command -v sha256sum >/dev/null 2>&1; then
    CONTA="sha256sum"
else
    CONTA="shasum -a 256"
fi
( cd "$PACCHETTO" && $CONTA -c --strict --quiet IMPRONTE >/dev/null 2>&1 ) \
    || fermo "I file in «${PACCHETTO}» non tornano con IMPRONTE: non firmo. Rifai il pacchetto con build-app.sh."

# Requires exactly one matching secret key that can sign and is not expired or
# revoked. A name can match several keys, and users would refuse a package
# signed with the wrong one when they compare the fingerprint.
SEGRETE=$(gpg --batch --with-colons --list-secret-keys -- "$CHIAVE" 2>/dev/null || true)
QUANTE=$(printf '%s\n' "$SEGRETE" | grep -c '^sec:' || true)
[ "$QUANTE" -eq 1 ] \
    || fermo "«${CHIAVE}» non indica una chiave segreta sola (ne trovo ${QUANTE}). Scrivi l'impronta intera, quella di: gpg --list-secret-keys"
printf '%s\n' "$SEGRETE" | awk -F: '$1 == "sec" && $2 !~ /[erd]/ && $12 ~ /S/ { ok = 1 } END { exit !ok }' \
    || fermo "La chiave «${CHIAVE}» non può firmare: è scaduta, revocata, o fatta solo per cifrare."
IMPRONTA=$(printf '%s\n' "$SEGRETE" | awk -F: '$1 == "fpr" { print $10; exit }')

rm -f "$PACCHETTO/IMPRONTE.asc" "$PACCHETTO/CHIAVE-PUBBLICA.asc"
echo "==> firmo IMPRONTE (rete $RETE): gpg può chiedere la passphrase della chiave"
gpg --armor --local-user "$IMPRONTA" --output "$PACCHETTO/IMPRONTE.asc" \
    --detach-sign "$PACCHETTO/IMPRONTE" \
    || fermo "gpg non ha firmato: niente è cambiato nel pacchetto."
gpg --batch --armor --export-options export-minimal \
    --output "$PACCHETTO/CHIAVE-PUBBLICA.asc" --export "$IMPRONTA" \
    || fermo "gpg non ha esportato la chiave pubblica."

# Verifies the new signature as `installa` will, in a keyring that holds only
# the key from the package.
PROVA=
trap '[ -z "$PROVA" ] || rm -rf "$PROVA"' EXIT
PROVA=$(mktemp -d "${TMPDIR:-/tmp}/firma-pacchetto.XXXXXX")
chmod 700 "$PROVA"
GNUPGHOME="$PROVA" gpg --batch --no-autostart --quiet \
    --import "$PACCHETTO/CHIAVE-PUBBLICA.asc" >/dev/null 2>&1 || true
VISTA=$(GNUPGHOME="$PROVA" gpg --batch --no-autostart --status-fd 1 \
    --verify "$PACCHETTO/IMPRONTE.asc" "$PACCHETTO/IMPRONTE" 2>/dev/null \
    | awk '$2 == "VALIDSIG" { print ($12 != "" ? $12 : $3) }' || true)
if [ "$VISTA" != "$IMPRONTA" ]; then
    rm -f "$PACCHETTO/IMPRONTE.asc" "$PACCHETTO/CHIAVE-PUBBLICA.asc"
    fermo "La firma appena fatta non risulta valida con la chiave messa nel pacchetto. Ho tolto firma e chiave: non distribuire questo pacchetto."
fi

GRUPPI=$(printf '%s\n' "$IMPRONTA" | sed 's/..../& /g; s/ $//')
cat <<FINE

Firmato: $PACCHETTO (rete $RETE)

    IMPRONTE.asc          la firma
    CHIAVE-PUBBLICA.asc   la chiave pubblica, per controllarla

L'impronta della chiave che ha firmato, quella che installa mostrerà:

    $GRUPPI

Deve essere identica a quella pubblicata (README, sito). Se non lo è, hai
firmato con la chiave sbagliata: non distribuire il pacchetto.
FINE
