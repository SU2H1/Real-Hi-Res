//! Command-line interface: `realhires PATH... [options]` (the Rust counterpart
//! of `realhires/cli.py`, with the same options, output and saved results).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

use clap::{ArgAction, Parser, ValueEnum};
use realhires::eta::{file_sizes, ScanEta};
use realhires::fix::{plan_fixes, prepare_gapless, run_fix, run_parallel, skip_summary, Backup};
use realhires::lyrics::{
    check_server, lyrics_summary, run_lyrics, skip_reason, LrclibClient, LyricsOutcome, Status, EXISTING_CLI,
};
use realhires::paths::{abspath, collect, file_name};
use realhires::probe::require_tools;
use realhires::report::{
    header, row, safe_stem, short_hash, summary, summary_bars, use_color, write_csv, write_json, write_svg,
};
use realhires::settings::{self, Settings};
use realhires::store::{open_store, settings_key, ResultStore};
use realhires::verdict::{analyze_file, FileResult, Verdict};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Only {
    Error,
    Fake,
    #[value(name = "fake-hires")]
    FakeHires,
    #[value(name = "fake-lossless")]
    FakeLossless,
    Lossy,
    #[value(name = "not-hires")]
    NotHires,
    Problems,
    Real,
    Suspicious,
}

impl Only {
    fn verdicts(self) -> &'static [Verdict] {
        match self {
            Only::Fake => &[Verdict::Fake, Verdict::FakeLossless],
            Only::FakeHires => &[Verdict::Fake],
            Only::FakeLossless => &[Verdict::FakeLossless],
            Only::Problems => &[Verdict::Fake, Verdict::FakeLossless, Verdict::Suspect],
            Only::Suspicious => &[Verdict::Suspect],
            Only::Real => &[Verdict::Real],
            Only::NotHires => &[Verdict::NotHires],
            Only::Lossy => &[Verdict::Lossy],
            Only::Error => &[Verdict::Error],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum BackupArg {
    Folder,
    Bak,
    None,
}

#[derive(Parser, Debug)]
#[command(
    name = "realhires",
    version,
    about = "Scan audio files and report which are real high-resolution / lossless audio and which are fakes: \
             upsampled from 44.1/48 kHz, 16-bit padded to 24-bit, or 'fake FLAC/WAV/ALAC' files made from MP3/AAC.",
    after_help = after_help()
)]
struct Args {
    /// audio files and/or folders to scan
    paths: Vec<String>,
    /// do not descend into sub-folders
    #[arg(short = 'n', long)]
    no_recursive: bool,
    /// files analysed in parallel; 0 = one per CPU core (default: from settings)
    #[arg(short = 'j', long)]
    jobs: Option<i64>,
    /// analyse only the first N seconds of each file (default: from settings, normally the whole file)
    #[arg(short = 't', long)]
    max_seconds: Option<f64>,
    /// show the full reasoning and measurements for every file
    #[arg(short = 'v', long)]
    verbose: bool,
    /// only list files with this verdict (repeatable; 'fake' = fake hi-res + fake lossless, 'problems' = fake +
    /// suspicious)
    #[arg(long, value_enum, action = ArgAction::Append)]
    only: Vec<Only>,
    /// write full results as JSON
    #[arg(long, value_name = "FILE")]
    json: Option<String>,
    /// write a CSV summary
    #[arg(long, value_name = "FILE")]
    csv: Option<String>,
    /// write an SVG average-spectrum plot per file into DIR
    #[arg(long, value_name = "DIR")]
    plot: Option<String>,
    /// write a PNG spectrogram per file into DIR
    #[arg(long, value_name = "DIR")]
    spectrogram: Option<String>,
    /// after scanning, fix every fake file in place so its properties show its real resolution (e.g. a fake 24/96
    /// becomes a genuine 16/44.1 file with the same name, tags and cover art); originals are moved to a
    /// 'Real-Hi-Res backups' folder
    #[arg(long)]
    fix: bool,
    /// with --fix: keep originals in the backup folder (default), as <name>.bak, or delete them
    #[arg(long, value_enum)]
    fix_backup: Option<BackupArg>,
    /// with --fix: put backups of originals into DIR
    #[arg(long, value_name = "DIR")]
    fix_backup_dir: Option<String>,
    /// with --fix: leave the files alone and save corrected copies next to them
    #[arg(long)]
    fix_copy: bool,
    /// with --fix: leave the files alone and save corrected copies into DIR
    #[arg(long, value_name = "DIR")]
    fix_dir: Option<String>,
    #[arg(long, hide = true)]
    fix_replace: bool,
    /// with --fix or --lyrics: only show what would be changed (lyrics are still looked up, but nothing is saved)
    #[arg(long)]
    dry_run: bool,
    /// after scanning, download synced lyrics from LRCLIB (lrclib.net) for every scanned song and save them as
    /// <name>.lrc next to it (plain lyrics as <name>.txt), as LRCGET does; songs are looked up by the title, artist,
    /// album and length in their tags
    #[arg(long)]
    lyrics: bool,
    /// download lyrics as --lyrics does, without analysing the files (only their tags are read)
    #[arg(long)]
    lyrics_only: bool,
    /// with --lyrics: skip songs that have an .lrc (skip-synced, the default), skip songs that have an .lrc or .txt
    /// (skip-any), or download again and replace (replace)
    #[arg(long, value_enum)]
    lyrics_existing: Option<ExistingArg>,
    /// check that the LRCLIB server can be reached (one request) and exit
    #[arg(long)]
    check_lyrics_server: bool,
    /// analyse every file again, even files with a saved result from an earlier scan (the saved results are
    /// updated)
    #[arg(long)]
    rescan: bool,
    /// analyse again the files whose saved result was made by older detection rules (after an update of
    /// Real-Hi-Res)
    #[arg(long)]
    rescan_outdated: bool,
    /// neither use nor save saved results in this run
    #[arg(long)]
    no_cache: bool,
    /// delete all saved results and exit
    #[arg(long)]
    forget_results: bool,
    /// use this settings file instead of the saved one
    #[arg(long, value_name = "FILE")]
    settings: Option<PathBuf>,
    /// ignore saved settings; use defaults
    #[arg(long)]
    no_settings: bool,
    /// override one setting for this run (repeatable)
    #[arg(long = "set", value_name = "NAME=VALUE", action = ArgAction::Append)]
    set: Vec<String>,
    /// print every setting with its current value and exit
    #[arg(long)]
    list_settings: bool,
    /// disable coloured output
    #[arg(long)]
    no_color: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ExistingArg {
    SkipSynced,
    SkipAny,
    Replace,
}

impl ExistingArg {
    fn choice(self) -> &'static str {
        let key = match self {
            ExistingArg::SkipSynced => "skip-synced",
            ExistingArg::SkipAny => "skip-any",
            ExistingArg::Replace => "replace",
        };
        EXISTING_CLI.iter().find(|(k, _)| *k == key).unwrap().1
    }
}

