//! `silt-cli`: the Silt core without the window, for checking numbers and for scripts.
//! Anything that deletes or rewrites files needs `--yes`; without it the command only shows what
//! it would do. `--json` prints JSON instead of text.

use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::Hasher;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use silt_core::journal::{Action, Entry, Journal};
use silt_core::known::KnownDirs;
use silt_core::largest::{self, Filter, Item};
use silt_core::report::Report;
use silt_core::rules::{self, Rule};
use silt_core::scan::{self, ScanResult};
use silt_core::shrink::{self, Algorithm, Control, Excludes, Sampler, ShrinkReport};
use silt_core::volume::{self, Volume};
use silt_core::{Category, clean, fmt_bytes, now_unix, trash};

const HELP: &str = "\
silt-cli: see what fills your drive and get the space back without deleting what you use.

USAGE
  silt-cli scan [C:] [--walk] [--path <folder>]... [--top <n>]
                                                     scan and check the totals
  silt-cli report [C:] [--no-estimate] [--algo <a>] categories and what can be recovered
  silt-cli largest [C:] [--folders] [--category <c,..>] [--min-size <100MB>]
                   [--older-than <days>] [--limit <n>] the largest files or folders
  silt-cli estimate <folder> [--algo <a>]            what Shrink would save there
  silt-cli shrink <folder> [--algo <a>] --yes        compress a folder (transparent, reversible)
  silt-cli restore <folder> --yes                    undo Shrink
  silt-cli clean [<rule>...] [--yes]                 list clean rules, or run them
  silt-cli trash <path>... --yes                     send to the Recycle Bin
  silt-cli empty-bin [--yes]                         empty the Recycle Bin
  silt-cli roundtrip <folder> [--algo <a>]           estimate, shrink, verify, restore, verify

OPTIONS
  --json    print JSON
  --walk    list folders instead of reading the MFT (no admin needed, slower)
  --algo    xpress4k | xpress8k (default) | xpress16k | lzx
";

struct Args {
    pos: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

const VALUE_FLAGS: [&str; 7] = [
    "path",
    "algo",
    "category",
    "min-size",
    "older-than",
    "limit",
    "top",
];

impl Args {
    fn parse(raw: impl Iterator<Item = String>) -> Self {
        let mut a = Args {
            pos: Vec::new(),
            flags: Vec::new(),
        };
        let mut it = raw.peekable();
        while let Some(s) = it.next() {
            if let Some(f) = s.strip_prefix("--") {
                if let Some((k, v)) = f.split_once('=') {
                    a.flags.push((k.to_string(), Some(v.to_string())));
                } else if VALUE_FLAGS.contains(&f) {
                    a.flags.push((f.to_string(), it.next()));
                } else {
                    a.flags.push((f.to_string(), None));
                }
            } else {
                a.pos.push(s);
            }
        }
        a
    }
    fn has(&self, k: &str) -> bool {
        self.flags.iter().any(|(f, _)| f == k)
    }
    fn get(&self, k: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(f, _)| f == k)
            .and_then(|(_, v)| v.as_deref())
    }
    fn all(&self, k: &str) -> Vec<&str> {
        self.flags
            .iter()
            .filter(|(f, _)| f == k)
            .filter_map(|(_, v)| v.as_deref())
            .collect()
    }
}

type Res = Result<(), String>;

