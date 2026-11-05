# Reproducible builds

The signature on a package says who made it; a rebuild says what is in it.
Anyone who builds the same commit with the same tools, on a machine of the
same kind, should get the same bytes as the published Tails package
(`scripts/build-app.sh signet|bitcoin`: the `plebline-gui` binary, the icon,
the scripts, and `IMPRONTE`, the SHA-256 of every file).

## What is pinned

| what | version | where |
|---|---|---|
| Rust | 1.94.1 | `rust-toolchain.toml`; `build-app.sh` refuses any other |
| zig (C compiler and linker, glibc 2.36 symbols) | 0.16.0 | `scripts/build-app.sh`, checked before building |
| cargo-zigbuild | 0.23.2 | `scripts/build-app.sh`, checked likewise |
| every crate | exact version and checksum | `Cargo.lock`, always `--locked` |
| release profile | `opt-level = "z"`, fat LTO, one codegen unit, `panic = "abort"`, stripped | `Cargo.toml` |
| CI runner and actions | `ubuntu-24.04`, actions pinned by commit | `.github/workflows/ci.yml` |

`build-app.sh` also rewrites every path of the building machine out of the
binaries (`scripts/percorsi-fissi.sh`: the project folder, `$CARGO_HOME` and
the Rust sources become fixed names), sets `SOURCE_DATE_EPOCH` to the commit
date, and refuses a path containing a space. These settings live in the
script, so development builds are untouched.

## Which machines can match

Two builds can be identical only with the **same host triple** (`rustc -vV`,
line `host:`) and the **same build of zig**. The host triple enters cargo's
hashes, and those hashes end up in the program, so an Apple Silicon Mac and an
x86_64 Linux give different binaries even with every tool equal. Homebrew's
zig writes its own compiler names into the binaries; the official build does
not.

The package worth publishing is therefore one built on x86_64 Linux with the
official zig, which anyone with such a machine can rebuild.

## Rebuilding and comparing

```sh
git checkout <commit>
sh scripts/build-app.sh signet      # or bitcoin
diff your/IMPRONTE published/IMPRONTE
```

`IMPRONTE` has one hash per file, so a difference names the file.

- **Against CI.** On every push the `package` job builds the signet package
  on `ubuntu-24.04` with the official zig, prints the hashes in the run
  summary and uploads `IMPRONTE` as the artifact `impronte-signet-<commit>`.
- **On one machine.** `scripts/verifica-riproducibile.sh [signet|bitcoin]
  [commit]` builds a commit twice, from two copies in folders with different
  names, compares the results, and searches the binaries for the build
  folders and the home directory.

## Not yet guaranteed

- Two different x86_64 Linux machines have not yet been shown to match.
- A global cargo configuration (`~/.cargo/config.toml` with a release profile)
  or `CARGO_PROFILE_RELEASE_*` variables would still change the build.
- The tools are trusted, not rebuilt: rustup checks the toolchain against the
  Rust project's manifest, and CI checks zig against Zig's signing key.
