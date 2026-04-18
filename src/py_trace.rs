use crate::metrics::SampleColumns;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

// ── setup ────────────────────────────────────────────────────────────────

pub struct PyTraceSetup {
    pub trace_path: PathBuf,
    site_dir: PathBuf,
}

pub fn setup_python_tracing(
    cmd: &mut Command,
    program: &str,
    py_filter: Option<&str>,
) -> Option<PyTraceSetup> {
    let is_python = matches!(
        Path::new(program)
            .file_stem()
            .and_then(|s| s.to_str()),
        Some(s) if s == "python" || s == "python3" || s.starts_with("python3.")
    );

    if !is_python {
        return None;
    }

    let pid = std::process::id();
    let trace_path = std::env::temp_dir().join(format!("viy_py_{}.trace", pid));
    let site_dir = std::env::temp_dir().join(format!("viy_py_{}_site", pid));

    std::fs::create_dir_all(&site_dir).ok()?;

    let script_path = site_dir.join("sitecustomize.py");
    let script = injector_script(trace_path.to_str()?, py_filter);
    std::fs::write(&script_path, script).ok()?;

    let pythonpath = std::env::var("PYTHONPATH").unwrap_or_default();
    let new_path = if pythonpath.is_empty() {
        site_dir.to_string_lossy().into_owned()
    } else {
        format!("{}:{}", site_dir.to_string_lossy(), pythonpath)
    };
    cmd.env("PYTHONPATH", new_path);

    Some(PyTraceSetup {
        trace_path,
        site_dir,
    })
}

pub fn cleanup(setup: &PyTraceSetup) {
    let _ = std::fs::remove_file(setup.site_dir.join("sitecustomize.py"));
    let _ = std::fs::remove_dir(&setup.site_dir);
    let _ = std::fs::remove_file(&setup.trace_path);
}

// ── injector script ──────────────────────────────────────────────────────

/// `py_filter` is an optional glob pattern (e.g. `"*test_workload.py"`) —
/// when set, only frames whose filename matches are recorded.
pub fn injector_script(trace_path: &str, py_filter: Option<&str>) -> String {
    let escaped_path = trace_path.replace('\\', "\\\\").replace('"', "\\\"");

    let (import_line, filter_line) = match py_filter {
        Some(pat) => {
            let ep = pat.replace('\\', "\\\\").replace('"', "\\\"");
            (
                "from fnmatch import fnmatch as _fnmatch".to_string(),
                format!("    if not _fnmatch(frame.f_code.co_filename, \"{ep}\"): return _tracer"),
            )
        }
        None => (String::new(), String::new()),
    };

    format!(
        r#"import sys as _sys, time as _time, atexit as _atexit, threading as _threading
{import_line}

_t0 = _time.monotonic()
_TRACE_PATH = "{path}"
_local = _threading.local()

def _tracer(frame, event, arg):
    if event not in ('call', 'return'):
        return _tracer
{filter_line}
    ms = (_time.monotonic() - _t0) * 1000.0
    mod = frame.f_globals.get('__name__', '') or ''
    fn  = frame.f_code.co_qualname if hasattr(frame.f_code, 'co_qualname') else frame.f_code.co_name
    fil = frame.f_code.co_filename
    buf = _local.__dict__.setdefault('buf', [])
    buf.append(f"{{ms:.1f}},{{event}},{{mod}},{{fn}},{{fil}}\n")
    if len(buf) >= 200:
        _flush_local(buf)
    return _tracer

def _flush_local(buf):
    try:
        with open(_TRACE_PATH, 'a') as _f:
            _f.writelines(buf)
        buf.clear()
    except Exception:
        pass

_orig_run = _threading.Thread.run
def _patched_run(self):
    try:
        _orig_run(self)
    finally:
        buf = _local.__dict__.get('buf')
        if buf:
            _flush_local(buf)
_threading.Thread.run = _patched_run

def _flush_all():
    buf = _local.__dict__.get('buf')
    if buf:
        _flush_local(buf)

_atexit.register(_flush_all)
_sys.settrace(_tracer)
_threading.settrace(_tracer)
"#,
        path = escaped_path,
        import_line = import_line,
        filter_line = filter_line,
    )
}