fn main() -> ExitCode {
    let mut raw = std::env::args().skip(1);
    let Some(cmd) = raw.next() else {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    };
    let a = Args::parse(raw);
    let r = match cmd.as_str() {
        "scan" => cmd_scan(&a),
        "report" => cmd_report(&a),
        "largest" => cmd_largest(&a),
        "estimate" => cmd_estimate(&a),
        "shrink" => cmd_shrink(&a),
        "restore" => cmd_restore(&a),
        "clean" => cmd_clean(&a),
        "trash" => cmd_trash(&a),
        "empty-bin" => cmd_empty_bin(&a),
        "roundtrip" => cmd_roundtrip(&a),
        "help" | "--help" | "-h" => {
            print!("{HELP}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`; run `silt-cli help`")),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------- helpers

fn print_json<T: Serialize>(v: &T) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn drive_arg(a: &Args) -> Result<Volume, String> {
    let letter = a
        .pos
        .first()
        .and_then(|s| s.chars().next())
        .unwrap_or('C')
        .to_ascii_uppercase();
    volume::info(letter).map_err(|e| format!("drive {letter}: {e}"))
}

fn folder_arg(a: &Args) -> Result<PathBuf, String> {
    let p = a.pos.first().ok_or("missing <folder>")?;
    let p = std::path::absolute(p).map_err(|e| e.to_string())?;
    if !p.is_dir() {
        return Err(format!("{} is not a folder", p.display()));
    }
    Ok(p)
}

fn algo_arg(a: &Args) -> Result<Algorithm, String> {
    match a.get("algo") {
        None => Ok(Algorithm::default()),
        Some(s) => Algorithm::from_id(s).ok_or_else(|| format!("unknown algorithm `{s}`")),
    }
}

fn parse_size(s: &str) -> Result<u64, String> {
    let s = s.trim().to_ascii_uppercase();
    let split = s
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: f64 = num.parse().map_err(|_| format!("bad size `{s}`"))?;
    let mult: u64 = match unit.trim().trim_end_matches('B') {
        "" => 1,
        "K" => 1 << 10,
        "M" => 1 << 20,
        "G" => 1 << 30,
        "T" => 1 << 40,
        _ => return Err(format!("bad size `{s}`")),
    };
    Ok((n * mult as f64) as u64)
}

fn scan_drive(
    v: &Volume,
    a: &Args,
    known: &KnownDirs,
    rules: &[Rule],
) -> Result<(ScanResult, Option<scan::Fallback>), String> {
    let (h, fallback) = scan::start(v, !a.has("walk"));
    if let Some(f) = &fallback
        && !a.has("walk")
    {
        eprintln!(
            "note: {} (slower walk instead of the MFT)",
            fallback_text(f)
        );
    }
    let r = h
        .collect(known, rules)
        .map_err(|e| format!("scan failed: {e}"))?;
    Ok((r, fallback))
}

fn fallback_text(f: &scan::Fallback) -> String {
    match f {
        scan::Fallback::NotAdmin => "not running as administrator".into(),
        scan::Fallback::NotNtfs => "not an NTFS drive".into(),
        scan::Fallback::MftFailed(e) => format!("could not read the MFT: {e}"),
    }
}

fn peak_memory() -> u64 {
    use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut c = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    if unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) }.as_bool() {
        c.PeakWorkingSetSize as u64
    } else {
        0
    }
}

fn secs(d: Duration) -> String {
    format!("{:.2} s", d.as_secs_f64())
}

