# Syzygy tablebases

Jakgro reads Syzygy endgame tablebases when `SyzygyPath` is set. This report records why they were the
next thing to add, what was added, and what it measured, on the engine series ending at
`e6e36aca254e054697717ec013ee4f96ae6aeca8`. Without a path nothing changes: fixed-depth searches visit
the parent's nodes exactly and the search runs as fast.

## What the external games showed

The engine's reports were checked against the three-, four- and five-man tables by replaying the 1,800
games of the Stockfish-skill gauntlet, 1,000 at 30+0.3 and 800 at 120+1, against Stockfish at skill
levels 10 to 19 and Stash 25.0. The build that played them predates the NNUE speed series.

Of the 69,146 positions in the games with at most five pieces and no castling rights, 34,594 had Jakgro
to move. Its reported evaluation there, against the tablebase result:

| Reported evaluation | Positions | Tablebase win | Draw | Loss |
| --- | ---: | ---: | ---: | ---: |
| +2.50 or more | 16,184 | 5.5% | 94.5% | 0.0% |
| +1.00 to +2.49 | 9,151 | 0.5% | 99.5% | 0.0% |
| -0.99 to +0.99 | 8,788 | 2.1% | 97.8% | 0.0% |
| -2.49 to -1.00 | 47 | 0.0% | 100.0% | 0.0% |
| -2.50 or less | 424 | 0.0% | 9.2% | 90.8% |

Nearly half of the positions carried an evaluation of at least +2.5, and 94.5% of those were draws. The
evaluation cannot tell a won ending from a dead one.

That costs points where a win has to be converted. Jakgro's moves in range changed the theoretical result
185 times in 34,594 (0.53%): 174 wins became draws, 3 wins became losses and 8 draws became losses. The
opponents' 34,552 moves did so 246 times, 243 of them draws that became losses. 168 games contained a
tablebase-won position for Jakgro at some point; it won 79, drew 89 and lost none. The 44.5 points that
left are 2.5% of all games, about 17 Elo at an even score, and are a ceiling on what perfect play in
five-man endings could return against these opponents. The drops were rook endings, KRPvKR 52, KRBvKR
27, KRNvKR 26, KQPvKQ 16 and KBNvKN 15, and in 93% of them the halfmove clock had reached 50 or more
(median 92): the win ran out the fifty-move rule. Nothing steered the search toward a zeroing move,
because the evaluation reads +2.5 for a win and for a draw alike. At first entry into tablebase range
Jakgro gave up only half a point across 576 games, so the loss is in the conversion and not the arrival.

## What was added

- **Options.** `SyzygyPath` (directories separated by colons), `SyzygyProbeDepth` (1 to 100, default
  1), `Syzygy50MoveRule` (default true) and `SyzygyProbeLimit` (0 to 7, default 7, narrowed to the
  largest table found). Loading reports `info string Found N WDL and M DTZ tablebase files (up to
  K-man)`; a path with no usable tables is refused with `info string SyzygyPath rejected: …`.
- **The library.** `pyrrhic-rs` 0.2 (MIT), a Rust translation of Pyrrhic and Fathom that borrows the
  engine's own attack generation; its only other dependency is `memmap2`. It keeps its tables in
  process-wide state, so one set exists at a time and is shared by every holder of a path; the root probe
  is not thread safe, so it is serialized and made once before any helper thread starts. It reports seven
  pieces as its largest table whatever files it is given, so the size is read from the file names, and a
  load that finds nothing usable leaves it unable to load again, so directories are checked first.
- **The root.** Each move of a covered root is ranked from the distance-to-zero tables as Stockfish ranks
  them: every move that keeps a win inside the fifty-move rule ranks alike, so the search chooses among
  them; a move that lets the win run out ranks below them; losses rank alike until the rule is in sight;
  draws rank between. Only the best-ranked moves are searched, and the root reports the tablebase score.
  Without distance files the moves are ranked by the win-draw-loss result of the reply.
- **The search.** A position that the last move has just cleared the halfmove clock of, with no castling
  rights and no more pieces than the limit, is answered from the tables in the main search and at every
  quiescence node, and the result is returned whatever the window. A win or loss is stored as a bound and a
  draw as exact; with the fifty-move rule on, a result it cancels scores 2 centipawns to the side that
  would have won. Results are stored six plies deeper than they were searched.
- **Scores.** A tablebase win found `ply` plies down scores `TB_WIN_SCORE - ply`, in the band between
  the highest network evaluation (16,000) and the shortest mate score. The table's ply normalization covers the
  band, and every rule that keeps evaluation-based pruning away from mate scores (the null-move window,
  static and reverse-futility pruning, aspiration windows, singular extensions and the styled root) now
  treats it alike. A decided ending is reported as `cp ±(20000 - plies)`; `tbhits` counts the positions
  the main searcher settled.
- **Tools.** `selfplay` and `tools/run_sprt.py` take `--candidate-syzygy-path` and
  `--baseline-syzygy-path`, so one binary can play itself with tables on one side.

## Two things the first gauntlet showed

The first gauntlet with tables (1,000 games at 30+0.3) converted 40 of the 41 won endings it reached,
but one, against Stash, was drawn by repetition from a won ending three plies from a zeroing move: Jakgro
reported +200.00 and moved its king back and forth for 40 plies. Two things were wrong.

