#!/usr/bin/env python3
"""Reads Linux binaries where readelf is missing, as on a Mac.

    elf.py dipendenze FILE...   shared libraries loaded at launch
    elf.py confronta A B        sections that differ between two builds

Anything beyond libc and libm would have to be installed on Tails, so
`dipendenze` reports it at build time instead of at the first launch there.
`confronta` is a first hint at why two builds are not identical.
"""
import struct
import sys

NOBITS = 8
DT_NULL, DT_NEEDED = 0, 1


def sezioni(percorso):
    """Section name -> (offset, size, type), from the 64-bit little-endian header."""
    dati = open(percorso, "rb").read()
    if dati[:4] != b"\x7fELF":
        raise SystemExit(f"{percorso}: non è un ELF")
    shoff = struct.unpack_from("<Q", dati, 0x28)[0]
    shentsize, shnum, shstrndx = struct.unpack_from("<HHH", dati, 0x3A)
    righe = [struct.unpack_from("<IIQQQQ", dati, shoff + i * shentsize) for i in range(shnum)]
    nomi = righe[shstrndx][4]
    tabella = {}
    for nome, tipo, _flag, _ind, inizio, misura in righe:
        fine = dati.index(b"\0", nomi + nome)
        tabella[dati[nomi + nome : fine].decode()] = (inizio, misura, tipo)
    return dati, tabella


def dipendenze(percorso):
    dati, tabella = sezioni(percorso)
    if ".dynamic" not in tabella:
        return []  # statically linked
    dyn = tabella[".dynamic"][0]
    dynstr = tabella[".dynstr"][0]
    librerie = []
    while True:
        tag, val = struct.unpack_from("<qQ", dati, dyn)
        dyn += 16
        if tag == DT_NULL:
            break
        if tag == DT_NEEDED:
            inizio = dynstr + val
            librerie.append(dati[inizio : dati.index(b"\0", inizio)].decode())
    return sorted(librerie)


def contenuti(percorso):
    dati, tabella = sezioni(percorso)
    return {
        nome: b"" if tipo == NOBITS else dati[inizio : inizio + misura]
        for nome, (inizio, misura, tipo) in tabella.items()
    }


def main(argomenti):
    # No `match`: the Mac's python3 may be 3.9.
    if len(argomenti) >= 2 and argomenti[0] == "dipendenze":
        for percorso in argomenti[1:]:
            librerie = dipendenze(percorso)
            if librerie:
                print(f"{percorso}:")
                for lib in librerie:
                    print(f"    {lib}")
            else:
                print(f"{percorso}: statico, nessuna libreria da caricare")
    elif len(argomenti) == 3 and argomenti[0] == "confronta":
        a, b = contenuti(argomenti[1]), contenuti(argomenti[2])
        for nome in sorted(set(a) | set(b)):
            if a.get(nome) != b.get(nome):
                print(f"        sezione {nome or '(senza nome)'}: diversa")
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