fn pct(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

fn journal(known: &KnownDirs, e: Entry) {
    if let Err(err) = Journal::for_user(known).append(&e) {
        eprintln!("warning: could not write the journal: {err}");
    }
}

// ---------------------------------------------------------------- scan

#[derive(Serialize)]
struct ScanOut {
    drive: String,
    fs: String,
    scanner: scan::ScannerKind,
    fallback: Option<scan::Fallback>,
    elapsed_ms: u64,
    entries: u64,
    files: u64,
    folders: u64,
    capacity: u64,
    used: u64,
    free: u64,
    /// Allocated bytes the scan attributed to files and folders.
    found: u64,
    /// `used - found`: file system metadata the scan cannot attribute.
    metadata: u64,
    overcount: u64,
    tree_memory: u64,
    peak_memory: u64,
    paths: Vec<PathOut>,
}

#[derive(Serialize)]
struct PathOut {
    path: String,
    seen: bool,
    alloc: u64,
    logical: u64,
    files: u64,
}

fn cmd_scan(a: &Args) -> Res {
    let v = drive_arg(a)?;
    let known = KnownDirs::current();
    let rules = rules::builtin();
    let (r, fallback) = scan_drive(&v, a, &known, &rules)?;
    let t = &r.tree;
    let paths = a
        .all("path")
        .into_iter()
        .map(|p| {
            let full = std::path::absolute(p)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(p.to_string());
            match t.find(&full) {
                Some(id) => {
                    let n = t.node(id);
                    PathOut {
                        path: full,
                        seen: true,
                        alloc: n.alloc,
                        logical: n.logical,
                        files: u64::from(n.files),
                    }
                }
                None => PathOut {
                    path: full,
                    seen: false,
                    alloc: 0,
                    logical: 0,
                    files: 0,
                },
            }
        })
        .collect();
    let out = ScanOut {
        drive: r.volume.drive(),
        fs: r.volume.fs.clone(),
        scanner: r.kind,
        fallback,
        elapsed_ms: r.elapsed.as_millis() as u64,
        entries: t.nodes.len() as u64,
        files: u64::from(t.node(t.root).files),
        folders: t.nodes.iter().filter(|n| n.is_dir()).count() as u64,
        capacity: r.volume.total,
        used: r.volume.used(),
        free: r.volume.free,
        found: t.total() - r.metadata,
        metadata: r.metadata,
        overcount: r.overcount,
        tree_memory: t.heap_bytes() as u64,
        peak_memory: peak_memory(),
        paths,
    };
    if a.has("json") {
        print_json(&out);
        return Ok(());
    }
    println!(
        "{} ({})  scanned with {:?} in {}",
        out.drive,
        out.fs,
        out.scanner,
        secs(r.elapsed)
    );
    println!(
        "  entries   {:>14}   files {}   folders {}",
        out.entries, out.files, out.folders
    );
    println!(
        "  used      {:>14}   ({} B)  reported by the drive",
        fmt_bytes(out.used),
        out.used
    );
    println!(
        "  found     {:>14}   ({} B)  {:.2}% of used",
        fmt_bytes(out.found),
        out.found,
        pct(out.found, out.used)
    );
    println!(
        "  metadata  {:>14}   ({} B)  {:.2}% of used, shown under System",
        fmt_bytes(out.metadata),
        out.metadata,
        pct(out.metadata, out.used)
    );
    if out.overcount > 0 {
        println!(
            "  overcount {:>14}   found beyond used (hard links seen twice by the walk)",
            fmt_bytes(out.overcount)
        );
    }
    println!(
        "  memory    tree {}   peak {}",
        fmt_bytes(out.tree_memory),
        fmt_bytes(out.peak_memory)
    );
    if let Some(n) = a.get("top").and_then(|n| n.parse::<usize>().ok()) {
        for c in t.children(t.root).take(n) {
            println!("  {:>12}  {}", fmt_bytes(t.node(c).alloc), t.name(c));
        }
    }
    for p in &out.paths {
        if p.seen {
            println!(
                "  {}  alloc {} ({} B)  logical {} ({} B)  files {}",
                p.path,
                fmt_bytes(p.alloc),
                p.alloc,
                fmt_bytes(p.logical),
                p.logical,
                p.files
            );
        } else {
            println!("  {}  not in the scan", p.path);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- report

fn cmd_report(a: &Args) -> Res {
    let v = drive_arg(a)?;
    let known = KnownDirs::current();
    let rules = rules::builtin();
    let algo = algo_arg(a)?;
    let (r, _) = scan_drive(&v, a, &known, &rules)?;
    let entries = Journal::for_user(&known).read();
    let mut rep = Report::build(
        &r,
        &rules,
        &entries,
        trash::bin_info_drive(&r.volume.root),
        now_unix(),
    );
    if !a.has("no-estimate") {
        let started = Instant::now();
        let done = AtomicUsize::new(0);
        rep.estimate(
            &r.tree,
            r.volume.cluster,
            algo,
            &AtomicBool::new(false),
            &done,
        );
        eprintln!(
            "estimated {} folders in {}",
            done.load(Ordering::Relaxed),
            secs(started.elapsed())
        );
    }
    if a.has("json") {
        print_json(&rep);
        return Ok(());
    }
    println!(
        "{} {}  {}  used {} of {}, {} free",
        rep.drive,
        rep.label,
        rep.fs,
        fmt_bytes(rep.used),
        fmt_bytes(rep.capacity),
        fmt_bytes(rep.free)
    );
    println!(
        "scanned {} files in {:.2} s ({:?})\n",
        rep.files,
        rep.elapsed_ms as f64 / 1000.0,
        rep.scanner
    );
    println!(
        "{:<12}{:>12}{:>12}{:>12}",
        "category", "size", "clean", "shrink"
    );
    for c in &rep.categories {
        println!(
            "{:<12}{:>12}{:>12}{:>12}",
            c.category.id(),
            fmt_bytes(c.bytes),
            fmt_bytes(c.clean),
            fmt_bytes(c.shrink)
        );
    }
    println!(
        "\nrecoverable {}  (without deleting anything: {})",
        fmt_bytes(rep.recoverable()),
        fmt_bytes(rep.without_deleting())
    );
    println!(
        "\nclean {} in {} rules",
        fmt_bytes(rep.clean_total()),
        rep.clean.len()
    );
    for c in &rep.clean {
        println!(
            "  {:<28}{:>10}  {:?}  {}",
            c.rule,
            fmt_bytes(c.bytes),
            c.safety,
            c.folders.join("; ")
        );
    }
    let (lo, hi) = rep.shrink_range();
    println!(
        "\nshrink {} to {} in {} folders",
        fmt_bytes(lo),
        fmt_bytes(hi),
        rep.shrink.len()
    );
    for s in rep.shrink.iter().take(25) {
        let saves = s.estimate.as_ref().map(|e| {
            let (l, h) = e.saving();
            format!("saves {} to {}", fmt_bytes(l), fmt_bytes(h))
        });
        let flags = format!(
            "{}{}",
            if s.directstorage {
                "  [DirectStorage]"
            } else {
                ""
            },
            if s.shrunk_before {
                "  [compressible again]"
            } else {
                ""
            }
        );
        println!(
            "  {:>10}  {:<9}{}  {}{}",
            fmt_bytes(s.eligible),
            s.category.id(),
            saves.unwrap_or_default(),
            s.path,
            flags
        );
    }
    if rep.shrink.len() > 25 {
        println!("  ... {} more", rep.shrink.len() - 25);
    }
    println!(
        "\nlarge and old files {} in {} files (100 MB+, untouched for 6 months)",
        fmt_bytes(rep.large_old.bytes),
        rep.large_old.files
    );
    println!(
        "recycle bin {} in {} items",
        fmt_bytes(rep.bin.bytes),
        rep.bin.items
    );
    let again = rep.compressible_again();
    if again > 0 {
        println!(
            "compressible again {} (updates rewrote shrunk files)",
            fmt_bytes(again)
        );
    }
    Ok(())
}

// ---------------------------------------------------------------- largest

fn cmd_largest(a: &Args) -> Res {
    let v = drive_arg(a)?;
    let known = KnownDirs::current();
    let rules = rules::builtin();
    let mut f = Filter::default();
    if let Some(c) = a.get("category") {
        f.categories = c
            .split(',')
            .map(|s| Category::from_id(s.trim()).ok_or_else(|| format!("unknown category `{s}`")))
            .collect::<Result<_, _>>()?;
    }
    if let Some(s) = a.get("min-size") {
        f.min_size = parse_size(s)?;
    }
    if let Some(d) = a.get("older-than") {
        f.min_age_days = d.parse().map_err(|_| format!("bad --older-than `{d}`"))?;
    }
    f.limit = a
        .get("limit")
        .map(|l| l.parse().map_err(|_| format!("bad --limit `{l}`")))
        .transpose()?
        .unwrap_or(50);
    let (r, _) = scan_drive(&v, a, &known, &rules)?;
    let t = &r.tree;
    let now = now_unix();
    let items: Vec<Item> = if a.has("folders") {
        let dom = largest::dominant_categories(t);
        largest::folders(t, &dom, &f, now)
            .into_iter()
            .map(|id| Item::new(t, id, dom[id as usize]))
            .collect()
    } else {
        largest::files(t, &f, now)
            .into_iter()
            .map(|id| Item::new(t, id, t.node(id).category))
            .collect()
    };
    if a.has("json") {
        print_json(&items);
        return Ok(());
    }
    for i in &items {
        let age = now.saturating_sub(u64::from(i.mtime)) / 86_400;
        println!(
            "{:>10}  {:<10}{:>6} d  {}",
            fmt_bytes(i.alloc),
            i.category.id(),
            age,
            i.path
        );
    }
    Ok(())
}

// ---------------------------------------------------------------- estimate, shrink, restore

fn estimate_folder(folder: &Path, algo: Algorithm) -> shrink::Estimate {
    let files = shrink::files_on_disk(folder, &Excludes::builtin());
    let cluster = volume::info(folder.to_string_lossy().chars().next().unwrap_or('C'))
        .map_or(4096, |v| v.cluster);
    let _bg = shrink::Background::enter();
    Sampler::new(algo).estimate(
        &folder.to_string_lossy(),
        &files,
        cluster,
        64 << 20,
        &AtomicBool::new(false),
    )
}

fn print_estimate(e: &shrink::Estimate, algo: Algorithm) {
    let (lo, hi) = e.saving();
    println!("{}  ({})", e.folder, algo.id());
    println!(
        "  {} files Shrink would compress: {}",
        e.files,
        fmt_bytes(e.eligible)
    );
    println!(
        "  after {} to {}, saves {} to {}",
        fmt_bytes(e.after_low),
        fmt_bytes(e.after_high),
        fmt_bytes(lo),
        fmt_bytes(hi)
    );
    if e.already > 0 {
        println!("  already compressed: {}", fmt_bytes(e.already));
    }
    if e.directstorage {
        println!(
            "  warning: this folder uses DirectStorage; compressed files may load slower in game"
        );
    }
}

fn cmd_estimate(a: &Args) -> Res {
    let folder = folder_arg(a)?;
    let algo = algo_arg(a)?;
    let started = Instant::now();
    let e = estimate_folder(&folder, algo);
    if a.has("json") {
        print_json(&e);
    } else {
        print_estimate(&e, algo);
        println!("  estimated in {}", secs(started.elapsed()));
    }
    Ok(())
}

/// Runs `f` while a thread prints progress to stderr every second.
fn with_progress<T>(ctl: &Control, f: impl FnOnce() -> T) -> T {
    let stop = AtomicBool::new(false);
    std::thread::scope(|s| {
        s.spawn(|| {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(250));
                let (d, t) = (
                    ctl.done_bytes.load(Ordering::Relaxed),
                    ctl.total_bytes.load(Ordering::Relaxed),
                );
                if t > 0 {
                    eprint!(
                        "\r  {} of {} ({:.0}%)   ",
                        fmt_bytes(d),
                        fmt_bytes(t),
                        pct(d, t)
                    );
                }
            }
            eprint!("\r{:60}\r", "");
        });
        let r = f();
        stop.store(true, Ordering::Relaxed);
        r
    })
}

fn print_shrink(r: &ShrinkReport, verb: &str, a: &Args) {
    if a.has("json") {
        print_json(r);
        return;
    }
    if let Some(why) = &r.refused {
        println!("refused: {why}");
        return;
    }
    println!("{verb} {} files in {}", r.files, r.folder);
    println!(
        "  before {}  after {}  ({}{})",
        fmt_bytes(r.before),
        fmt_bytes(r.after),
        if r.saved() >= 0 { "-" } else { "+" },
        fmt_bytes(r.saved().unsigned_abs())
    );
    if r.not_beneficial > 0 {
        println!(
            "  {} files left as they were (nothing to gain)",
            r.not_beneficial
        );
    }
    for s in r.skipped.iter().take(20) {
        println!("  skipped {}: {}", s.path, s.reason);
    }
    if r.skipped.len() > 20 {
        println!("  ... {} more skipped", r.skipped.len() - 20);
    }
    if r.cancelled {
        println!("  cancelled");
    }
}

fn cmd_shrink(a: &Args) -> Res {
    let folder = folder_arg(a)?;
    let algo = algo_arg(a)?;
    if !a.has("yes") {
        print_estimate(&estimate_folder(&folder, algo), algo);
        println!("\nadd --yes to compress");
        return Ok(());
    }
    let ctl = Control::default();
    let r = with_progress(&ctl, || {
        shrink::shrink_folder(&folder, algo, &Excludes::builtin(), &ctl)
    });
    if r.refused.is_none() {
        journal(
            &KnownDirs::current(),
            Entry {
                ts: now_unix(),
                action: Action::Shrink,
                target: r.folder.clone(),
                algorithm: Some(algo.id().into()),
                before: r.before,
                after: r.after,
                files: r.files,
                skipped: r.skipped.len() as u64,
            },
        );
    }
    print_shrink(&r, "compressed", a);
    Ok(())
}

fn cmd_restore(a: &Args) -> Res {
    let folder = folder_arg(a)?;
    if !a.has("yes") {
        let f = shrink::files_on_disk(&folder, &Excludes::builtin());
        println!(
            "{}: {} compressed, {} once restored",
            folder.display(),
            fmt_bytes(f.already),
            fmt_bytes(f.already_logical)
        );
        println!("\nadd --yes to restore");
        return Ok(());
    }
    let ctl = Control::default();
    let r = with_progress(&ctl, || shrink::restore_folder(&folder, &ctl));
    if r.refused.is_none() {
        journal(
            &KnownDirs::current(),
            Entry {
                ts: now_unix(),
                action: Action::Restore,
                target: r.folder.clone(),
                algorithm: None,
                before: r.before,
                after: r.after,
                files: r.files,
                skipped: r.skipped.len() as u64,
            },
        );
    }
    print_shrink(&r, "restored", a);
    Ok(())
}

// ---------------------------------------------------------------- clean, trash, empty-bin

#[derive(Serialize)]
struct RuleOut<'a> {
    rule: &'a Rule,
    targets: Vec<(String, u64)>,
    bytes: u64,
}

fn cmd_clean(a: &Args) -> Res {
    let known = KnownDirs::current();
    let all = rules::builtin();
    let chosen: Vec<&Rule> = if a.pos.is_empty() {
        all.iter().collect()
    } else {
        a.pos
            .iter()
            .map(|id| {
                all.iter()
                    .find(|r| &r.id == id)
                    .ok_or_else(|| format!("unknown rule `{id}`"))
            })
            .collect::<Result<_, _>>()?
    };
    if a.pos.is_empty() || !a.has("yes") {
        let out: Vec<RuleOut> = chosen
            .iter()
            .map(|r| {
                let targets: Vec<(String, u64)> = r
                    .targets(&known)
                    .iter()
                    .map(|t| (t.to_string_lossy().into_owned(), clean::measure(t).0))
                    .collect();
                RuleOut {
                    rule: r,
                    bytes: targets.iter().map(|t| t.1).sum(),
                    targets,
                }
            })
            .collect();
        if a.has("json") {
            print_json(&out);
            return Ok(());
        }
        for o in &out {
            println!(
                "{:<28}{:>10}  {:?}  {}",
                o.rule.id,
                fmt_bytes(o.bytes),
                o.rule.safety,
                o.rule.note
            );
            for (t, b) in &o.targets {
                println!("    {:>10}  {t}", fmt_bytes(*b));
            }
        }
        if !a.pos.is_empty() {
            println!("\nadd --yes to clean");
        }
        return Ok(());
    }
    let cancel = AtomicBool::new(false);
    let mut reports = Vec::new();
    for r in chosen {
        let rep = clean::clean_rule(r, &known, &cancel);
        journal(
            &known,
            Entry {
                ts: now_unix(),
                action: Action::Clean,
                target: r.id.clone(),
                algorithm: None,
                before: rep.before,
                after: rep.after,
                files: rep.deleted_files,
                skipped: rep.skipped.len() as u64,
            },
        );
        reports.push(rep);
    }
    if a.has("json") {
        print_json(&reports);
        return Ok(());
    }
    for r in &reports {
        println!(
            "{}: freed {} ({} files deleted, {} skipped)",
            r.rule,
            fmt_bytes(r.freed()),
            r.deleted_files,
            r.skipped.len()
        );
        for s in r.refused.iter().chain(r.skipped.iter().take(10)) {
            println!("    {}: {}", s.path, s.reason);
        }
    }
    Ok(())
}

fn cmd_trash(a: &Args) -> Res {
    if a.pos.is_empty() {
        return Err("missing <path>".into());
    }
    let paths: Vec<PathBuf> = a
        .pos
        .iter()
        .map(|p| std::path::absolute(p).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    if !a.has("yes") {
        for p in &paths {
            println!("{:>10}  {}", fmt_bytes(clean::measure(p).0), p.display());
        }
        println!("\nadd --yes to send to the Recycle Bin");
        return Ok(());
    }
    let known = KnownDirs::current();
    let r = trash::send_to_trash(&paths, &known);
    if !r.trashed.is_empty() {
        journal(
            &known,
            Entry {
                ts: now_unix(),
                action: Action::Trash,
                target: format!("{} items", r.trashed.len()),
                algorithm: None,
                before: r.bytes,
                after: 0,
                files: r.trashed.len() as u64,
                skipped: (r.refused.len() + r.failed.len()) as u64,
            },
        );
    }
    if a.has("json") {
        print_json(&r);
        return Ok(());
    }
    println!(
        "sent {} items ({}) to the Recycle Bin",
        r.trashed.len(),
        fmt_bytes(r.bytes)
    );
    for s in r.refused.iter().chain(&r.failed) {
        println!("  {}: {}", s.path, s.reason);
    }
    Ok(())
}

fn cmd_empty_bin(a: &Args) -> Res {
    let info = trash::bin_info();
    if !a.has("yes") {
        println!(
            "the Recycle Bin holds {} in {} items",
            fmt_bytes(info.bytes),
            info.items
        );
        if info.items > 0 {
            println!("\nadd --yes to empty it");
        }
        return Ok(());
    }
    let before = trash::empty_bin().map_err(|e| e.to_string())?;
    if before.items > 0 {
        journal(
            &KnownDirs::current(),
            Entry {
                ts: now_unix(),
                action: Action::EmptyBin,
                target: format!("{} items", before.items),
                algorithm: None,
                before: before.bytes,
                after: 0,
                files: before.items,
                skipped: 0,
            },
        );
    }
    println!(
        "emptied the Recycle Bin: {} in {} items",
        fmt_bytes(before.bytes),
        before.items
    );
    Ok(())
}

// ---------------------------------------------------------------- roundtrip

#[derive(Serialize)]
struct Roundtrip {
    folder: String,
    algorithm: Algorithm,
    files: u64,
    /// Allocated bytes of the files Shrink compresses.
    eligible: u64,
    estimate_low: u64,
    estimate_high: u64,
    estimate_ms: u64,
    /// Measured after Shrink.
    after: u64,
    saved: i64,
    saved_pct: f64,
    /// (actual saving - estimate middle) / actual saving.
    estimate_error_pct: f64,
    not_beneficial: u64,
    skipped: u64,
    shrink_ms: u64,
    restore_ms: u64,
    identical_after_shrink: bool,
    identical_after_restore: bool,
    /// Files that could not be opened for reading (and so were not compared).
    unreadable: u64,
    restored_to: u64,
}

/// Every file that could be read before reads back the same.
fn same(before: &[Option<u64>], after: &[Option<u64>]) -> bool {
    before.iter().zip(after).all(|(b, a)| b.is_none() || b == a)
}

fn hash_files(files: &[PathBuf]) -> Vec<Option<u64>> {
    let mut buf = vec![0u8; 1 << 20];
    files
        .iter()
        .map(|p| {
            let mut f = File::open(p).ok()?;
            let mut h = DefaultHasher::new();
            loop {
                let n = f.read(&mut buf).ok()?;
                if n == 0 {
                    break;
                }
                h.write(&buf[..n]);
            }
            Some(h.finish())
        })
        .collect()
}

fn cmd_roundtrip(a: &Args) -> Res {
    let folder = folder_arg(a)?;
    let algo = algo_arg(a)?;
    let ex = Excludes::builtin();
    let found = shrink::files_on_disk(&folder, &ex);
    if found.already > 0 {
        return Err(format!(
            "{} already has compressed files ({}); restoring them at the end would change it",
            folder.display(),
            fmt_bytes(found.already)
        ));
    }
    let t = Instant::now();
    let est = estimate_folder(&folder, algo);
    let estimate_ms = t.elapsed().as_millis() as u64;
    let paths: Vec<PathBuf> = found.eligible.iter().map(|c| c.path.clone()).collect();
    eprintln!(
        "hashing {} files ({})",
        paths.len(),
        fmt_bytes(found.eligible.iter().map(|c| c.logical).sum())
    );
    let before = hash_files(&paths);

    let ctl = Control::default();
    let t = Instant::now();
    let s = with_progress(&ctl, || shrink::shrink_folder(&folder, algo, &ex, &ctl));
    let shrink_ms = t.elapsed().as_millis() as u64;
    if let Some(why) = s.refused {
        return Err(why);
    }
    let after_shrink = hash_files(&paths);

    let ctl = Control::default();
    let t = Instant::now();
    let r = with_progress(&ctl, || shrink::restore_folder(&folder, &ctl));
    let restore_ms = t.elapsed().as_millis() as u64;
    let after_restore = hash_files(&paths);

    let saved = s.saved();
    let mid = est.saving_mid() as f64;
    let out = Roundtrip {
        folder: folder.to_string_lossy().into_owned(),
        algorithm: algo,
        files: paths.len() as u64,
        eligible: est.eligible,
        estimate_low: est.saving().0,
        estimate_high: est.saving().1,
        estimate_ms,
        after: est.eligible.saturating_sub(saved.max(0) as u64),
        saved,
        saved_pct: pct(saved.max(0) as u64, est.eligible),
        estimate_error_pct: if saved > 0 {
            (saved as f64 - mid) * 100.0 / saved as f64
        } else {
            0.0
        },
        not_beneficial: s.not_beneficial,
        skipped: s.skipped.len() as u64,
        shrink_ms,
        restore_ms,
        identical_after_shrink: same(&before, &after_shrink),
        identical_after_restore: same(&before, &after_restore),
        unreadable: before.iter().filter(|h| h.is_none()).count() as u64,
        restored_to: r.after,
    };
    if a.has("json") {
        print_json(&out);
        return Ok(());
    }
    println!("{}  ({})", out.folder, algo.id());
    println!(
        "  files {}  eligible {}",
        out.files,
        fmt_bytes(out.eligible)
    );
    println!(
        "  estimate saves {} to {} ({} ms)",
        fmt_bytes(out.estimate_low),
        fmt_bytes(out.estimate_high),
        out.estimate_ms
    );
    println!(
        "  actual saved {} ({:.1}%), estimate error {:+.1}%",
        fmt_bytes(out.saved.max(0) as u64),
        out.saved_pct,
        out.estimate_error_pct
    );
    println!(
        "  shrink {} ms, restore {} ms, not beneficial {}, skipped {}",
        out.shrink_ms, out.restore_ms, out.not_beneficial, out.skipped
    );
    println!(
        "  identical after shrink: {}  after restore: {}",
        out.identical_after_shrink, out.identical_after_restore
    );
    if out.unreadable > 0 {
        println!(
            "  {} files could not be opened for reading and were not compared",
            out.unreadable
        );
    }
    Ok(())
}
