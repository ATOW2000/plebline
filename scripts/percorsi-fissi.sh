# Keeps this machine's paths out of the binaries, for the package scripts.
# Panic messages name their source file, so without this a
# package would carry the builder's home directory (user name included) and
# project folder.
#
#     cd <project>
#     . scripts/percorsi-fissi.sh
#     percorsi_fissi x86_64-unknown-linux-gnu    # the target the C code is for
#
# The paths are rewritten to fixed names for Rust and for the C code inside
# `ring` and `secp256k1`. Rust's own sources get the name the toolchain
# already gives them, with or without rust-src. Exports
# CARGO_ENCODED_RUSTFLAGS with these flags only, which also keeps RUSTFLAGS or
# a cargo config elsewhere from adding others.

percorsi_fissi() {
    RADICE=$(pwd -P)
    CASA_CARGO=${CARGO_HOME:-$HOME/.cargo}
    CASA_CARGO_VERA=$(cd "$CASA_CARGO" 2>/dev/null && pwd -P || echo "$CASA_CARGO")
    SORGENTI_RUST="$(rustc --print sysroot)/lib/rustlib/src/rust"
    COMMIT_RUST=$(rustc -vV | sed -n 's/^commit-hash: //p')

    # The C compiler's flags are split on spaces.
    for percorso in "$RADICE" "$CASA_CARGO" "$CASA_CARGO_VERA"; do
        case "$percorso" in
            *[[:space:]]*)
                echo "Questo percorso contiene uno spazio: $percorso" >&2
                echo "    Il codice C dentro i binari si porterebbe dietro il nome della" >&2
                echo "    cartella. Sposta il progetto (o CARGO_HOME) in un percorso senza spazi." >&2
                exit 1
                ;;
        esac
    done

    US=$(printf '\037')
    PER_RUST="--remap-path-prefix=$RADICE=/plebline$US--remap-path-prefix=$CASA_CARGO=/cargo"
    PER_C="-ffile-prefix-map=$RADICE=/plebline -ffile-prefix-map=$CASA_CARGO=/cargo"
    if [ "$CASA_CARGO_VERA" != "$CASA_CARGO" ]; then
        PER_RUST="$PER_RUST$US--remap-path-prefix=$CASA_CARGO_VERA=/cargo"
        PER_C="$PER_C -ffile-prefix-map=$CASA_CARGO_VERA=/cargo"
    fi
    PER_RUST="$PER_RUST$US--remap-path-prefix=$SORGENTI_RUST=/rustc/$COMMIT_RUST"

    export CARGO_ENCODED_RUSTFLAGS="$PER_RUST"
    export "CFLAGS_$(echo "$1" | tr '-' '_')=$PER_C"
    # The package scripts copy the binaries from here after the build.
    export CARGO_TARGET_DIR="$RADICE/target"
}
