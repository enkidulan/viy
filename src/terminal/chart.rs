use owo_colors::OwoColorize;
use std::io::Write;

const BLOCKS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
const CHART_HEIGHT: usize = 5;

/// Downsample a value series to at most `max_points` using min-max bucketing.
pub fn downsample(values: &[f64], max_points: usize) -> Vec<f64> {
    if values.len() <= max_points {
        return values.to_vec();
    }
    let bucket_size = values.len() as f64 / max_points as f64;
    let mut result = Vec::with_capacity(max_points);
    for i in 0..max_points {
        let start = (i as f64 * bucket_size) as usize;
        let end = (((i + 1) as f64 * bucket_size) as usize).min(values.len());
        let bucket = &values[start..end];
        if bucket.is_empty() {
            continue;
        }
        let val = if i % 2 == 0 {
            bucket.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
        } else {
            bucket.iter().cloned().fold(f64::INFINITY, f64::min)
        };
        result.push(val);
    }
    result
}

pub enum ChartColor {
    Cyan,
    Green,
    Yellow,
    Magenta,
    Blue,
}

pub struct Chart<'a> {
    pub title: &'a str,
    pub subtitle: &'a str,
    pub values: Vec<f64>,
    pub color: ChartColor,
    pub unit_formatter: Box<dyn Fn(f64) -> String + 'a>,
    pub duration_secs: f64,
}

pub fn render_chart(out: &mut impl Write, chart: &Chart, width: usize, use_color: bool) {
    if chart.values.is_empty() {
        return;
    }

    let inner = width.saturating_sub(2);
    let label_w = 10;
    let bar_w = inner.saturating_sub(label_w + 2).max(10);

    let values = downsample(&chart.values, bar_w);
    if values.is_empty() {
        return;
    }

    let max_val = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let min_val = values
        .iter()
        .cloned()
        .fold(f64::INFINITY, f64::min)
        .min(0.0);
    let range = if (max_val - min_val).abs() < f64::EPSILON {
        max_val.max(1.0)
    } else {
        max_val - min_val
    };

    let title_part = format!(" {} ", chart.title);
    let sub_part = if chart.subtitle.is_empty() {
        String::new()
    } else {
        format!(" {} ", chart.subtitle)
    };
    let used = 2 + title_part.len() + sub_part.len() + 1;
    let fill = width.saturating_sub(used);

    if use_color {
        let _ = write!(out, "{}", "╭─".dimmed());
        let _ = write!(out, "{}", title_part.bold());
        if !sub_part.is_empty() {
            let _ = write!(out, "{}", sub_part.dimmed());
        }
        let _ = write!(out, "{}", "─".repeat(fill).dimmed());
        let _ = writeln!(out, "{}", "╮".dimmed());
    } else {
        let _ = writeln!(out, "╭─{}{}{}╮", title_part, sub_part, "─".repeat(fill));
    }

    for row in (0..CHART_HEIGHT).rev() {
        dim_char(out, "│", use_color);

        let y_label = match row {
            r if r == CHART_HEIGHT - 1 => (chart.unit_formatter)(max_val),
            r if r == CHART_HEIGHT / 2 => (chart.unit_formatter)(min_val + range / 2.0),
            0 => (chart.unit_formatter)(min_val),
            _ => String::new(),
        };
        let _ = write!(out, "{:>w$} ", y_label, w = label_w);

        for &val in &values {
            let normalized = (val - min_val) / range;
            let total_units = (normalized * (CHART_HEIGHT as f64) * 8.0).round() as usize;
            let units_below = row * 8;

            let block = if total_units <= units_below {
                " "
            } else {
                let in_row = (total_units - units_below).min(8) - 1;
                BLOCKS[in_row]
            };

            color_str(out, block, &chart.color, use_color);
        }

        let pad = inner.saturating_sub(label_w + 1 + values.len());
        let _ = write!(out, "{:w$}", "", w = pad);
        dim_char(out, "│\n", use_color);
    }

    dim_char(out, "│", use_color);
    let _ = write!(out, "{:>w$} ", "", w = label_w);
    render_time_axis(out, bar_w, chart.duration_secs, use_color);
    let pad = inner.saturating_sub(label_w + 1 + bar_w);
    let _ = write!(out, "{:w$}", "", w = pad);
    dim_char(out, "│\n", use_color);

    let bot = "─".repeat(inner);
    if use_color {
        let _ = writeln!(out, "{}{}{}", "╰".dimmed(), bot.dimmed(), "╯".dimmed());
    } else {
        let _ = writeln!(out, "╰{}╯", bot);
    }
}

fn color_str(out: &mut impl Write, s: &str, color: &ChartColor, use_color: bool) {
    if !use_color || s.trim().is_empty() {
        let _ = write!(out, "{}", s);
        return;
    }
    let c = match color {
        ChartColor::Cyan => format!("{}", s.cyan()),
        ChartColor::Green => format!("{}", s.green()),
        ChartColor::Yellow => format!("{}", s.yellow()),
        ChartColor::Magenta => format!("{}", s.magenta()),
        ChartColor::Blue => format!("{}", s.blue()),
    };
    let _ = write!(out, "{}", c);
}

pub fn dim_char(out: &mut impl Write, s: &str, use_color: bool) {
    if use_color {
        let _ = write!(out, "{}", s.dimmed());
    } else {
        let _ = write!(out, "{}", s);
    }
}

fn render_time_axis(out: &mut impl Write, width: usize, duration_secs: f64, use_color: bool) {
    let mut line = vec![b' '; width];
    for i in 0..=4 {
        let frac = i as f64 / 4.0;
        let pos = ((width.saturating_sub(1)) as f64 * frac).round() as usize;
        let lbl = format_time_short(duration_secs * frac);
        let start = pos.saturating_sub(lbl.len() / 2);
        for (j, ch) in lbl.bytes().enumerate() {
            if start + j < width {
                line[start + j] = ch;
            }
        }
    }
    let s: String = line.iter().map(|&b| b as char).collect();
    if use_color {
        let _ = write!(out, "{}", s.dimmed());
    } else {
        let _ = write!(out, "{}", s);
    }
}

pub fn format_time_short(secs: f64) -> String {
    if secs < 0.001 {
        "0".into()
    } else if secs < 1.0 {
        format!("{:.0}ms", secs * 1000.0)
    } else if secs < 60.0 {
        format!("{:.1}s", secs)
    } else {
        format!("{:.0}m", secs / 60.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsample_preserves_small() {
        let values = vec![1.0, 2.0, 3.0];
        assert_eq!(downsample(&values, 10), values);
    }

    #[test]
    fn downsample_reduces() {
        let values: Vec<f64> = (0..100).map(|i| i as f64).collect();
        assert_eq!(downsample(&values, 20).len(), 20);
    }
}
