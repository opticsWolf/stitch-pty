# Parser and screen benchmark

Implements the **Native parser** and screen-state lanes from `docs/TESTS.md`.

Parser lanes feed raw bytes into `terminal::ansi_parser::Parser` with counting
callbacks; `vte` runs the same payloads as the candidate. Screen lanes feed
`HistoryScreen` directly, with no PTY, daemon, or client, so screen-state cost
can be attributed by operation. Setup, parity checks, and teardown stay
outside the timer.

## Run

```sh
cargo run --release --manifest-path benchmark/Cargo.toml -- \
  --samples 30 --warmup 5 --bytes 1048576 \
  --out benchmark/results/parser.json
```

Options: `--samples`, `--warmup`, `--bytes`, `--feeds A,B,C`, `--out`, `--help`.
Default feed sizes are 1 KiB, 16 KiB, and 64 KiB. Screen lanes use at most
64 KiB of payload and at most 10 samples to keep the suite short; both limits
are recorded in `config`.

## Payload families (parser lanes)

- `ascii` — fixed-width numbered rows
- `unicode` — CJK plus combining marks
- `csi` — SGR and cursor movement
- `images` — synthetic Kitty APC string data; skipped, not decoded

## Screen lanes

`screen_results` contains:

- **draw** — ASCII overwrites, alternating SGR runs, mixed-width text, all at
  cursor home so no scroll occurs;
- **scroll** — numbered rows at 80x24 / 80x48 / 200x60 with caps 100–20,000,
  plus soft wrap, alternate screen, and a 5..20 scroll region;
- **erase** — ED, EL, ECH, cursor-only moves, reverse index, and styled
  redraws, reported in ops/s;
- **resize/project** — reflow, `styled_range`, `styled_viewport`, and plain
  `display()` serialization.

Every feed lane compares whole-stream and chunked output before timing and
checks buffer shape, history cap, and cursor bounds after each sample.

## Allocator instrumentation

The binary installs a counting global allocator. Before every timed sample it
resets the counters and records allocations and requested bytes, reported as
`allocations` and `allocated_bytes` with an explicit `unit` per lane. Parser
lanes report zero allocations. Screen lanes expose per-operation cost; for
example, an 80x24 capped history with cap 100 drops from ~3.37M to ~0.50M
allocations per MiB thanks to the evicted-row pool. The steady-state
`resize_reflow` lane (eight alternating cycles) dropped from ~26M allocations
per MiB in the original reflow to ~24.5k allocations per resize after row-owner
moves and blank-cell recycling.

## Phase profiles

The dependency enables stitch-pty's `bench-profile` feature for two extra
instrumented passes:

- `ascii_ground_profile` — ESC scan, UTF-8 validation, ground dispatch
- `screen_ground_profile` — parser feed, `Screen::scroll_up`, blank-row erase,
  scrollback collection, and remaining draw/other work
- `cursor_erase_profile` — the stitch-mux cursor workload, split into ED2,
  EL, cursor movement, and remaining work

Timers add overhead, so use the percentages as a guide rather than as absolute
cost. The screen profile attributes ~86% of feed time to scroll-up erase.

## Correctness gate

Before timing, each parser family is parsed as a whole stream, byte at a time,
and at several chunk sizes. The native parser's counts must match in every
case; a mismatch fails the lane. Candidate (`vte`) mismatches are recorded in
`correctness.candidate_notes` but do not block the native numbers.

This gate caught a real bug: the native parser dropped characters when a feed
boundary split a multibyte UTF-8 sequence. Fixed in `ansi_parser.rs` with a
regression test; `vte` still shows the same split-boundary mismatch and is
labelled as an external note.

## Report

`benchmark/results/parser.json` records host/profile, sample counts, feed
sizes, p50/p95/min/max, raw samples, correctness failures, candidate notes,
both phase profiles, `results` (parser), and `screen_results`. Parser MB/s is
decimal (1 MB = 1,000,000 bytes), matching kitty's published convention;
screen feed lanes use the same unit, while erase/resize/project lanes use
ops/s.