// ── trace parsing ────────────────────────────────────────────────────────

#[derive(Debug)]
struct FnSpan {
    qualname: String,
    module: String,
    start_ms: f64,
    end_ms: f64,
}

fn parse_trace(path: &Path) -> Vec<FnSpan> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return vec![];
    };

    let mut stack: Vec<(String, String, String, f64)> = Vec::new();
    let mut spans: Vec<FnSpan> = Vec::new();

    for line in content.lines() {
        let parts: Vec<&str> = line.splitn(5, ',').collect();
        if parts.len() != 5 {
            continue;
        }
        let Ok(ms) = parts[0].parse::<f64>() else {
            continue;
        };
        let event = parts[1];
        let module = parts[2].to_string();
        let qualname = parts[3].to_string();
        let file = parts[4].to_string();

        match event {
            "call" => stack.push((module, qualname, file, ms)),
            "return" => {
                if let Some(pos) = stack
                    .iter()
                    .rposition(|(m, q, _, _)| *m == module && *q == qualname)
                {
                    let (m, q, _f, start) = stack.remove(pos);
                    spans.push(FnSpan {
                        qualname: q,
                        module: m,
                        start_ms: start,
                        end_ms: ms,
                    });
                }
            }
            _ => {}
        }
    }

    spans
}

// ── timeline ─────────────────────────────────────────────────────────────

/// One row in the Python timeline table.
#[derive(Debug, Clone)]
pub struct PyTimelineRow {
    /// Short display label: "qualname (file.py)"
    pub label: String,
    /// Dominant resource tag: "CPU", "MEM", "I/O", or ""
    pub tag: String,
    /// Bar cells across the timeline width — each char is one of " ░▒▓█"
    pub bar: String,
    /// Human-readable peak value for the dominant resource
    pub peak: String,
}

