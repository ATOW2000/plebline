#!/usr/bin/env sh
# Builds the Tails package, a directory to copy onto a USB stick.
#
#     sh scripts/build-app.sh signet     # test package
#     sh scripts/build-app.sh bitcoin    # mainnet package
#
# The network has no default because the two packages look alike, and a stick
# with the wrong one would be noticed late. The package carries a `RETE` file,
# and the installer and the launcher take the network from it.
#
# The package holds one program, the window. It is linked against glibc
# because it loads X11/Wayland and OpenGL at runtime, so it cannot be static.
#
# The same commit, built with the same tools on the same kind of machine,
# gives the same bytes in any directory (docs/REPRODUCIBLE.md).
#
# Requires the Rust version in rust-toolchain.toml, zig and cargo-zigbuild at
# the versions below, and readelf or python3.

set -eu

# The paths below are relative to the project root.
cd "$(dirname "$0")/.."

# The per-network names are in reti.sh, which also ships in the package for
# `installa`.
. scripts/tails-app/reti.sh
descrivi_rete "${1:-}" || {
    echo "uso: sh scripts/build-app.sh [signet|bitcoin]" >&2
    exit 2
}

GNU=x86_64-unknown-linux-gnu.2.36
COSTRUITI=target/x86_64-unknown-linux-gnu/release
OUT="dist/$CARTELLA"

# ------------------------------------------------------------ the tools
#
# A different version of any of these produces different bytes, and then
# nobody can check the package by rebuilding it, so a mismatch stops the build.
ZIG_FISSATO=0.16.0
ZIGBUILD_FISSATO=0.23.2
RUST_FISSATO=$(sed -n 's/^channel *= *"\(.*\)"$/\1/p' rust-toolchain.toml)

# `versione PROGRAMMA TROVATA FISSATA COME_AVERLA`
versione() {
    [ "$2" = "$3" ] && return 0
    if [ -n "$2" ]; then
        echo "$1: c'è la versione ${2}, questo pacchetto si costruisce con la ${3}." >&2
    else
        echo "$1: non lo trovo, e questo pacchetto si costruisce con la versione ${3}." >&2
    fi
    echo "    Con un'altra versione i binari escono diversi, e nessuno potrebbe" >&2
    echo "    ricontrollarli costruendo di nuovo lo stesso commit. Per averla:" >&2
    echo "        $4" >&2
    exit 1
}

echo "==> controllo gli strumenti"
# rustup reads rust-toolchain.toml here, and may install that version first.
versione rustc "$(rustc -V | cut -d' ' -f2)" "$RUST_FISSATO" \
    "rustup toolchain install $RUST_FISSATO --profile minimal --component rustfmt,clippy --target x86_64-unknown-linux-gnu"
versione zig "$(zig version 2>/dev/null || true)" "$ZIG_FISSATO" \
    "https://ziglang.org/download/ (la $ZIG_FISSATO, controllando la firma)"
versione cargo-zigbuild "$(cargo-zigbuild --version 2>/dev/null | cut -d' ' -f2 || true)" "$ZIGBUILD_FISSATO" \
    "cargo install cargo-zigbuild --version $ZIGBUILD_FISSATO --locked"
echo "    rustc $RUST_FISSATO, zig $ZIG_FISSATO, cargo-zigbuild $ZIGBUILD_FISSATO"
# Homebrew's zig has the same version number but links Homebrew's LLVM, whose
# name ends up in the binaries. The linker is asked because `zig cc --version`
# leaves an empty a.o behind.
case "$(zig ld.lld --version 2>/dev/null | head -n 1)" in
    Homebrew*)
        echo "    attenzione: questo zig è quello di Homebrew, che compila con l'LLVM di"
        echo "    Homebrew e ne scrive il nome nei binari. Tornano uguali solo a chi"
        echo "    usa lo stesso zig di Homebrew (docs/REPRODUCIBLE.md)."
        ;;
esac

# ------------------------------------------------------ the same bytes
#
# Keeps this machine's paths out of the binaries, so the same commit gives the
# same bytes in any directory and the package does not carry the builder's
# user name.
. scripts/percorsi-fissi.sh
percorsi_fissi x86_64-unknown-linux-gnu

