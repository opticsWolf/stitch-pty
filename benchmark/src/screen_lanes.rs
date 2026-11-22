//! Screen-state lanes from `docs/TESTS.md`.
//!
//! These lanes feed `HistoryScreen` directly (no PTY, daemon, or client) so
//! the cost of screen-state maintenance can be attributed by operation.

use std::hint::black_box;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use stitch_pty::terminal::HistoryScreen;
use stitch_pty::terminal::screen::screen_profile;

use crate::{alloc_stats, percentile, reset_alloc_stats};

const MIB: f64 = 1024.0 * 1024.0;

pub struct LaneResult {
    name: String,
    unit: &'static str,
    alloc_unit: &'static str,
    note: &'static str,
    config: Value,
    values: Vec<f64>,
    allocs: Vec<f64>,
    bytes: Vec<f64>,
}

impl LaneResult {
    pub fn json(&self) -> Value {
        let mut sorted = self.values.clone();
        sorted.sort_by(f64::total_cmp);
        let mut allocs = self.allocs.clone();
        allocs.sort_by(f64::total_cmp);
        let mut bytes = self.bytes.clone();
        bytes.sort_by(f64::total_cmp);
        json!({
            "metric": self.name,
            "unit": self.unit,
            "note": self.note,
            "config": self.config,
            "count": self.values.len(),
            "p50": percentile(&sorted, 0.50),
            "p95": percentile(&sorted, 0.95),
            "min": sorted.first().copied().unwrap_or(0.0),
            "max": sorted.last().copied().unwrap_or(0.0),
            "samples": self.values,
            "allocations": {
                "unit": self.alloc_unit,
                "p50": percentile(&allocs, 0.50),
                "max": allocs.last().copied().unwrap_or(0.0),
            },
            "allocated_bytes": {
                "unit": "bytes requested per allocation unit above",
                "p50": percentile(&bytes, 0.50),
                "max": bytes.last().copied().unwrap_or(0.0),
            },
        })
    }
}

fn repeat(unit: &[u8], bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes + unit.len());
    while out.len() < bytes {
        out.extend_from_slice(unit);
    }
    out
}

fn cursor_erase_payload(bytes: usize) -> Vec<u8> {
    repeat(
        b"\x1b[H\x1b[2J\x1b[12;40H cursor-move-test-row-column \x1b[1;5H\x1b[K\r\n",
        bytes,
    )
}

fn numbered_rows(bytes: usize) -> Vec<u8> {
    let line = format!("{:08} {}\r\n", 0, "x".repeat(62));
    repeat(line.as_bytes(), bytes)
}

fn feed_all(screen: &mut HistoryScreen, data: &[u8], feed: usize) {
    for part in data.chunks(feed.max(1)) {
        screen.feed(part);
    }
}

fn check_shape(screen: &HistoryScreen, cap: usize) -> Result<(), String> {
    let columns = screen.columns();
    let lines = screen.lines();
    if screen.buffer().len() != lines {
        return Err(format!("expected {lines} visible rows, got {}", screen.buffer().len()));
    }
    for (index, row) in screen.buffer().iter().enumerate() {
        if row.len() != columns {
            return Err(format!("row {index} has {} cells, expected {columns}", row.len()));
        }
    }
    if cap > 0 && screen.history_size() > cap {
        return Err(format!("history {} exceeds cap {cap}", screen.history_size()));
    }
    let (x, y) = screen.absolute_cursor();
    if x > columns || y >= screen.total_lines() {
        return Err(format!("cursor ({x},{y}) outside {}x{}", columns, screen.total_lines()));
    }
    Ok(())
}

fn whole_vs_chunked(
    make: &impl Fn() -> HistoryScreen,
    data: &[u8],
    feed: usize,
    cap: usize,
) -> Result<(), String> {
    let mut whole = make();
    feed_all(&mut whole, data, data.len());
    check_shape(&whole, cap)?;
    let mut chunked = make();
    feed_all(&mut chunked, data, feed);
    check_shape(&chunked, cap)?;
    if whole.display() != chunked.display() {
        return Err("whole-stream and chunked displays differ".to_owned());
    }
    Ok(())
}

