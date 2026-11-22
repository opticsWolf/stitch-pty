//! Parser and screen throughput harness for stitch-pty.
//!
//! Implements the "Native parser" lane from `docs/TESTS.md` with raw bytes
//! through `terminal::ansi_parser::Parser` and a `vte` comparison candidate,
//! plus direct `HistoryScreen` lanes for draw, scroll, erase, resize, and
//! projection. Setup, teardown, and parity checks stay outside the timer.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::json;

mod screen_lanes;
use stitch_pty::terminal::ansi_parser::{
    Params as NativeParams, Perform as NativePerformTrait, Parser as NativeParser, bench_profile,
};
use vte::{Params as VteParams, Perform as VtePerformTrait, Parser as VteParser};

struct CountingAllocator;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        ALLOCATED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        ALLOCATED_BYTES.fetch_add(new_size, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

pub(crate) fn reset_alloc_stats() {
    ALLOCATIONS.store(0, Ordering::Relaxed);
    ALLOCATED_BYTES.store(0, Ordering::Relaxed);
}

pub(crate) fn alloc_stats() -> (usize, usize) {
    (
        ALLOCATIONS.load(Ordering::Relaxed),
        ALLOCATED_BYTES.load(Ordering::Relaxed),
    )
}

pub(crate) fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Counts {
    printed: u64,
    control: u64,
    csi: u64,
    esc: u64,
    osc: u64,
    string_bytes: u64,
}

impl Counts {
    fn total(self) -> u64 {
        self.printed + self.control + self.csi + self.esc + self.osc + self.string_bytes
    }
}

#[derive(Default)]
struct NativePerform(Counts);

impl NativePerformTrait for NativePerform {
    fn print(&mut self, c: char) {
        self.0.printed += 1;
        black_box(c);
    }

    fn execute(&mut self, byte: u8) {
        self.0.control += 1;
        black_box(byte);
    }

    fn put(&mut self, byte: u8) {
        self.0.string_bytes += 1;
        black_box(byte);
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        self.0.osc += 1;
    }

    fn csi_dispatch(
        &mut self,
        _params: &NativeParams,
        _intermediates: &[u8],
        _ignore: bool,
        _action: char,
    ) {
        self.0.csi += 1;
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {
        self.0.esc += 1;
    }
}

#[derive(Default)]
struct VtePerform(Counts);

impl VtePerformTrait for VtePerform {
    fn print(&mut self, c: char) {
        self.0.printed += 1;
        black_box(c);
    }

    fn execute(&mut self, byte: u8) {
        self.0.control += 1;
        black_box(byte);
    }

    fn put(&mut self, byte: u8) {
        self.0.string_bytes += 1;
        black_box(byte);
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        self.0.osc += 1;
    }

    fn csi_dispatch(
        &mut self,
        _params: &VteParams,
        _intermediates: &[u8],
        _ignore: bool,
        _action: char,
    ) {
        self.0.csi += 1;
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {
        self.0.esc += 1;
    }
}

#[derive(Clone, Copy)]
enum ParserKind {
    Native,
    Vte,
}

impl ParserKind {
    fn name(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Vte => "vte",
        }
    }
}

#[derive(Clone, Copy)]
enum Payload {
    Ascii,
    Unicode,
    Csi,
    Images,
}

impl Payload {
    fn name(self) -> &'static str {
        match self {
            Self::Ascii => "ascii",
            Self::Unicode => "unicode",
            Self::Csi => "csi",
            Self::Images => "images",
        }
    }

    fn note(self) -> &'static str {
        match self {
            Self::Images => "synthetic Kitty APC string data; skipped, not decoded",
            _ => "parsed normally",
        }
    }

    fn chunk(self) -> String {
        match self {
            Self::Ascii => {
                "00000000 xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\r\n"
                    .to_owned()
            }
            Self::Unicode => {
                "00000000 日本語テスト combining: e\u{0301} 日本語テスト\r\n".to_owned()
            }
            Self::Csi => {
                "\x1b[1;31m00000000\x1b[0m \x1b[38;2;1;2;3mrgb\x1b[0m \x1b[5;20Hcursor\r\n"
                    .to_owned()
            }
            Self::Images => format!(
                "\x1b_Ga=T,f=100,s=100,v=100,r=1,c=1;{}\x1b\\",
                "A".repeat(4_096)
            ),
        }
    }
}

fn payload_bytes(kind: Payload, bytes: usize) -> Vec<u8> {
    let chunk = kind.chunk();
    let mut payload = Vec::with_capacity(bytes + chunk.len());
    while payload.len() < bytes {
        payload.extend_from_slice(chunk.as_bytes());
    }
    payload
}

fn run_native(payload: &[u8], feed: usize) -> Counts {
    let mut parser = NativeParser::new();
    let mut performer = NativePerform::default();
    for part in payload.chunks(feed.max(1)) {
        parser.advance(&mut performer, part);
    }
    performer.0
}

fn run_vte(payload: &[u8], feed: usize) -> Counts {
    let mut parser = VteParser::new();
    let mut performer = VtePerform::default();
    for part in payload.chunks(feed.max(1)) {
        parser.advance(&mut performer, part);
    }
    performer.0
}

