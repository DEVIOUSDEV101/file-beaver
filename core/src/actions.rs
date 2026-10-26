//! Runs a ring action on dropped files and reports progress. One file failing never stops
//! the rest; originals are never modified.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use serde::Serialize;

use crate::compress;
use crate::convert::{self, name};
use crate::formats::Format;
use crate::metadata;
use crate::naming::{self, OutputDir};

#[derive(Serialize, Clone, Debug)]
pub struct Progress {
    /// 0.0 ..= 1.0
    pub fraction: f32,
    /// "1 of 3 file(s)" or "Page 2 of 12".
    pub detail: String,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Outcome {
    pub outputs: Vec<PathBuf>,
    pub summary: String,
    pub errors: Vec<String>,
    pub cancelled: bool,
}

impl Outcome {
    pub fn failed(msg: String) -> Outcome {
        Outcome { errors: vec![msg], summary: "Failed".into(), ..Default::default() }
    }

    pub fn cancelled() -> Outcome {
        Outcome { cancelled: true, summary: "Cancelled".into(), ..Default::default() }
    }
}

/// Whether an action runs straight away (vs. opening an editor window).
pub fn is_quick(action: &str) -> bool {
    action.starts_with("convert:") || action == "tool:compress" || action == "tool:metadata"
}

/// Card title while running, e.g. "Converting to JPG".
pub fn title(action: &str) -> String {
    match action {
        "tool:compress" => "Compressing".into(),
        "tool:metadata" => "Removing metadata".into(),
        a => match a.strip_prefix("convert:").and_then(Format::from_id) {
            Some(f) => format!("Converting to {}", f.label()),
            None => "Working".into(),
        },
    }
}

/// Card title when finished, e.g. "Converted to JPG".
pub fn done_title(action: &str, o: &Outcome) -> String {
    if o.cancelled {
        return "Cancelled".into();
    }
    if o.outputs.is_empty() {
        return if o.errors.is_empty() { "Nothing to do".into() } else { "Couldn't finish".into() };
    }
    match action {
        "tool:compress" => "Compressed".into(),
        "tool:metadata" => "Metadata removed".into(),
        a => match a.strip_prefix("convert:").and_then(Format::from_id) {
            Some(f) => format!("Converted to {}", f.label()),
            None => "Done".into(),
        },
    }
}

pub fn run(action: &str, paths: &[PathBuf], out: &OutputDir, cancel: &AtomicBool, progress: &mut (dyn FnMut(Progress) + Send)) -> Outcome {
    if let Some(id) = action.strip_prefix("convert:") {
        return match convert::target(id) {
            Ok(f) => convert::run(paths, f, out, cancel, progress),
            Err(e) => Outcome::failed(format!("{e:#}")),
        };
    }
    match action {
        "tool:compress" => per_file(paths, cancel, progress, |p| {
            let before = std::fs::metadata(p)?.len();
            match compress::compress(p)? {
                Some((bytes, fmt)) => {
                    let saved = naming::write_new(&naming::target_dir(p, out), &format!("{} Compressed", naming::stem(p)), fmt.ext(), &bytes)?;
                    Ok((Some(saved), (before, bytes.len() as u64)))
                }
                None => Ok((None, (before, before))),
            }
        })
        .map_summary(|o, sizes| {
            let (b, a): (u64, u64) = sizes.iter().fold((0, 0), |(x, y), (b, a)| (x + b, y + a));
            if o.outputs.is_empty() {
                "Already well compressed, nothing saved".into()
            } else {
                let pct = 100.0 * (1.0 - a as f64 / b as f64);
                format!("{} → {} (−{pct:.0}%)", compress::human(b), compress::human(a))
            }
        }),
        "tool:metadata" => per_file(paths, cancel, progress, |p| match metadata::strip(p)? {
            Some(s) => {
                let saved = naming::write_new(&naming::target_dir(p, out), &format!("{} Clean", naming::stem(p)), s.format.ext(), &s.bytes)?;
                Ok((Some(saved), (s.removed.join(", "), s.had_gps)))
            }
            None => Ok((None, (String::new(), false))),
        })
        .map_summary(|o, found| {
            let gps = found.iter().any(|(_, g)| *g);
            match (o.outputs.len(), found.iter().find(|(r, _)| !r.is_empty())) {
                (0, _) => "No metadata found, nothing saved".into(),
                (1, Some((what, _))) => format!("Removed {what}{}", if gps { ", incl. GPS location" } else { "" }),
                (n, _) => format!("Cleaned {n} files{}", if gps { ", incl. GPS location" } else { "" }),
            }
        }),
        other => Outcome::failed(format!("{other} isn't a quick action")),
    }
}

/// Outcome plus per-file results, for building the summary afterwards.
struct PerFile<T>(Outcome, Vec<T>);

impl<T> PerFile<T> {
    fn map_summary(self, f: impl FnOnce(&Outcome, &[T]) -> String) -> Outcome {
        let PerFile(mut o, results) = self;
        if o.cancelled {
            return o;
        }
        o.summary = if o.outputs.is_empty() && !o.errors.is_empty() {
            // Every file failed: say so, don't claim "nothing to do".
            format!("Couldn't process {} file(s)", o.errors.len())
        } else {
            f(&o, &results)
        };
        o
    }
}

/// Runs `each` on every file on the worker pool, in parallel. Results (and any
/// output file) are gathered back in the order the files were dropped.
fn per_file<T: Send>(
    paths: &[PathBuf],
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(Progress) + Send),
    each: impl Fn(&PathBuf) -> anyhow::Result<(Option<PathBuf>, T)> + Sync,
) -> PerFile<T> {
    use rayon::prelude::*;
    let done = AtomicUsize::new(0);
    let total = paths.len();
    progress(Progress { fraction: 0.0, detail: format!("1 of {total} file(s)") });
    let progress = Mutex::new(progress);
    let results: Vec<anyhow::Result<(Option<PathBuf>, T)>> = crate::pool::get().install(|| {
        paths
            .par_iter()
            .map(|p| {
                if cancel.load(Ordering::SeqCst) {
                    anyhow::bail!("cancelled");
                }
                let r = each(p);
                let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                if let Ok(mut report) = progress.try_lock() {
                    report(Progress { fraction: n as f32 / total as f32, detail: format!("{n} of {total} file(s)") });
                }
                r
            })
            .collect()
    });
    if cancel.load(Ordering::SeqCst) {
        return PerFile(Outcome::cancelled(), Vec::new());
    }
    let mut outcome = Outcome::default();
    let mut per = Vec::new();
    for (p, r) in paths.iter().zip(results) {
        match r {
            Ok((saved, data)) => {
                outcome.outputs.extend(saved);
                per.push(data);
            }
            Err(e) => outcome.errors.push(format!("{}: {e:#}", name(p))),
        }
    }
    PerFile(outcome, per)
}