fn measure<S, T, F>(
    name: &str,
    unit: &'static str,
    note: &'static str,
    config: Value,
    samples: usize,
    warmup: usize,
    alloc_unit: &'static str,
    alloc_divisor: f64,
    mut setup: S,
    mut work: F,
) -> LaneResult
where
    S: FnMut() -> T,
    F: FnMut(&mut T) -> (f64, Duration),
{
    for _ in 0..warmup {
        let mut state = setup();
        let (value, _) = work(&mut state);
        black_box(value);
    }
    let mut result = LaneResult {
        name: name.to_owned(),
        unit,
        alloc_unit,
        note,
        config,
        values: Vec::with_capacity(samples),
        allocs: Vec::with_capacity(samples),
        bytes: Vec::with_capacity(samples),
    };
    for _ in 0..samples {
        let mut state = setup();
        reset_alloc_stats();
        let (value, elapsed) = work(&mut state);
        let (allocations, allocated) = alloc_stats();
        black_box(&mut state);
        result.values.push(value);
        let divisor = alloc_divisor.max(f64::EPSILON);
        result.allocs.push(allocations as f64 / divisor);
        result.bytes.push(allocated as f64 / divisor);
        black_box(elapsed);
    }
    result
}

fn bytes_per_second(data_len: usize, elapsed: Duration) -> f64 {
    (data_len as f64 / 1_000_000.0) / elapsed.as_secs_f64().max(f64::EPSILON)
}

fn ops_per_second(ops: usize, elapsed: Duration) -> f64 {
    ops as f64 / elapsed.as_secs_f64().max(f64::EPSILON)
}

fn feed_lane(
    name: String,
    note: &'static str,
    config: Value,
    samples: usize,
    warmup: usize,
    data: &[u8],
    feed: usize,
    make: impl Fn() -> HistoryScreen,
    cap: usize,
    failures: &mut Vec<String>,
) -> LaneResult {
    if let Err(error) = whole_vs_chunked(&make, data, feed, cap) {
        failures.push(format!("{name}: {error}"));
    }
    let data = data.to_vec();
    let failure_name = name.clone();
    measure(
        &name,
        "MB/s",
        note,
        config,
        samples,
        warmup,
        "allocations per MiB fed",
        data.len() as f64 / MIB,
        move || make(),
        move |screen| {
            let start = Instant::now();
            feed_all(screen, &data, feed);
            let elapsed = start.elapsed();
            if let Err(error) = check_shape(screen, cap) {
                failures.push(format!("{failure_name}: {error}"));
            }
            (bytes_per_second(data.len(), elapsed), elapsed)
        },
    )
}