/// Build timeline rows for the top functions by total active duration.
/// `bar_width` is the number of character columns for the bar.
/// `py_epoch_offset_ms` is the sampler timestamp (ms) of the first sample —
/// py trace timestamps are relative to Python startup which lags the sampler
/// start, so we shift py timestamps by subtracting this offset.
pub fn build_timeline(
    samples: &SampleColumns,
    trace_path: &Path,
    wall_time: Duration,
    py_filter: Option<&str>,
    bar_width: usize,
    py_top: usize,
    py_epoch_offset_ms: f64,
) -> Vec<PyTimelineRow> {
    if samples.len() < 2 || bar_width == 0 {
        return vec![];
    }

    let all_spans = parse_trace(trace_path);
    if all_spans.is_empty() {
        return vec![];
    }

    let wall_ms = wall_time.as_secs_f64() * 1000.0;

    // Keep only non-trivial spans, shifting py timestamps to sampler clock
    let spans: Vec<&FnSpan> = all_spans
        .iter()
        .filter(|s| {
            let dur = s.end_ms - s.start_ms;
            !s.qualname.starts_with('<') && dur < wall_ms * 0.85
        })
        .filter(|s| py_filter.is_some() || !is_stdlib_noise(&s.qualname))
        .collect();

    // Shift helper: convert py trace ms to sampler-clock ms
    let py_to_sampler = |ms: f64| ms + py_epoch_offset_ms;

    if spans.is_empty() {
        return vec![];
    }

    // Aggregate total active duration per (qualname, module) key
    let mut totals: std::collections::HashMap<(&str, &str), f64> = std::collections::HashMap::new();
    for s in &spans {
        *totals.entry((&s.qualname, &s.module)).or_default() += s.end_ms - s.start_ms;
    }

    // Pick top N by total duration, then sort by first appearance for timeline order
    let mut ranked: Vec<_> = totals.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    ranked.truncate(py_top);

    // Sort by earliest span start time, then by latest span end time (sampler clock)
    ranked.sort_by(|a, b| {
        let first_a = spans
            .iter()
            .filter(|s| s.qualname == a.0.0 && s.module == a.0.1)
            .map(|s| py_to_sampler(s.start_ms))
            .fold(f64::MAX, f64::min);
        let first_b = spans
            .iter()
            .filter(|s| s.qualname == b.0.0 && s.module == b.0.1)
            .map(|s| py_to_sampler(s.start_ms))
            .fold(f64::MAX, f64::min);
        first_a.partial_cmp(&first_b).unwrap().then_with(|| {
            let last_a = spans
                .iter()
                .filter(|s| s.qualname == a.0.0 && s.module == a.0.1)
                .map(|s| py_to_sampler(s.end_ms))
                .fold(f64::MIN, f64::max);
            let last_b = spans
                .iter()
                .filter(|s| s.qualname == b.0.0 && s.module == b.0.1)
                .map(|s| py_to_sampler(s.end_ms))
                .fold(f64::MIN, f64::max);
            last_a.partial_cmp(&last_b).unwrap()
        })
    });

    // Pre-compute per-sample CPU rate and I/O rate for resource scoring.
    // Use actual sample timestamps for bucket boundaries.
    let tps = procfs::ticks_per_second() as f64;
    let n = samples.len();
    // rates[i] covers interval (timestamp[i], timestamp[i+1]), midpoint in sampler ms
    let cpu_rates: Vec<(f64, f64)> = (0..n.saturating_sub(1))
        .map(|i| {
            let dt = (samples.timestamp[i + 1] - samples.timestamp[i]).as_secs_f64();
            let mid_ms =
                (samples.timestamp[i].as_secs_f64() + samples.timestamp[i + 1].as_secs_f64()) / 2.0
                    * 1000.0;
            let rate = if dt > 0.0 {
                ((samples.utime_ticks[i + 1] + samples.stime_ticks[i + 1])
                    .saturating_sub(samples.utime_ticks[i] + samples.stime_ticks[i]))
                    as f64
                    / tps
                    / dt
                    * 100.0
            } else {
                0.0
            };
            (mid_ms, rate)
        })
        .collect();
    let io_rates: Vec<(f64, f64)> = (0..n.saturating_sub(1))
        .map(|i| {
            let dt = (samples.timestamp[i + 1] - samples.timestamp[i]).as_secs_f64();
            let mid_ms =
                (samples.timestamp[i].as_secs_f64() + samples.timestamp[i + 1].as_secs_f64()) / 2.0
                    * 1000.0;
            let rate = if dt > 0.0 {
                (samples.rchar[i + 1].saturating_sub(samples.rchar[i])
                    + samples.wchar[i + 1].saturating_sub(samples.wchar[i])) as f64
                    / dt
            } else {
                0.0
            };
            (mid_ms, rate)
        })
        .collect();
    // rss indexed by sample (not interval)
    let rss_vals: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            (
                samples.timestamp[i].as_secs_f64() * 1000.0,
                samples.rss_bytes[i] as f64,
            )
        })
        .collect();

    let max_cpu = cpu_rates
        .iter()
        .map(|(_, r)| *r)
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let max_io = io_rates
        .iter()
        .map(|(_, r)| *r)
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let min_rss = rss_vals.iter().map(|(_, v)| *v).fold(f64::MAX, f64::min);
    let max_rss = rss_vals
        .iter()
        .map(|(_, v)| *v)
        .fold(0.0_f64, f64::max)
        .max(min_rss + 1.0);

    const SHADES: [char; 5] = [' ', '░', '▒', '▓', '█'];

    let mut rows = Vec::new();

    for ((qualname, module), _total_dur) in &ranked {
        // Collect all spans for this function
        let fn_spans: Vec<&FnSpan> = spans
            .iter()
            .copied()
            .filter(|s| s.qualname == *qualname && s.module == *module)
            .collect();

        // Use actual sample timestamps for bar bucket boundaries.
        // Each bar column covers an equal slice of wall_ms.
        let bucket_ms = wall_ms / bar_width as f64;
        let mut cpu_score = 0.0_f64;
        let mut mem_score = 0.0_f64;
        let mut io_score = 0.0_f64;
        let mut bar_chars = Vec::with_capacity(bar_width);

        for b in 0..bar_width {
            let lo = b as f64 * bucket_ms;
            let hi = lo + bucket_ms;

            // Fraction of this bucket covered by any span (using sampler-clock ms)
            let covered: f64 = fn_spans
                .iter()
                .map(|s| {
                    let s_lo = py_to_sampler(s.start_ms);
                    let s_hi = py_to_sampler(s.end_ms);
                    (s_hi.min(hi) - s_lo.max(lo)).max(0.0)
                })
                .sum::<f64>()
                / bucket_ms;
            let covered = covered.min(1.0);

            if covered < 0.01 {
                bar_chars.push(' ');
                continue;
            }

            // Find resource values for samples whose timestamp falls in this bucket
            let avg_cpu = avg_in_range(&cpu_rates, lo, hi);
            let avg_io = avg_in_range(&io_rates, lo, hi);
            let avg_rss = avg_in_range(&rss_vals, lo, hi);

            // Accumulate scores for dominant-resource detection (normalised 0..1)
            let norm_rss = (avg_rss - min_rss) / (max_rss - min_rss);
            cpu_score += (avg_cpu / max_cpu) * covered;
            mem_score += norm_rss * covered;
            io_score += (avg_io / max_io) * covered;

            // Shade intensity = coverage * normalised dominant resource
            let intensity = covered * (avg_cpu / max_cpu).max(avg_io / max_io).max(norm_rss);
            let idx = ((intensity * 4.0).round() as usize).min(4);
            bar_chars.push(SHADES[idx]);
        }

        // Dominant resource tag and peak value (using sampler-clock ms for span matching)
        let (tag, peak) = if cpu_score >= mem_score && cpu_score >= io_score {
            let peak_cpu = cpu_rates
                .iter()
                .filter(|(mid, _)| {
                    fn_spans.iter().any(|s| {
                        py_to_sampler(s.start_ms) <= *mid && py_to_sampler(s.end_ms) >= *mid
                    })
                })
                .map(|(_, r)| *r)
                .fold(0.0_f64, f64::max);
            ("CPU", format!("{:.0}%", peak_cpu))
        } else if io_score >= mem_score {
            let peak_io = io_rates
                .iter()
                .filter(|(mid, _)| {
                    fn_spans.iter().any(|s| {
                        py_to_sampler(s.start_ms) <= *mid && py_to_sampler(s.end_ms) >= *mid
                    })
                })
                .map(|(_, r)| *r)
                .fold(0.0_f64, f64::max);
            (
                "I/O",
                format!("{}/s", crate::insights::format_bytes(peak_io as u64)),
            )
        } else {
            let peak_rss = rss_vals
                .iter()
                .filter(|(ts, _)| {
                    fn_spans
                        .iter()
                        .any(|s| py_to_sampler(s.start_ms) <= *ts && py_to_sampler(s.end_ms) >= *ts)
                })
                .map(|(_, v)| *v)
                .fold(0.0_f64, f64::max);
            ("MEM", crate::insights::format_bytes(peak_rss as u64))
        };

        let label = format!("{} ({})", qualname, module);

        let bar: String = bar_chars.into_iter().collect();
        if bar.chars().all(|c| c == ' ') {
            continue;
        }

        rows.push(PyTimelineRow {
            label,
            tag: tag.to_string(),
            bar,
            peak,
        });
    }

    rows
}