# Tools that stamp a date get the commit's date.
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
    SOURCE_DATE_EPOCH=$(git log -1 --format=%ct 2>/dev/null) || {
        echo "Non trovo la data del commit: qui non c'è git." >&2
        echo "    Dalla copia di un commit si passa a mano:" >&2
        echo "        SOURCE_DATE_EPOCH=<data del commit, in secondi> sh scripts/build-app.sh $RETE" >&2
        exit 1
    }
fi
export SOURCE_DATE_EPOCH

# The network is fixed at build time.
echo "==> compilo per Linux (glibc), rete $RETE"
if [ "$RETE" = signet ]; then
    cargo zigbuild --locked --release --target "$GNU" -p plebline-gui --features signet
else
    cargo zigbuild --locked --release --target "$GNU" -p plebline-gui
fi

# Anything left from a previous build would end up in the package.
rm -rf "$OUT"
mkdir -p "$OUT/programma"
cp scripts/tails-app/plebline.png "$OUT/programma/plebline.png"

echo "==> metto insieme $OUT"
mkdir -p "$OUT/senza-terminale"
cp "$COSTRUITI/plebline-gui" "$OUT/programma/plebline-gui"
cp scripts/tails-app/avvia     "$OUT/programma/avvia"
cp scripts/tails-app/voce.desktop "$OUT/programma/voce.desktop"
cp scripts/tails-app/reti.sh "$OUT/programma/reti.sh"
printf '%s\n' "$RETE" > "$OUT/programma/RETE"
# In the package root, because it is the file the user drags into the terminal.
cp scripts/tails-app/installa  "$OUT/installa"
sed -e "s|@NOME@|$NOME|" \
    -e "s|@MONETE@|$MONETE|" \
    scripts/tails-app/LEGGIMI.txt > "$OUT/LEGGIMI.txt"

# A menu entry to copy by hand, for users who never open the terminal. It
# points at the package directory copied into the home, since without
# `installa` nothing is in ~/.plebline.
sed -e '/^#/d' \
    -e "s|@NOME@|$NOME|" \
    -e "s|@COMMENTO@|$COMMENTO|" \
    -e "s|@CASA@|/home/amnesia/$CARTELLA/programma|" \
    scripts/tails-app/voce.desktop > "$OUT/senza-terminale/$VOCE"

chmod +x "$OUT/installa" "$OUT/programma/avvia" "$OUT/programma/plebline-gui"

# The libraries the program loads at launch. Anything beyond libc would have
# to be installed on Tails, so the list is printed at build time.
echo "==> da cosa dipende"
if command -v readelf >/dev/null 2>&1; then
    readelf -d "$OUT"/programma/plebline-gui | sed -n 's/.*(NEEDED).*\[\(.*\)\]/    \1/p'
else
    python3 scripts/elf.py dipendenze "$OUT"/programma/plebline-gui
fi

# The hashes that `installa` checks before it installs anything.
sh scripts/impronte.sh "$OUT"

echo
echo "==> impronte (SHA-256): lo stesso commit, con gli stessi strumenti su una"
echo "    macchina dello stesso tipo, deve dare le stesse"
grep -E '  programma/plebline-gui$' "$OUT/IMPRONTE" | sed 's/^/    /'
if command -v sha256sum >/dev/null 2>&1; then
    TUTTE=$(sha256sum "$OUT/IMPRONTE" | cut -d' ' -f1)
else
    TUTTE=$(shasum -a 256 "$OUT/IMPRONTE" | cut -d' ' -f1)
fi
echo "    $TUTTE  IMPRONTE"

echo
find "$OUT" | sort
echo
echo "==> la cartella pesa $(du -sh "$OUT" | cut -f1). Copiala su una chiavetta così com'è."
if [ "$RETE" = bitcoin ]; then
    echo "==> ma prima firmala: senza firma installa la rifiuta"
else
    echo "==> senza firma installa avvisa e installa lo stesso; per firmarla:"
fi
echo "    sh scripts/firma-pacchetto.sh $OUT <impronta della chiave>"