pub fn run(
    samples: usize,
    warmup: usize,
    bytes: usize,
    feeds: &[usize],
    failures: &mut Vec<String>,
) -> Vec<LaneResult> {
    let mut results = Vec::new();

    // Draw without scroll: cursor is homed each iteration, so history stays empty.
    let draw_payloads = [
        (
            "draw_ascii_overwrite",
            repeat(format!("\x1b[H{}\r", "x".repeat(72)).as_bytes(), bytes),
        ),
        (
            "draw_sgr_runs",
            repeat(
                format!(
                    "\x1b[H\x1b[31m{}\x1b[32m{}\x1b[0m\r",
                    "r".repeat(36),
                    "g".repeat(36)
                )
                .as_bytes(),
                bytes,
            ),
        ),
        (
            "draw_mixed_width",
            repeat("\x1b[H日本語テスト e\u{0301} 日本語テスト\r".as_bytes(), bytes),
        ),
    ];
    for feed in feeds {
        for (name, payload) in &draw_payloads {
            results.push(feed_lane(
                format!("{name}_feed{feed}"),
                "printable draws at cursor home; no scroll",
                json!({"geometry": "80x24", "scrollback": 0, "feed_bytes": feed}),
                samples,
                warmup,
                payload,
                *feed,
                || HistoryScreen::new(80, 24, 0),
                0,
                failures,
            ));
        }
    }

    // Scroll and retain.
    let scroll_payload = numbered_rows(bytes);
    let geometries = [
        (80, 24, 100),
        (80, 24, 1_000),
        (80, 24, 20_000),
        (80, 48, 2_000),
        (200, 60, 2_000),
    ];
    for feed in feeds {
        for (columns, lines, cap) in geometries {
            results.push(feed_lane(
                format!("scroll_numbered_{columns}x{lines}_cap{cap}_feed{feed}"),
                "numbered ASCII rows; history retention",
                json!({"geometry": format!("{columns}x{lines}"), "scrollback": cap, "feed_bytes": feed}),
                samples,
                warmup,
                &scroll_payload,
                *feed,
                || HistoryScreen::new(columns, lines, cap),
                cap,
                failures,
            ));
        }
    }
    let largest_feed = *feeds.iter().max().unwrap_or(&65_536);
    let wrap_payload = repeat(b"w", bytes);
    results.push(feed_lane(
        "scroll_soft_wrap".to_owned(),
        "unbroken printable run; soft wrapping",
        json!({"geometry": "80x24", "scrollback": 2_000, "feed_bytes": largest_feed}),
        samples,
        warmup,
        &wrap_payload,
        largest_feed,
        || HistoryScreen::new(80, 24, 2_000),
        2_000,
        failures,
    ));
    let alt_payload = {
        let mut payload = b"\x1b[?1049h".to_vec();
        payload.extend_from_slice(&scroll_payload);
        payload.extend_from_slice(b"\x1b[?1049l");
        payload
    };
    results.push(feed_lane(
        "scroll_alt_screen".to_owned(),
        "alternate screen; primary history is restored",
        json!({"geometry": "80x24", "scrollback": 2_000, "feed_bytes": largest_feed}),
        samples,
        warmup,
        &alt_payload,
        largest_feed,
        || HistoryScreen::new(80, 24, 2_000),
        2_000,
        failures,
    ));
    let region_payload = {
        let mut payload = b"\x1b[5;20r".to_vec();
        payload.extend_from_slice(&scroll_payload);
        payload
    };
    results.push(feed_lane(
        "scroll_partial_region".to_owned(),
        "scroll margins 5..20",
        json!({"geometry": "80x24", "scrollback": 2_000, "feed_bytes": largest_feed}),
        samples,
        warmup,
        &region_payload,
        largest_feed,
        || HistoryScreen::new(80, 24, 2_000),
        2_000,
        failures,
    ));

    // Erase and repaint: report operations per second, plus allocation counts.
    let ech_payload = format!("{}\r\x1b[X", "abcdefghij".repeat(8));
    let erase_payloads = [
        ("erase_display", b"\x1b[2J".as_slice(), 1),
        ("erase_line", b"\x1b[2K".as_slice(), 1),
        ("erase_characters", ech_payload.as_bytes(), 1),
        ("cursor_only", b"\x1b[5;20H".as_slice(), 1),
        ("reverse_index", b"\x1bM".as_slice(), 1),
        (
            "styled_redraw",
            b"\x1b[H\x1b[31mrrrr\x1b[0m".as_slice(),
            1,
        ),
    ];
    for (name, unit, ops_per_unit) in erase_payloads {
        let lane_failures = &mut *failures;
        let payload = repeat(unit, bytes);
        let ops = payload.len() / unit.len() * ops_per_unit;
        let unit_len = unit.len();
        results.push(measure(
            name,
            "ops/s",
            "erase/cursor/repaint operation throughput",
            json!({"geometry": "80x24", "scrollback": 1_000, "bytes": bytes, "sequence_bytes": unit_len, "operations": ops}),
            samples,
            warmup,
            "allocations per operation",
            ops as f64,
            || HistoryScreen::new(80, 24, 1_000),
            move |screen| {
                let start = Instant::now();
                feed_all(screen, &payload, largest_feed);
                let elapsed = start.elapsed();
                if let Err(error) = check_shape(screen, 1_000) {
                    lane_failures.push(format!("{name}: {error}"));
                }
                (ops_per_second(ops, elapsed), elapsed)
            },
        ));
    }

    let cursor_payload = cursor_erase_payload(bytes);
    results.push(feed_lane(
        "erase_cursor_mixed".to_owned(),
        "home + ED2 + cursor move + print + EL0 loop",
        json!({"geometry": "80x24", "scrollback": 1_000, "feed_bytes": largest_feed}),
        samples,
        warmup,
        &cursor_payload,
        largest_feed,
        || HistoryScreen::new(80, 24, 1_000),
        1_000,
        failures,
    ));

    // Resize and project. Setup (including the initial feed) is outside the timer.
    let project_payload = numbered_rows(bytes);
    let make_projected = || {
        let mut screen = HistoryScreen::new(80, 24, 20_000);
        feed_all(&mut screen, &project_payload, largest_feed);
        screen
    };
    let reflow_cycles = 8;
    let reflow_ops = reflow_cycles * 2;
    results.push(measure(
        "resize_reflow",
        "ops/s",
        "steady-state alternating 80->60->80 reflow; 8 cycles amortize the first fill of the blank-cell pool",
        json!({"bytes": bytes, "history_rows": "up to 20000", "cycles": reflow_cycles, "resizes_per_sample": reflow_ops}),
        samples,
        warmup,
        "allocations per resize",
        reflow_ops as f64,
        make_projected,
        |screen| {
            let start = Instant::now();
            for _ in 0..reflow_cycles {
                screen.resize(24, 60);
                screen.resize(24, 80);
            }
            let elapsed = start.elapsed();
            (ops_per_second(reflow_ops, elapsed), elapsed)
        },
    ));
    results.push(measure(
        "project_styled_range",
        "ops/s",
        "borrowed viewport projection of 24 styled rows",
        json!({"bytes": bytes, "rows": 24}),
        samples,
        warmup,
        "allocations per operation",
        1.0,
        make_projected,
        |screen| {
            let start = Instant::now();
            let rows = screen.styled_range(0, 24);
            let elapsed = start.elapsed();
            black_box(rows);
            (ops_per_second(1, elapsed), elapsed)
        },
    ));
    results.push(measure(
        "project_styled_viewport",
        "ops/s",
        "full history + visible styled projection",
        json!({"bytes": bytes}),
        samples,
        warmup,
        "allocations per operation",
        1.0,
        make_projected,
        |screen| {
            let start = Instant::now();
            let rows = screen.styled_viewport();
            let elapsed = start.elapsed();
            black_box(rows);
            (ops_per_second(1, elapsed), elapsed)
        },
    ));
    results.push(measure(
        "project_display",
        "ops/s",
        "plain-text serialization of all rows",
        json!({"bytes": bytes}),
        samples,
        warmup,
        "allocations per operation",
        1.0,
        make_projected,
        |screen| {
            let start = Instant::now();
            let rows = screen.display();
            let elapsed = start.elapsed();
            black_box(rows);
            (ops_per_second(1, elapsed), elapsed)
        },
    ));

    results
}

