# Verified aggression: style and strength result

This report validates the engine series ending at `67cc981f2e4c6b66222e6716e3dcbe9dc0cb4c9f` against the frozen pre-series baseline `329e8de7d7a5410588441e53006d8c9a64f32722`. Both binaries used `Aggression=100`; this is an old-versus-new default-profile comparison, not a comparison between personality endpoints.

The machine-readable match result is stored in [`data/verified-aggression-elo.summary.json`](data/verified-aggression-elo.summary.json), and the fixed-node and benchmark evidence is stored in [`data/verified-aggression-style.summary.json`](data/verified-aggression-style.summary.json).

## Acceptance result

| Gate | Result |
| --- | --- |
| Rust, Python, formatting, and clippy checks | Pass |
| Search and personality regression suites | Pass |
| Candidate expected fixed-node choices | 16/16 personality and 4/4 sacrifice/control positions |
| Safety and anti-sacrifice regressions | 0 |
| New sacrifice-suite hits over the old binary | 0; improvement gate not established |
| Same-profile Elo lower bound above 0 | Pass |
| Same-profile color balance | Identical 84.375% candidate score as White and Black |

The accepted claims are therefore deliberately narrow: the new default profile is much stronger than the old default profile at this node limit, produces substantially more forcing-play proxies in their paired games, and preserves the reviewed sacrifice and safety choices. The frozen suite does **not** demonstrate an increased sacrifice hit rate, so this report does not claim that sacrifice frequency itself increased.

## Same-profile strength result

The candidate and baseline played 96 games from 48 sequential, color-reversed opening pairs at 10,000 nodes per move:

- candidate W/D/L: **68/26/2**;
- candidate score: **84.375%**;
- approximate Elo: **+293.0**;
- 95% Hoeffding score bound: **64.77% to 100.00%**;
- transformed Elo lower bound: **+105.8**;
- decisive games: **72.92%**;
- average game length: **50.16 plies**.

The predeclared lower-bound gate required the paired 95% score lower bound to exceed 50%. The observed lower bound was 64.77%, so the gate passed. This is fixed-node self-play evidence at one opening corpus and one node limit, not an external rating or an SPRT result.

## Match style indicators

The analyzer's SAN-derived rates moved strongly toward forcing play in the same-profile old-versus-new match:

| Indicator per 100 moves | Candidate | Baseline | Change |
| --- | ---: | ---: | ---: |
| Checks | 15.92 | 9.07 | +75.5% |
| Captures | 30.33 | 19.92 | +52.3% |
| Promotions | 0.62 | 0.25 | +151.6% |
| Checks, captures, or promotions | 41.29 | 26.54 | +55.6% |

These are descriptive spectacle proxies. They do not establish move quality or identify deliberate material sacrifices, so they are reported alongside—not in place of—the strength and fixed-position gates.

## Fixed-node personality and sacrifice gates

At `Aggression=100`, the old and new binaries selected the same reviewed move in all 16 personality positions and all four frozen sacrifice/control positions. The candidate retained:

- the reviewed `Bxf7` sacrifice;
- both unsupported-sacrifice rejections;
- all forced-defense and tactical-safety moves; and
- all initiative, king-attack, pawn-storm, and simplification choices.

The candidate used fewer nodes on many positions while reaching more depth in several of them. The sacrifice improvement gate remained false because the old binary already hit the sole positive sacrifice position. Expanding the frozen corpus with independently reviewed pawn, exchange, clearance, and quiet sacrifices remains necessary before claiming broader sacrifice ability.

## Search benchmark evidence

Representative fixed-node benchmark rows show higher completed depth or lower wall time despite the more expensive legal exchange analysis:

| Position | Baseline depth/time | Candidate depth/time |
| --- | ---: | ---: |
| Win hanging queen | 1 / 7 ms | 4 / 8 ms |
| Punish central queen | 1 / 32 ms | 3 / 30 ms |
| Developed open game | 1 / 50 ms | 3 / 43 ms |
| Starting style | 4 / 52 ms | 4 / 47 ms |
| Open-game style | 3 / 52 ms | 3 / 43 ms |

Raw NPS is lower in some rows because legal SEE and sacrifice settlement make individual nodes more expensive. Completed depth, deterministic move quality, wall time, and the paired match are the relevant combined checks.

## Exploratory endpoint diagnostics

Two additional 96-game diagnostics were run with the same openings and node limit but were not acceptance gates:

- new `Aggression=0` versus old `Aggression=0`: 36/23/37, 49.48%, approximately -3.6 Elo with a wide interval crossing zero;
- new `Aggression=100` versus new `Aggression=0`: 7/4/85, 9.38%, approximately -394 Elo.

The second result means `Aggression=100` is emphatically not the strongest setting in the new binary. The confirmed Elo claim in this report is only that the **new default aggressive profile beats the old default aggressive profile**. Future work should reduce this within-build personality cost without weakening the forcing-play and safety gates.

## Reproducibility

The accepted same-profile match used:

- candidate commit: `67cc981f2e4c6b66222e6716e3dcbe9dc0cb4c9f`;
- baseline commit: `329e8de7d7a5410588441e53006d8c9a64f32722`;
- candidate binary SHA-256: `2b85f0e19c88880955cbab7e7d6ee1d72fe02413dac7542658f4e25187e2801d`;
- baseline binary SHA-256: `d2092e6ab3354d04af83925a2c14ed0111a14fa9e524b4fe4825b35cfea5d656`;
- opening SHA-256: `8f67f7bdb3c659140516e9f692694ca8513633ee0d9302374dccef927eaa0cde`;
- match-runner SHA-256: `fa87139b72a2ee1b7972c3af540a51b17abc0557b198158dde48edb90d9999b7`;
- `cutechess-cli 1.5.1`, binary SHA-256 `bb8ec8df71ce0ef95ec03614440fe93c31730bbc0c8fbfd07a535e14b7b5d550`;
- 16 MiB hash, one concurrent game, 10,000 nodes per move, 48 rounds and 96 games;
- PGN SHA-256: `5175863f9fa2afd791c2cea8fa5cc93068e7fbe26f88236573b72c6514a38aeb`;
- manifest SHA-256: `6248fd6917eb66201c01500ac5834bd96b509f56797c83a203169fc9589d5c1f`.

The analyzer was invoked with `--min-elo-lower-bound 0`; it wrote the summary before returning success. Every manifest input hash was rechecked after match execution.