// ── correlation ──────────────────────────────────────────────────────────

pub fn correlate(
    samples: &SampleColumns,
    trace_path: &Path,
    wall_time: Duration,
    py_filter: Option<&str>,
    py_epoch_offset_ms: f64,
) -> Vec<String> {
    if samples.len() < 2 {
        return vec![];
    }

    let all_spans = parse_trace(trace_path);
    if all_spans.is_empty() {
        return vec![];
    }

    let wall_ms = wall_time.as_secs_f64() * 1000.0;

    // Drop spans that cover most of the run (e.g. <module>, top-level wrappers) —
    // they overlap every peak and tell us nothing useful.
    let spans: Vec<&FnSpan> = all_spans
        .iter()
        .filter(|s| {
            let dur = s.end_ms - s.start_ms;
            !s.qualname.starts_with('<') && dur < wall_ms * 0.7
        })
        .collect();

    if spans.is_empty() {
        return vec![];
    }

    // For a given sample timestamp (ms), find the best matching function:
    // the shortest span that contains that point (most specific call frame),
    // preferring user code over stdlib when py_filter is not set.
    let fn_at = |ts_ms: f64| -> Option<String> {
        // ts_ms is in sampler clock; convert to py trace clock for span lookup
        let py_ms = ts_ms - py_epoch_offset_ms;
        let mut candidates: Vec<&FnSpan> = spans
            .iter()
            .copied()
            .filter(|s| s.start_ms <= py_ms && s.end_ms >= py_ms)
            .collect();

        if candidates.is_empty() {
            candidates = spans
                .iter()
                .copied()
                .filter(|s| {
                    let mid = (s.start_ms + s.end_ms) / 2.0;
                    (mid - py_ms).abs() < 500.0
                })
                .collect();
        }

        if candidates.is_empty() {
            return None;
        }

        // Prefer user code when no filter is set
        if py_filter.is_none() {
            let user: Vec<_> = candidates
                .iter()
                .copied()
                .filter(|s| !is_stdlib_noise(&s.qualname))
                .collect();
            if !user.is_empty() {
                candidates = user;
            }
        }

        // Pick the shortest (most specific) span
        candidates.sort_by(|a, b| {
            let da = a.end_ms - a.start_ms;
            let db = b.end_ms - b.start_ms;
            da.partial_cmp(&db).unwrap()
        });

        let s = candidates[0];
        Some(format!("{} ({})", s.qualname, s.module))
    };

    let tps = procfs::ticks_per_second() as f64;
    let n = samples.len();

    // ── CPU peak ────────────────────────────────────────────────────────
    let cpu_rates: Vec<(f64, f64)> = (0..n.saturating_sub(1))
        .map(|i| {
            let dt = (samples.timestamp[i + 1] - samples.timestamp[i]).as_secs_f64();
            let mid_ms =
                (samples.timestamp[i].as_secs_f64() + samples.timestamp[i + 1].as_secs_f64()) / 2.0
                    * 1000.0;
            let rate = if dt > 0.0 {
                ((samples.utime_ticks[i + 1] + samples.stime_ticks[i + 1])
                    .saturating_sub(samples.utime_ticks[i] + samples.stime_ticks[i]))
                    as f64
                    / tps
                    / dt
                    * 100.0
            } else {
                0.0
            };
            (mid_ms, rate)
        })
        .collect();

    let mut insights = Vec::new();

    if let Some(&(peak_ms, peak_pct)) = cpu_rates
        .iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        && peak_pct > 50.0
        && let Some(f) = fn_at(peak_ms)
    {
        insights.push(format!("[PY]   CPU peak ({:.0}%) → {}", peak_pct, f));
    }

    // ── Memory peak ─────────────────────────────────────────────────────
    if let Some(peak_sample_idx) = samples
        .rss_bytes
        .iter()
        .enumerate()
        .max_by_key(|&(_, &v)| v)
        .map(|(i, _)| i)
        && samples.rss_bytes[peak_sample_idx]
            > samples
                .rss_bytes
                .first()
                .copied()
                .unwrap_or(0)
                .saturating_mul(2)
    {
        // peak_ms is in sampler clock
        let peak_ms = samples.timestamp[peak_sample_idx].as_secs_f64() * 1000.0;
        if let Some(f) = fn_at(peak_ms) {
            insights.push(format!(
                "[PY]   Memory peak ({}) → {}",
                crate::insights::format_bytes(samples.rss_bytes[peak_sample_idx]),
                f
            ));
        }
    }

    // ── I/O peak ────────────────────────────────────────────────────────
    let io_rates: Vec<(f64, f64)> = (0..n.saturating_sub(1))
        .map(|i| {
            let dt = (samples.timestamp[i + 1] - samples.timestamp[i]).as_secs_f64();
            let mid_ms =
                (samples.timestamp[i].as_secs_f64() + samples.timestamp[i + 1].as_secs_f64()) / 2.0
                    * 1000.0;
            let rate = if dt > 0.0 {
                (samples.rchar[i + 1].saturating_sub(samples.rchar[i])
                    + samples.wchar[i + 1].saturating_sub(samples.wchar[i])) as f64
                    / dt
            } else {
                0.0
            };
            (mid_ms, rate)
        })
        .collect();

    if let Some(&(peak_ms, peak_rate)) = io_rates
        .iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        && peak_rate > 1024.0 * 1024.0
        && let Some(f) = fn_at(peak_ms)
    {
        insights.push(format!(
            "[PY]   I/O peak ({}/s) → {}",
            crate::insights::format_bytes(peak_rate as u64),
            f
        ));
    }

    insights
}