pub fn profile(bytes: usize, feed: usize) -> Value {
    let data = numbered_rows(bytes);
    let mut screen = HistoryScreen::new(80, 24, 20_000);
    screen_profile::reset();
    screen_profile::set_enabled(true);
    feed_all(&mut screen, &data, feed);
    screen_profile::set_enabled(false);
    let stats = screen_profile::snapshot();
    let mib = data.len() as f64 / MIB;
    let other_ns = stats.feed_ns.saturating_sub(stats.scroll_ns);
    let total = stats.feed_ns.max(1);
    let pct = |ns: u64| ns as f64 / total as f64;
    json!({
        "note": "single instrumented pass; percentages are indicative and include timer overhead",
        "feed_bytes": data.len(),
        "feed_bytes_per_chunk": feed,
        "feed_ns": stats.feed_ns,
        "scroll_ns": stats.scroll_ns,
        "scroll_erase_ns": stats.scroll_erase_ns,
        "collect_ns": stats.collect_ns,
        "other_ns": other_ns,
        "scroll_pct": pct(stats.scroll_ns),
        "scroll_erase_pct": pct(stats.scroll_erase_ns),
        "collect_pct": pct(stats.collect_ns),
        "other_pct": pct(other_ns),
        "scroll_rows": stats.scroll_rows,
        "draw_cells": stats.draw_cells,
        "collect_rows": stats.collect_rows,
        "feed_ns_per_mib": stats.feed_ns as f64 / mib,
        "scroll_ns_per_row": stats.scroll_rows.checked_sub(0).map_or(0.0, |rows| {
            if rows == 0 { 0.0 } else { stats.scroll_ns as f64 / rows as f64 }
        }),
        "draw_ns_per_cell": if stats.draw_cells == 0 { 0.0 } else { other_ns as f64 / stats.draw_cells as f64 },
        "history_rows": screen.history_size(),
    })
}

pub fn cursor_erase_profile(bytes: usize, feed: usize) -> Value {
    let data = cursor_erase_payload(bytes);
    let mut screen = HistoryScreen::new(80, 24, 1_000);
    screen_profile::reset();
    screen_profile::set_enabled(true);
    feed_all(&mut screen, &data, feed);
    screen_profile::set_enabled(false);
    let stats = screen_profile::snapshot();
    let mib = data.len() as f64 / MIB;
    let total = stats.feed_ns.max(1);
    let pct = |ns: u64| ns as f64 / total as f64;
    json!({
        "note": "single instrumented pass over the stitch-mux cursor payload; percentages include timer overhead",
        "feed_bytes": data.len(),
        "feed_bytes_per_chunk": feed,
        "feed_ns": stats.feed_ns,
        "erase_display_ns": stats.erase_display_ns,
        "erase_line_ns": stats.erase_line_ns,
        "cursor_ns": stats.cursor_ns,
        "erase_cells": stats.erase_cells,
        "erase_ops": stats.erase_ops,
        "draw_cells": stats.draw_cells,
        "erase_display_pct": pct(stats.erase_display_ns),
        "erase_line_pct": pct(stats.erase_line_ns),
        "cursor_pct": pct(stats.cursor_ns),
        "remaining_pct": pct(total.saturating_sub(stats.erase_display_ns + stats.erase_line_ns + stats.cursor_ns)),
        "erase_ns_per_cell": if stats.erase_cells == 0 { 0.0 } else { (stats.erase_display_ns + stats.erase_line_ns) as f64 / stats.erase_cells as f64 },
        "feed_ns_per_mib": stats.feed_ns as f64 / mib,
    })
}