fn after_help() -> String {
    format!(
        "Settings saved in the desktop app (Settings window) are used here too; see {}. Use --set NAME=VALUE to \
         override one for a run (e.g. --set cliff_db=30) and --list-settings to see all names.",
        settings::default_path().display()
    )
}

fn load_settings(args: &Args) -> Result<Settings, String> {
    let mut cfg = if args.no_settings { Settings::default() } else { settings::load(args.settings.as_deref()) };
    for item in &args.set {
        let Some((name, value)) = item.split_once('=') else {
            return Err(format!("--set expects NAME=VALUE, got '{item}'"));
        };
        cfg.set_value(name.trim(), value.trim())?;
    }
    Ok(cfg)
}

fn write_spectrogram_png(r: &FileResult, out_dir: &str, cfg: &Settings) -> Option<String> {
    let sg = r.plot.as_ref()?.spectrogram.as_ref()?;
    let mut markers = vec![];
    if let Some(u) = r.upsampled_from {
        markers.push((u as f64 / 2.0, [0, 200, 255]));
    }
    if let Some(l) = r.lossy_cutoff_hz {
        markers.push((l, [255, 80, 80]));
    }
    std::fs::create_dir_all(out_dir).ok()?;
    let path = Path::new(out_dir).join(format!("{}_{}.png", safe_stem(&r.path), short_hash(&r.path)));
    let png = realhires::imaging::spectrogram_png(sg, 1000, 400, cfg.spectrogram_range_db, &cfg.colormap, &markers)?;
    std::fs::write(&path, png).ok()?;
    Some(path.to_string_lossy().into_owned())
}

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args = Args::parse();
    let mut cfg = match load_settings(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    if args.list_settings {
        for info in Settings::field_info() {
            let v = cfg.get(info.name).unwrap().repr();
            println!("{:<22} = {:<28} # {}", info.name, v, info.label);
        }
        return 0;
    }
    if args.forget_results {
        return match ResultStore::open(None, realhires::store::DEFAULT_GRAPHS_MB) {
            Ok(st) => match st.forget(None) {
                Ok(n) => {
                    println!("Deleted {n} saved result(s) from {}.", st.path.display());
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    2
                }
            },
            Err(e) => {
                eprintln!("error: {e}");
                2
            }
        };
    }
    if args.check_lyrics_server {
        let mut client = LrclibClient::new(&cfg.lyrics_server);
        return match check_server(&mut client) {
            Ok(text) => {
                println!("{text}");
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        };
    }
    if args.paths.is_empty() {
        eprintln!("usage: realhires [OPTIONS] [PATHS]...\nrealhires: error: no files or folders given");
        return 2;
    }
    if let Err(e) = require_tools() {
        eprintln!("error: {e}");
        return 2;
    }

    let recursive = cfg.recursive && !args.no_recursive;
    let files = collect(&args.paths, recursive, &cfg.extension_set(), &mut |p| eprintln!("warning: not found: {p}"));
    if files.is_empty() {
        eprintln!("No audio files found.");
        return 1;
    }
    if let Some(e) = args.lyrics_existing {
        cfg.lyrics_existing = e.choice().into();
    }
    if args.lyrics_only {
        return run_lyrics_cli(&files, &cfg, args.dry_run, args.verbose, &HashMap::new());
    }

    let stdout_tty = std::io::stdout().is_terminal();
    let color = use_color(stdout_tty, args.no_color);
    let wanted: Option<HashSet<Verdict>> =
        (!args.only.is_empty()).then(|| args.only.iter().flat_map(|o| o.verdicts().iter().copied()).collect());
    let base = (args.paths.len() == 1 && Path::new(&args.paths[0]).is_dir()).then(|| args.paths[0].clone());
    // -t 0 asks for the whole file even when the settings limit the length
    let max_seconds = args.max_seconds.or_else(|| (cfg.max_seconds != 0.0).then_some(cfg.max_seconds));

    if args.fix_replace {
        cfg.fix_output = "Replace the file".into();
    }
    if args.fix_copy {
        cfg.fix_output = "Saving a copy next to it".into();
    }
    if let Some(d) = &args.fix_dir {
        cfg.fix_output = "Saving a copy into a folder".into();
        cfg.fix_folder = abspath(d);
    }
    if let Some(b) = args.fix_backup {
        cfg.fix_backup = match b {
            BackupArg::Folder => "Move the original to a backup folder",
            BackupArg::Bak => "Keep it as <name>.bak next to the file",
            BackupArg::None => "Delete it (no backup)",
        }
        .into();
    }
    if let Some(d) = &args.fix_backup_dir {
        cfg.fix_backup_folder = abspath(d);
    }

    // Saved results of files that have not changed since an earlier scan are used
    // as they are; only the other files are analysed (and then saved).
    let mut store = if args.no_cache { None } else { open_store(&cfg) };
    let skey = settings_key(&cfg);
    let mut ready: BTreeMap<usize, FileResult> = BTreeMap::new();
    if let Some(st) = store.as_ref() {
        if !args.rescan && args.plot.is_none() {
            let again_outdated = args.rescan_outdated || cfg.rescan_outdated;
            for (i, f) in files.iter().enumerate() {
                let Some(mut r) = st.lookup(f, &skey, max_seconds) else { continue };
                if r.saved.as_ref().is_some_and(|s| s.outdated) && again_outdated {
                    continue;
                }
                if args.spectrogram.is_some() {
                    r.plot = st.get_plot(&r.path, &skey);
                    if !r.plot.as_ref().is_some_and(|p| p.spectrogram.is_some()) {
                        continue; // no saved spectrogram: analyse to draw one
                    }
                }
                ready.insert(i, r);
            }
        }
    }

    println!("{}", header());
    let jobs = realhires::effective_jobs(args.jobs.unwrap_or(cfg.jobs));
    let show_progress = std::io::stderr().is_terminal();
    let todo: Vec<usize> = (0..files.len()).filter(|i| !ready.contains_key(i)).collect();
    let mut eta = show_progress.then(|| {
        let paths: Vec<String> = todo.iter().map(|i| abspath(&files[*i])).collect();
        ScanEta::new(file_sizes(paths.iter()))
    });
    let mut results: Vec<FileResult> = Vec::with_capacity(files.len());
    let keep_spectrum = args.plot.is_some();
    let keep_plot = args.spectrogram.is_some();
    let total = files.len();

    let mut handle = |i: usize, mut r: FileResult, results: &mut Vec<FileResult>, eta: &mut Option<ScanEta>| {
        if r.saved.is_none() {
            if let Some(st) = store.as_mut() {
                st.put(&r, &skey, max_seconds, r.plot.as_ref());
            }
        }
        if show_progress {
            eprint!("\r\x1b[K");
        }
        if let (Some(dir), true) = (&args.plot, r.spectrum.is_some()) {
            if let Err(e) = write_svg(&r, dir) {
                eprintln!("warning: cannot write a plot into {dir}: {e}");
            }
        }
        if let Some(dir) = &args.spectrogram {
            write_spectrogram_png(&r, dir, &cfg);
        }
        r.spectrum = None; // free memory
        r.plot = None;
        if let Some(e) = eta.as_mut() {
            if r.saved.is_none() {
                e.add(&r.path);
            }
        }
        if wanted.as_ref().is_none_or(|w| w.contains(&r.verdict)) {
            println!("{}", row(&r, color, base.as_deref(), args.verbose, &cfg));
            let _ = std::io::stdout().flush();
        }
        results.push(r);
        if show_progress && i < total {
            let left = eta.as_ref().map(ScanEta::text).unwrap_or_default();
            eprint!("[{i}/{total}] analysing… {left}");
            let _ = std::io::stderr().flush();
        }
    };

    let workers = jobs.min(todo.len()).max(1);
    let next = AtomicUsize::new(0);
    std::thread::scope(|s| {
        let (tx, rx) = mpsc::channel::<(usize, FileResult)>();
        for _ in 0..workers {
            let tx = tx.clone();
            let (todo, files, next, cfg) = (&todo, &files, &next, &cfg);
            s.spawn(move || loop {
                let k = next.fetch_add(1, Ordering::SeqCst);
                let Some(&i) = todo.get(k) else { break };
                let r = analyze_file(&files[i], max_seconds, keep_spectrum, keep_plot, Some(cfg));
                if tx.send((i, r)).is_err() {
                    break;
                }
            });
        }
        drop(tx);
        let mut pending: BTreeMap<usize, FileResult> = BTreeMap::new();
        for i in 0..files.len() {
            let r = match ready.remove(&i) {
                Some(r) => r,
                None => loop {
                    if let Some(r) = pending.remove(&i) {
                        break r;
                    }
                    match rx.recv() {
                        Ok((j, r)) => {
                            pending.insert(j, r);
                        }
                        Err(_) => break FileResult::new(&abspath(&files[i]), Verdict::Error),
                    }
                },
            };
            handle(i + 1, r, &mut results, &mut eta);
        }
    });
    if show_progress {
        eprint!("\r\x1b[K");
    }

    println!();
    println!("{}", summary(&results, color));
    println!("{}", summary_bars(&results, color, 30));
    if store.is_some() {
        let reused: Vec<&FileResult> = results.iter().filter(|r| r.saved.is_some()).collect();
        println!(
            "Saved results: {} file(s) unchanged since an earlier scan were not analysed again, {} analysed. (--rescan \
             analyses everything again.)",
            reused.len(),
            results.len() - reused.len()
        );
        let old = reused.iter().filter(|r| r.saved.as_ref().is_some_and(|s| s.outdated)).count();
        if old > 0 {
            println!(
                "{old} of them were checked by older detection rules; --rescan-outdated analyses them with the current \
                 rules."
            );
        }
    }
    if let Some(p) = &args.json {
        match write_json(&results, p) {
            Ok(()) => println!("JSON written to {p}"),
            Err(e) => eprintln!("error: cannot write {p}: {e}"),
        }
    }
    if let Some(p) = &args.csv {
        match write_csv(&results, p) {
            Ok(()) => println!("CSV written to {p}"),
            Err(e) => eprintln!("error: cannot write {p}: {e}"),
        }
    }
    if let Some(p) = &args.plot {
        println!("Spectrum plots written to {p}/");
    }
    if let Some(p) = &args.spectrogram {
        println!("Spectrograms written to {p}/");
    }
    let mut rc = if args.fix { run_fixes(&results, &cfg, jobs, args.dry_run, store.as_mut()) } else { 0 };
    if args.lyrics {
        let paths: Vec<String> = results.iter().map(|r| r.path.clone()).collect();
        let by_path: HashMap<String, FileResult> = results.iter().map(|r| (r.path.clone(), r.clone())).collect();
        rc = rc.max(run_lyrics_cli(&paths, &cfg, args.dry_run, args.verbose, &by_path));
    }
    rc
}

/// Download synced lyrics for `paths` (--lyrics, --lyrics-only), one song at a time.
fn run_lyrics_cli(
    paths: &[String],
    cfg: &Settings,
    dry_run: bool,
    verbose: bool,
    results: &HashMap<String, FileResult>,
) -> i32 {
    // Ctrl+C stops after the current song, as in the Python version; a second one quits at once
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let _ = ctrlc::set_handler(move || {
        if flag.swap(true, Ordering::SeqCst) {
            std::process::exit(130);
        }
    });
    let mut client = LrclibClient::new(&cfg.lyrics_server).with_stop(stop.clone());
    let todo: Vec<&String> = paths.iter().filter(|p| skip_reason(p, cfg).is_empty()).collect();
    println!();
    let skipped = paths.len() - todo.len();
    let note = if skipped > 0 {
        let (has, is) = if skipped == 1 { ("has", "is") } else { ("have", "are") };
        format!("; {skipped} already {has} lyrics and {is} skipped")
    } else {
        String::new()
    };
    println!(
        "Lyrics: looking up {} song(s) on LRCLIB ({}), one at a time{note}{}",
        todo.len(),
        client.server,
        if dry_run { " (dry run: nothing is saved)." } else { "." }
    );
    let _ = std::io::stdout().flush();
    let show_progress = std::io::stderr().is_terminal();
    let mut eta = show_progress.then(|| ScanEta::new(todo.iter().map(|p| (p.to_string(), 1u64)).collect()));
    let mut outcomes: Vec<LyricsOutcome> = vec![];
    run_lyrics(paths, cfg, results, dry_run, &mut client, |o| {
        if show_progress {
            eprint!("\r\x1b[K");
        }
        if let Some(e) = eta.as_mut() {
            e.add(&o.path);
        }
        if o.status != Status::Skipped || verbose {
            let name = o.track.as_ref().map(|t| t.label()).unwrap_or_else(|| file_name(&o.path));
            println!("  {:<20} {name}\n  {:<20} {}", o.status.label(), "", o.message);
            let _ = std::io::stdout().flush();
        }
        outcomes.push(o);
        if show_progress && outcomes.len() < paths.len() {
            let left = eta.as_ref().map(ScanEta::text).unwrap_or_default();
            eprint!("[{}/{}] looking up lyrics… {left}", outcomes.len(), paths.len());
            let _ = std::io::stderr().flush();
        }
    });
    if show_progress {
        eprint!("\r\x1b[K");
    }
    if stop.load(Ordering::SeqCst) {
        println!("Lyrics: stopped after {} of {} song(s): {}", outcomes.len(), paths.len(), lyrics_summary(&outcomes));
        return 130;
    }
    println!("Lyrics: {}", lyrics_summary(&outcomes));
    if outcomes.iter().any(|o| o.status == Status::Failed) {
        1
    } else {
        0
    }
}

fn run_fixes(
    results: &[FileResult],
    cfg: &Settings,
    jobs: usize,
    dry_run: bool,
    store: Option<&mut ResultStore>,
) -> i32 {
    let (mut plans, skipped) = plan_fixes(results, cfg);
    println!();
    if !skipped.is_empty() {
        println!("Fix: {}", skip_summary(&skipped));
    }
    if plans.is_empty() {
        println!("Fix: nothing to convert (no fake hi-res / fake lossless files that can be reduced).");
        return 0;
    }
    let action = if plans[0].replace_original { "fix in place" } else { "save corrected copies of" };
    println!("Fix: {}{action} {} file(s):", if dry_run { "would " } else { "going to " }, plans.len());
    if dry_run {
        for p in &plans {
            let dest =
                if p.replace_original && p.output == p.source { "(same file)".to_string() } else { p.output.clone() };
            println!("  {:<22} {}\n  {:<22} -> {dest}", p.description, p.source, "");
            if p.replace_original {
                let what = match p.backup {
                    Backup::Folder => format!("moved to {}", p.backup_dir.clone().unwrap_or_default()),
                    Backup::Bak => "kept as .bak".into(),
                    Backup::None => "deleted".into(),
                };
                println!("  {:<22}    original: {what}", "");
            }
        }
        return 0;
    }
    // Read the joins between album neighbours before any file is replaced.
    let joins = prepare_gapless(&mut plans, results, &mut |m| println!("Fix: {m}"));
    if joins > 0 {
        println!(
            "Fix: {joins} gapless join(s) found; those tracks are converted together with their neighbours so the \
             joins stay seamless."
        );
    }
    let outcomes = run_parallel(&plans, jobs.clamp(1, 16), |p| run_fix(p, cfg));
    let skey = settings_key(cfg);
    let mut store = store;
    let mut failed = 0;
    for out in &outcomes {
        failed += (!out.ok) as usize;
        if let (Some(st), Some(new)) = (store.as_deref_mut(), out.new_result.as_ref()) {
            st.put(new, &skey, None, None); // the fixed file is known from now on
        }
        println!("  {} {}\n       {}", if out.ok { "OK  " } else { "FAIL" }, out.plan.source, out.message);
    }
    println!("Fix: {} fixed, {failed} failed.", plans.len() - failed);
    if failed > 0 {
        1
    } else {
        0
    }
}