fn run(kind: ParserKind, payload: &[u8], feed: usize) -> Counts {
    match kind {
        ParserKind::Native => run_native(payload, feed),
        ParserKind::Vte => run_vte(payload, feed),
    }
}

#[derive(Clone)]
struct Sample {
    parser: ParserKind,
    payload: Payload,
    feed: usize,
    mb_per_s: Vec<f64>,
    allocs_per_mib: Vec<f64>,
    bytes_per_mib: Vec<f64>,
}

impl Sample {
    fn name(&self) -> String {
        format!(
            "parser_{}_{}_feed{}",
            self.parser.name(),
            self.payload.name(),
            self.feed
        )
    }

    fn json(&self) -> serde_json::Value {
        let mut sorted = self.mb_per_s.clone();
        sorted.sort_by(f64::total_cmp);
        let mut allocs = self.allocs_per_mib.clone();
        allocs.sort_by(f64::total_cmp);
        let mut bytes = self.bytes_per_mib.clone();
        bytes.sort_by(f64::total_cmp);
        json!({
            "parser": self.parser.name(),
            "payload": self.payload.name(),
            "feed_bytes": self.feed,
            "note": self.payload.note(),
            "unit": "MB/s",
            "count": self.mb_per_s.len(),
            "p50": percentile(&sorted, 0.50),
            "p95": percentile(&sorted, 0.95),
            "min": sorted.first().copied().unwrap_or(0.0),
            "max": sorted.last().copied().unwrap_or(0.0),
            "samples": self.mb_per_s,
            "allocs_per_mib": {
                "unit": "allocations per MiB parsed",
                "p50": percentile(&allocs, 0.50),
                "max": allocs.last().copied().unwrap_or(0.0),
            },
            "allocated_bytes_per_mib": {
                "unit": "bytes requested per MiB parsed",
                "p50": percentile(&bytes, 0.50),
                "max": bytes.last().copied().unwrap_or(0.0),
            },
        })
    }
}

fn correctness_gate(
    payload: Payload,
    bytes: usize,
    failures: &mut Vec<String>,
    notes: &mut Vec<String>,
) -> bool {
    let data = payload_bytes(payload, bytes.min(8_192));
    let expected_native = run_native(&data, data.len());
    let expected_vte = run_vte(&data, data.len());
    let mut ok = true;
    for (kind, expected, hard_gate) in [
        (ParserKind::Native, expected_native, true),
        (ParserKind::Vte, expected_vte, false),
    ] {
        let split = run(kind, &data, 1);
        let mut mismatches = Vec::new();
        for feed in [1_usize, 7, 17, 31, 64, 1_024] {
            let chunked = run(kind, &data, feed);
            if chunked != expected {
                mismatches.push(format!("feed={feed} counts={chunked:?}"));
            }
        }
        if split != expected || !mismatches.is_empty() {
            let message = format!(
                "{} {}: split parity mismatch whole={:?} byte={:?} mismatches={}",
                kind.name(),
                payload.name(),
                expected,
                split,
                mismatches.join("; ")
            );
            if hard_gate {
                failures.push(message);
                ok = false;
            } else {
                notes.push(message);
            }
        }
        if expected.total() == 0 {
            let message = format!(
                "{} {}: parser produced no events",
                kind.name(),
                payload.name()
            );
            if hard_gate {
                failures.push(message);
                ok = false;
            } else {
                notes.push(message);
            }
        }
    }
    ok
}

fn measure(
    kind: ParserKind,
    payload: Payload,
    feed: usize,
    bytes: usize,
    warmup: usize,
    samples: usize,
) -> Sample {
    let data = payload_bytes(payload, bytes);
    let mut result = Sample {
        parser: kind,
        payload,
        feed,
        mb_per_s: Vec::with_capacity(samples),
        allocs_per_mib: Vec::with_capacity(samples),
        bytes_per_mib: Vec::with_capacity(samples),
    };
    for _ in 0..warmup {
        black_box(run(kind, &data, feed));
    }
    let mib = data.len() as f64 / (1024.0 * 1024.0);
    for _ in 0..samples {
        reset_alloc_stats();
        let start = Instant::now();
        let counts = run(kind, &data, feed);
        let elapsed = start.elapsed();
        let (allocations, allocated_bytes) = alloc_stats();
        black_box(counts);
        result.mb_per_s.push(
            (data.len() as f64 / 1_000_000.0) / elapsed.as_secs_f64().max(f64::EPSILON),
        );
        result.allocs_per_mib.push(allocations as f64 / mib);
        result.bytes_per_mib.push(allocated_bytes as f64 / mib);
    }
    result
}