- **A tablebase result was used only where it crossed the search window.** Once the root's score is
  decided the aspiration window is opened fully, as it is for mates, and inside a full window a
  tablebase win never reaches beta, so the tree ignored it and a pawn push toward a decided ending looked
  no better than a shuffle beside it. Results are now returned whatever the window.
- **The root ranked every win that fitted the fifty-move budget alike.** Nothing kept the search from
  repeating positions long before the counter mattered. As Stockfish does, once a position since the last
  zeroing move has occurred twice, wins rank by their distance, so only the fastest keep the top rank.

The conversion rows below are what separate the builds.

## Measurements

The self-play rows play one binary against itself at 50 ms per move, with `--candidate-syzygy-path`
giving the tables to one side, colours reversed within each pair.

| Check | Result |
| --- | --- |
| Search without tables | Node-for-node the parent's: 24,857,646, 26,995,867 and 49,930,907 nodes to depth 13 over 40 opening and middlegame positions at Aggression 0, 75 and 100, and the same 26,995,867 at 75 with the five-man tables loaded |
| Throughput with tables loaded | -0.24% at 16 MiB and -0.43% at 64 MiB, median of five alternating runs |
| Keeping a won position: 731 positions from the games where Jakgro was to move in a tablebase win that fit the fifty-move rule, 20,000 nodes | Without tables 656 kept (89.7%; 93.3%, 81.8% and 56.5% at halfmove clock below 40, 40 to 79 and 80 or more); with tables 731 (100%) |
| Converting: 800 positions from the games where the side to move has a tablebase win, 1,600 games | The side with tables won all 800; the same engine without tables won 513 and drew 287; the build before the two changes above won 736 and drew 64 |
| Self-play from won positions | +63.0 [+57.1, +69.0] (+49.6 [+43.4, +55.9] before the two changes) |
| Self-play from endgame entries: 864 positions with six to eight pieces from the games, 1,728 games | +39.8 [+34.5, +45.1] |
| Self-play from ordinary openings: 4,096 games from the book, and two more samples of 4,096 with four quiet random plies added to each opening | +3.9 [+0.7, +7.2] over the 12,288 games (+1.9, +3.3 and +6.6 in the three samples) |
| Probing inside quiescence as well, both sides with tables | +4.5 [-1.5, +10.5] from ordinary openings and +7.2 [+2.8, +11.7] from endgame entries, measured before the two changes |

## Gauntlet

The 1,000 games at 30+0.3 that exposed the problem were played again with the same openings, seed and
opponents, with tables given to Jakgro alone, and each run was audited against the tablebases. The
tables-off run was played by the build before the NNUE speed series, the second by the first version of
this change and the third by the version this report describes.

| 1,000 games at 30+0.3 | Tables off | Tables on, first build | Tables on, final build |
| --- | ---: | ---: | ---: |
| Score | 54.5% | 55.7% | 54.9% |
| Games that reached five pieces or fewer | 326 | 231 | 220 |
| Jakgro's moves there that changed the theoretical result | 111 of 18,978 | 0 of 3,840 | 0 of 3,276 |
| Games with a tablebase-won position (won, drawn) | 94 (42, 52) | 41 (40, 1) | 60 (60, 0) |
| Points not converted | 26.0 (2.6% of the games, about 18 Elo) | 0.5 | 0.0 |
| Games in which Jakgro stood in a tablebase-lost ending | 39 (37 lost) | 0 | 0 |
| Mean game length in plies | 202 | 186 | 178 |

The scores are inside the noise of a 1,000-game run against a ladder, about 20 Elo either way, and
separate none of the runs. The audit does: with tables Jakgro converts every won ending it reaches, never
changes a result by its own move, and never enters a lost ending. Games with tables are shorter because a
drawn ending is now reported as one: 154 and 175 draws were adjudicated against 125, and 106 and 78 games
were played out to the fifty-move rule or insufficient material against 145. The evaluation's +2.5 used to
keep those games going, and in that run the opponents turned a drawn ending into a lost one 144 times, a
gift that skill-limited opponents make more readily than full-strength engines would.

## Limits

- Only the three-, four- and five-man tables have been exercised. The library and the option ranges
  reach seven pieces, but nothing larger was tested, and the six-man files are 160 GB that a cold search
  would have to page in.
- The tablebases are consulted only where the halfmove clock is zero. A stored tablebase result carries no
  clock, as no result in the table does; the root ranking, which does see the clock, is what keeps a win
  inside the fifty-move rule.
- Without distance-to-zero files the root avoids moves that change the result but does not steer toward a
  zeroing move, so a long win can still run out the rule.
- `tbhits` counts the positions the main searcher settled and omits helper threads.
- The audit and the win-keeping check used python-chess to read the same files, which is not a dependency
  of this repository, so those scripts are not in the tree.

## Reproduction

The tables come from `https://tablebase.lichess.ovh/tables/standard/3-4-5-wdl/` and
`https://tablebase.lichess.ovh/tables/standard/3-4-5-dtz/`. The self-play rows:

```sh
cargo build --release --locked --bin jakgro --bin selfplay
python3 tools/run_sprt.py \
  --engine target/release/jakgro --baseline-engine target/release/jakgro \
  --candidate-syzygy-path ~/syzygy/3-4-5 \
  --games 4096 --movetime-ms 50 --concurrency 88 \
  --elo0 0 --elo1 5 \
  --openings docs/tuning/data/selective-search-confirmation.epd \
  --pgn artifacts/syzygy-50ms.pgn
```

The gauntlet passes `option.SyzygyPath=$HOME/syzygy/3-4-5` to Jakgro alone.
