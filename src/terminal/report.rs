use crate::insights::format_bytes;
use crate::metrics::{ProcessResult, SampleColumns};
use crate::terminal::chart::{Chart, ChartColor, format_time_short, render_chart};
use crate::terminal::format::{
    box_bottom, box_row, box_top, colorize_insight, fmt_dur, fmt_num, lbl, terminal_width,
};
use std::io::{self, Write};

pub fn print_report(result: &ProcessResult, use_color: bool, quiet: bool) {
    let mut out = io::stderr().lock();
    let w = terminal_width();

    let _ = writeln!(out);
    print_header(&mut out, result, use_color, w);
    print_summary(&mut out, result, use_color, w);
    print_charts(&mut out, result, use_color, w);
    if result.py_trace_path.is_some() {
        print_py_timeline(&mut out, result, use_color, w);
    }

    if !quiet && (!result.insights.is_empty() || !result.py_insights.is_empty()) {
        print_insights(&mut out, result, use_color, w);
    }
    let _ = writeln!(out);
}

fn print_header(out: &mut impl Write, r: &ProcessResult, uc: bool, w: usize) {
    use owo_colors::OwoColorize;
    let exit_str = match (r.exit_code, r.signal) {
        (Some(0), _) => {
            if uc {
                format!("{}", "0".green())
            } else {
                "0".into()
            }
        }
        (Some(c), _) => {
            if uc {
                format!("{}", c.red())
            } else {
                c.to_string()
            }
        }
        (None, Some(s)) => {
            let m = format!("signal {}", s);
            if uc { format!("{}", m.red()) } else { m }
        }
        _ => "unknown".into(),
    };

    box_top(out, "viy", w, uc);
    box_row(
        out,
        &format!(" {:<13}{}", lbl("Command:", uc), r.command),
        w,
        uc,
    );
    box_row(
        out,
        &format!(" {:<13}{}", lbl("Exit code:", uc), exit_str),
        w,
        uc,
    );
    box_row(
        out,
        &format!(" {:<13}{}", lbl("Wall time:", uc), fmt_dur(r.wall_time)),
        w,
        uc,
    );
    box_bottom(out, w, uc);
}

fn print_summary(out: &mut impl Write, r: &ProcessResult, uc: bool, w: usize) {
    let s = &r.summary;
    box_top(out, "Summary", w, uc);

    let rows: Vec<(String, String)> = vec![
        (
            format!(
                " {:<13}{:.1}% (user {}, sys {})",
                lbl("CPU:", uc),
                s.cpu_percent,
                fmt_dur(s.user_time),
                fmt_dur(s.system_time)
            ),
            format!(
                "{:<13}{} -> {} (peak {})",
                lbl("Memory:", uc),
                format_bytes(s.initial_rss_bytes),
                format_bytes(s.final_rss_bytes),
                format_bytes(s.peak_rss_bytes)
            ),
        ),
        (
            format!(
                " {:<13}{} read, {} written",
                lbl("I/O:", uc),
                format_bytes(s.total_rchar),
                format_bytes(s.total_wchar)
            ),
            format!(
                "{:<13}{} in, {} out (host)",
                lbl("Network:", uc),
                format_bytes(s.total_net_rx_bytes),
                format_bytes(s.total_net_tx_bytes)
            ),
        ),
        (
            format!(
                " {:<13}{} vol, {} invol",
                lbl("Ctx switch:", uc),
                fmt_num(s.total_voluntary_ctxt_switches),
                fmt_num(s.total_nonvoluntary_ctxt_switches)
            ),
            format!(
                "{:<13}{} minor, {} major",
                lbl("Page faults:", uc),
                fmt_num(s.total_minflt),
                fmt_num(s.total_majflt)
            ),
        ),
        (
            format!(
                " {:<13}{}",
                lbl("Threads:", uc),
                if s.peak_threads > 1 || s.initial_threads != s.final_threads {
                    format!(
                        "{} -> {} (peak {})",
                        s.initial_threads, s.final_threads, s.peak_threads
                    )
                } else {
                    format!("{}", s.final_threads)
                }
            ),
            format!(
                "{:<13}{} (peak {})",
                lbl("File descs:", uc),
                s.final_fd_count,
                s.peak_fd_count
            ),
        ),
    ];

    for (left, right) in &rows {
        box_row(out, &format!("{}    {}", left, right), w, uc);
    }

    if s.child_processes_spawned > 0 {
        box_row(
            out,
            &format!(
                " {:<13}{} spawned",
                lbl("Processes:", uc),
                s.child_processes_spawned
            ),
            w,
            uc,
        );
    }

    box_bottom(out, w, uc);
}