fn is_stdlib_noise(qualname: &str) -> bool {
    qualname.starts_with('<')
}

/// Average the values in `data` (timestamp_ms, value) pairs whose timestamp falls in [lo, hi).
/// Falls back to the nearest point if none fall in range.
fn avg_in_range(data: &[(f64, f64)], lo: f64, hi: f64) -> f64 {
    let in_range: Vec<f64> = data
        .iter()
        .filter(|(t, _)| *t >= lo && *t < hi)
        .map(|(_, v)| *v)
        .collect();
    if !in_range.is_empty() {
        return in_range.iter().sum::<f64>() / in_range.len() as f64;
    }
    // nearest
    let mid = (lo + hi) / 2.0;
    data.iter()
        .min_by_key(|(t, _)| ((*t - mid).abs() * 1e6) as u64)
        .map(|(_, v)| *v)
        .unwrap_or(0.0)
}

// ── unit tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_trace(lines: &[&str]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        for line in lines {
            writeln!(f, "{}", line).unwrap();
        }
        f
    }

    #[test]
    fn parse_trace_basic_span() {
        let f = write_trace(&[
            "0.0,call,__main__,my_fn,/app/script.py",
            "50.0,return,__main__,my_fn,/app/script.py",
        ]);
        let spans = parse_trace(f.path());
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].qualname, "my_fn");
        assert_eq!(spans[0].module, "__main__");
        assert!((spans[0].start_ms - 0.0).abs() < 0.1);
        assert!((spans[0].end_ms - 50.0).abs() < 0.1);
    }

    #[test]
    fn parse_trace_nested_spans() {
        let f = write_trace(&[
            "0.0,call,__main__,outer,/app/script.py",
            "10.0,call,__main__,inner,/app/script.py",
            "20.0,return,__main__,inner,/app/script.py",
            "30.0,return,__main__,outer,/app/script.py",
        ]);
        let spans = parse_trace(f.path());
        assert_eq!(spans.len(), 2);
        let inner = spans.iter().find(|s| s.qualname == "inner").unwrap();
        let outer = spans.iter().find(|s| s.qualname == "outer").unwrap();
        assert!((inner.end_ms - inner.start_ms - 10.0).abs() < 0.1);
        assert!((outer.end_ms - outer.start_ms - 30.0).abs() < 0.1);
    }

    #[test]
    fn parse_trace_missing_return_is_ignored() {
        let f = write_trace(&["0.0,call,__main__,orphan,/app/script.py"]);
        let spans = parse_trace(f.path());
        assert_eq!(spans.len(), 0);
    }

    #[test]
    fn parse_trace_empty_file() {
        let f = write_trace(&[]);
        assert!(parse_trace(f.path()).is_empty());
    }

    #[test]
    fn parse_trace_malformed_lines_skipped() {
        let f = write_trace(&[
            "not,valid",
            "0.0,call,__main__,fn,/f.py",
            "10.0,return,__main__,fn,/f.py",
        ]);
        let spans = parse_trace(f.path());
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn injector_script_no_filter_contains_settrace() {
        let script = injector_script("/tmp/trace.out", None);
        assert!(script.contains("sys.settrace"));
        assert!(script.contains("threading.settrace"));
        assert!(script.contains("/tmp/trace.out"));
        assert!(!script.contains("fnmatch"));
    }

    #[test]
    fn injector_script_with_filter_contains_fnmatch() {
        let script = injector_script("/tmp/trace.out", Some("*myapp.py"));
        assert!(script.contains("fnmatch"));
        assert!(script.contains("*myapp.py"));
    }
}