fn profile_ascii(bytes: usize, feed: usize) -> serde_json::Value {
    let data = payload_bytes(Payload::Ascii, bytes);
    bench_profile::reset();
    bench_profile::set_enabled(true);
    black_box(run_native(&data, feed));
    bench_profile::set_enabled(false);
    let stats = bench_profile::snapshot();
    let mib = data.len() as f64 / (1024.0 * 1024.0);
    let total = stats.scan_ns + stats.utf8_ns + stats.dispatch_ns;
    let fraction = |ns: u64| -> f64 {
        if total == 0 {
            0.0
        } else {
            ns as f64 / total as f64
        }
    };
    json!({
        "note": "single instrumented pass; percentages are indicative, timings include timer overhead",
        "feed_bytes": feed,
        "calls": stats.calls,
        "total_ns": total,
        "scan_ns": stats.scan_ns,
        "utf8_ns": stats.utf8_ns,
        "dispatch_ns": stats.dispatch_ns,
        "scan_pct": fraction(stats.scan_ns),
        "utf8_pct": fraction(stats.utf8_ns),
        "dispatch_pct": fraction(stats.dispatch_ns),
        "scan_ns_per_mib": stats.scan_ns as f64 / mib,
        "utf8_ns_per_mib": stats.utf8_ns as f64 / mib,
        "dispatch_ns_per_mib": stats.dispatch_ns as f64 / mib,
    })
}

fn parse_args() -> (usize, usize, usize, Vec<usize>, std::path::PathBuf) {
    let mut samples = 30_usize;
    let mut warmup = 5_usize;
    let mut bytes = 1_048_576_usize;
    let mut feeds = vec![1_024_usize, 16_384, 65_536];
    let mut out = std::path::PathBuf::from("benchmark/results/parser.json");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--samples" => samples = args.next().expect("--samples").parse().unwrap(),
            "--warmup" => warmup = args.next().expect("--warmup").parse().unwrap(),
            "--bytes" => bytes = args.next().expect("--bytes").parse().unwrap(),
            "--feeds" => {
                feeds = args
                    .next()
                    .expect("--feeds")
                    .split(',')
                    .map(|v| v.trim().parse().expect("feed size"))
                    .collect();
            }
            "--out" => out = std::path::PathBuf::from(args.next().expect("--out")),
            "--help" => {
                println!(
                    "stitch-pty parser benchmark\n  --samples N   samples per lane (default 30)\n  --warmup N    warmups per lane (default 5)\n  --bytes N     payload bytes (default 1048576)\n  --feeds A,B,C feed chunk sizes (default 1024,16384,65536)\n  --out PATH    JSON report path\n"
                );
                std::process::exit(0);
            }
            other => panic!("unknown argument: {other}"),
        }
    }
    (samples, warmup, bytes, feeds, out)
}

fn main() {
    let (samples, warmup, bytes, feeds, out) = parse_args();
    let payloads = [Payload::Ascii, Payload::Unicode, Payload::Csi, Payload::Images];
    let kinds = [ParserKind::Native, ParserKind::Vte];
    let mut failures = Vec::new();
    let mut notes = Vec::new();
    let mut results = Vec::new();

    eprintln!(
        "benchmark: samples={samples} warmup={warmup} bytes={bytes} feeds={feeds:?}"
    );

    for payload in payloads {
        if !correctness_gate(payload, bytes, &mut failures, &mut notes) {
            eprintln!("correctness gate failed for {}", payload.name());
        }
        for feed in &feeds {
            // Alternate baseline and candidate per payload/feed to limit drift.
            for kind in kinds {
                results.push(measure(kind, payload, *feed, bytes, warmup, samples));
            }
        }
    }

    let largest_feed = *feeds.iter().max().unwrap_or(&65_536);
    let screen_bytes = bytes.min(65_536);
    let screen_samples = samples.min(10);
    let mut screen_failures = Vec::new();
    let screen_results = screen_lanes::run(
        screen_samples,
        warmup,
        screen_bytes,
        &feeds,
        &mut screen_failures,
    );
    failures.extend(
        screen_failures
            .into_iter()
            .map(|failure| format!("screen: {failure}")),
    );
    let failure_count = failures.len();

    let report = json!({
        "schema": 2,
        "tool": "stitch-pty-parser-bench",
        "ascii_ground_profile": profile_ascii(bytes, largest_feed),
        "screen_ground_profile": screen_lanes::profile(screen_bytes, largest_feed),
        "config": {
            "samples": samples,
            "warmup": warmup,
            "payload_bytes": bytes,
            "feed_sizes": feeds,
            "screen_payload_bytes": screen_bytes,
            "screen_samples": screen_samples,
            "baseline": "native",
            "candidate": "vte",
            "allocator_instrumented": true,
        },
        "host": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "cpu_count": std::thread::available_parallelism().map(|n| n.get()).ok(),
            "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
            "timestamp_unix": SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .ok(),
        },
        "correctness": {
            "failures": failure_count,
            "details": failures,
            "candidate_notes": notes,
        },
        "results": results
            .iter()
            .map(|sample| {
                let mut object = sample.json().as_object().unwrap().clone();
                object.insert("metric".to_owned(), json!(sample.name()));
                serde_json::Value::Object(object)
            })
            .collect::<Vec<_>>(),
        "screen_results": screen_results
            .iter()
            .map(|lane| lane.json())
            .collect::<Vec<_>>(),
    });

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("create report directory");
    }
    std::fs::write(&out, serde_json::to_string_pretty(&report).expect("encode report"))
        .expect("write report");
    eprintln!("report written to {}", out.display());
}
