# Decisions

The choices the code alone does not explain, with their reason.

## One binary holds both the keys and the network

Two binaries, one with keys and one with the network, exchanging files by
USB stick, was friction nobody would accept. The separation moved to the
machine: an amnesic system with the network unplugged lets no packet out,
whatever the code does. What is still enforced: `plebline-core` (keys and
signing) has no network crate in its tree (CI job `offline-core`), and the
network libraries are named in one file, `plebline/src/chain.rs`.

## The network is chosen at compile time

`--features signet` builds a different program, whose version line and
first screen say it cannot touch real coins. A test build cannot be mistaken
for the real one, and the real one cannot be talked into another network.

## Questions and the chain are passed explicitly, the screen is not

Questions go to `&mut dyn Ask` and the network to `&mut dyn Chain`, with
`Scripted` and `Fake` for tests. `chain.rs` holds only what sends a packet,
so tests exercise the real logic on the answers; the first end-to-end tests
found two defects on the main path that no unit test had seen.

Screen lines are environment, not dialogue: a parameter would have reached
hundreds of functions in code that moves money, for information none of them
use. The sink is per thread because the window runs the ceremony on a thread
of its own.

## The program never generates seed words

A seed drawn by a program is worth as much as the program, the machine and
whoever supplied them. A user without a wallet is told how to make one by
hand, with dice or cards; the program only recomputes the checksum bits of
the last word and gives its row number in the word list, not the word. That
help is offered only to someone making a new seed: for an existing one it
would turn a copying mistake into a valid wallet that is not theirs.

## The words are asked every time, and dropped before the network returns

`wallet.json` decides nothing: it is rewritten from the words at each
opening. Before the words, the program checks that the chosen server answers
(tip height only), so a missing Tor is found before 24 words are typed. The
cost, stated on screen: the words are typed with the network still up,
because the scan needs the public descriptors.

They then stay in memory while the network is unplugged, so several rounds
need one typing, and are dropped before it is plugged back in for any
reason (`home::plug_back`). Signing after that asks for them again and
checks they open the same wallet.

## Tor inside the program, `socks5h`, onion servers first

Scanning a descriptor hands the server every address of the wallet. The
SOCKS5 client lives in the program (the async Esplora client, the only one
that speaks SOCKS), `socks5h` keeps DNS inside Tor, and one function decides
the route: Tor, the user's own node, or refused before any packet. The
default servers are onion addresses, then clearnet through Tor; one that
does not answer hands over to the next.

## Commitments come from the signed files, and last one session

A signed, unbroadcast refill commits its coin, which looks free on chain. A
registry of committed coins was rejected: it is a copy of a fact that lives
in the signed files, and a copy that outlived its batch would lock the user
out of their own coins. The signed files are the only truth, read from their
signed bytes, machine-wide (a signature commits its coin wherever the file
sits), skipping another wallet's files. Reading other folders only adds
commitments; deleting is always limited to the current ceremony.

Every opening starts from zero: the signed files of earlier sessions are set
aside in `precedenti/<n>/`, unread, and the chain is asked nothing about
them. Refills are sent from the phone, outside the program, over months;
on Tails without persistence the files were gone at the next start anyway,
and checks at opening misfired on sound groups. The accepted cost: a new
refill can spend the coin of an old one still unsent, and then the one sent
first counts. The reminder before the first signature of each session says
so.

## Committed coins are shown, never selectable; "Replace" frees one

Only free coins are spent. To use a committed one, "Replace" withdraws the
single refill that holds it, after the chain says it was never sent, with a
confirmation that defaults to no and warns that a copy on a stick or in a QR
is still valid. The other refills of the group stay valid, and no
cancellation file is written: the truth stays in the batches.

## RBF is signalled, and the fee is frozen

BDK's default sequence signals RBF. The fee, 1–500 sat/vB, is chosen by the
user and fixed at signing; raising it means signing again, and CPFP from the
change needs the cold storage too. Only the recipient can speed up a stuck
refill without it. This is a property of presigning, stated to the user.

## Recipient addresses come from the phone, after the coins

The phone step sits just before "how many refills": only a ceremony that
will sign needs it, and the number of addresses received becomes the
proposed count. PlebLine itself receives nothing from the network
(OnionShare, a separate program, writes a text file), and it reads at most
64 KiB of text through the strict list parser.

OnionShare runs in public mode, because on a real Tails the phone could not
use its private key. The onion address is long, random, shown only on the
machine's screen and alive a few minutes, and every address that arrives is
shown back in groups of four characters and as a QR to scan back, before a
confirmation that defaults to no.

## Control characters are shown, not obeyed

Labels and addresses read from files or from the phone may carry escape
sequences or bidi overrides. They are neutralized when shown, not refused
when read: refusing a batch over an unsigned label would lock the user out of
their own refills.

## Before sending, only what cannot have changed is checked again

The full check runs right after signing, against the refills' definition and
the coin list. Both change legitimately afterwards: a new round replaces the
definition, and reading the chain again drops the coins already spent.
Checked against them before sending, healthy groups were refused. The
signatures prove the group is the one signed, so before sending only the
signatures, independence and the change coming home are checked again.