fn print_charts(out: &mut impl Write, r: &ProcessResult, uc: bool, w: usize) {
    let samples = &r.samples;
    if samples.len() < 2 {
        return;
    }

    let dur = r.wall_time.as_secs_f64();
    let tps = procfs::ticks_per_second() as f64;

    render_chart(
        out,
        &Chart {
            title: "CPU Utilization (%)",
            subtitle: &format!("avg {:.1}%", r.summary.cpu_percent),
            values: compute_rate(samples, |i| {
                (samples.utime_ticks[i] + samples.stime_ticks[i]) as f64 / tps * 100.0
            }),
            color: ChartColor::Cyan,
            unit_formatter: Box::new(|v| format!("{:.0}%", v)),
            duration_secs: dur,
        },
        w,
        uc,
    );

    render_chart(
        out,
        &Chart {
            title: "Memory RSS",
            subtitle: &format!("peak {}", format_bytes(r.summary.peak_rss_bytes)),
            values: samples.rss_bytes.iter().map(|&v| v as f64).collect(),
            color: ChartColor::Green,
            unit_formatter: Box::new(|v| format_bytes(v.max(0.0) as u64)),
            duration_secs: dur,
        },
        w,
        uc,
    );

    render_chart(
        out,
        &Chart {
            title: "I/O Rate",
            subtitle: &format!(
                "{} read, {} written",
                format_bytes(r.summary.total_rchar),
                format_bytes(r.summary.total_wchar)
            ),
            values: compute_rate(samples, |i| (samples.rchar[i] + samples.wchar[i]) as f64),
            color: ChartColor::Yellow,
            unit_formatter: Box::new(|v| format!("{}/s", format_bytes(v.max(0.0) as u64))),
            duration_secs: dur,
        },
        w,
        uc,
    );

    render_chart(
        out,
        &Chart {
            title: "Network (host)",
            subtitle: &format!(
                "{} in, {} out",
                format_bytes(r.summary.total_net_rx_bytes),
                format_bytes(r.summary.total_net_tx_bytes)
            ),
            values: compute_rate(samples, |i| {
                (samples.net_rx_bytes[i] + samples.net_tx_bytes[i]) as f64
            }),
            color: ChartColor::Blue,
            unit_formatter: Box::new(|v| format!("{}/s", format_bytes(v.max(0.0) as u64))),
            duration_secs: dur,
        },
        w,
        uc,
    );

    if r.summary.peak_threads > 1 {
        render_chart(
            out,
            &Chart {
                title: "Threads",
                subtitle: &format!("peak {}", r.summary.peak_threads),
                values: samples.threads.iter().map(|&v| v as f64).collect(),
                color: ChartColor::Magenta,
                unit_formatter: Box::new(|v| format!("{:.0}", v)),
                duration_secs: dur,
            },
            w,
            uc,
        );
    }
}

fn compute_rate(samples: &SampleColumns, extractor: impl Fn(usize) -> f64) -> Vec<f64> {
    (0..samples.len().saturating_sub(1))
        .map(|i| {
            let dt = (samples.timestamp[i + 1] - samples.timestamp[i]).as_secs_f64();
            if dt > 0.0 {
                ((extractor(i + 1) - extractor(i)) / dt).max(0.0)
            } else {
                0.0
            }
        })
        .collect()
}

fn print_py_timeline(out: &mut impl Write, r: &ProcessResult, uc: bool, w: usize) {
    let label_w = 10usize;
    let inner = w.saturating_sub(2);
    let bar_w = inner.saturating_sub(label_w + 1).max(10);

    let Some(trace_path) = &r.py_trace_path else {
        return;
    };
    let rows = crate::py_trace::build_timeline(
        &r.samples,
        trace_path,
        r.wall_time,
        r.py_filter.as_deref(),
        bar_w,
        r.py_top,
        r.py_epoch_offset_ms,
    );
    if rows.is_empty() {
        return;
    }

    box_top(
        out,
        &format!("Python Timeline  \u{2192}  {}", fmt_dur(r.wall_time)),
        w,
        uc,
    );

    for row in &rows {
        let tag_str = format!("[{}]", row.tag);
        if uc {
            use owo_colors::OwoColorize;
            let colored_tag = match row.tag.as_str() {
                "CPU" => format!("{}", tag_str.cyan().bold()),
                "MEM" => format!("{}", tag_str.green().bold()),
                "I/O" => format!("{}", tag_str.yellow().bold()),
                _ => tag_str.clone(),
            };
            box_row(
                out,
                &format!(" {}  {}  {}", row.label.bold(), colored_tag, row.peak),
                w,
                uc,
            );
        } else {
            box_row(
                out,
                &format!(" {}  {}  {}", row.label, tag_str, row.peak),
                w,
                uc,
            );
        }

        let colored_bar: String = if uc {
            use owo_colors::OwoColorize;
            row.bar
                .chars()
                .map(|c| {
                    if c == ' ' {
                        c.to_string()
                    } else {
                        match row.tag.as_str() {
                            "CPU" => format!("{}", c.to_string().cyan()),
                            "MEM" => format!("{}", c.to_string().green()),
                            "I/O" => format!("{}", c.to_string().yellow()),
                            _ => c.to_string(),
                        }
                    }
                })
                .collect()
        } else {
            row.bar.clone()
        };

        box_row(
            out,
            &format!("{:>w$} {}", "", colored_bar, w = label_w),
            w,
            uc,
        );
    }

    // Time axis
    let mut time_axis = vec![b' '; bar_w];
    for i in 0..=4 {
        let frac = i as f64 / 4.0;
        let pos = ((bar_w.saturating_sub(1)) as f64 * frac).round() as usize;
        let lbl_str = format_time_short(r.wall_time.as_secs_f64() * frac);
        let start = pos.saturating_sub(lbl_str.len() / 2);
        for (j, b) in lbl_str.bytes().enumerate() {
            if start + j < bar_w {
                time_axis[start + j] = b;
            }
        }
    }
    let time_str: String = time_axis.iter().map(|&b| b as char).collect();
    let time_content = if uc {
        use owo_colors::OwoColorize;
        format!("{:>w$} {}", "", time_str.dimmed(), w = label_w)
    } else {
        format!("{:>w$} {}", "", time_str, w = label_w)
    };
    box_row(out, &time_content, w, uc);

    box_bottom(out, w, uc);
}

fn print_insights(out: &mut impl Write, r: &ProcessResult, uc: bool, w: usize) {
    box_top(out, "Insights", w, uc);
    for insight in r.insights.iter().chain(r.py_insights.iter()) {
        let text = if uc {
            colorize_insight(insight)
        } else {
            format!(" {}", insight)
        };
        box_row(out, &text, w, uc);
    }
    box_bottom(out, w, uc);
}
