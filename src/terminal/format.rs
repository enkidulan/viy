use owo_colors::OwoColorize;
use std::io::Write;
use std::time::Duration;

pub fn lbl(t: &str, uc: bool) -> String {
    if uc {
        format!("{}", t.bold())
    } else {
        t.to_string()
    }
}

pub fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s < 0.001 {
        format!("{:.0}us", s * 1_000_000.0)
    } else if s < 1.0 {
        format!("{:.1}ms", s * 1000.0)
    } else if s < 60.0 {
        format!("{:.2}s", s)
    } else if s < 3600.0 {
        format!("{:.0}m{:.1}s", (s / 60.0).floor(), s % 60.0)
    } else {
        let h = (s / 3600.0).floor();
        let m = ((s - h * 3600.0) / 60.0).floor();
        format!("{:.0}h{:.0}m{:.1}s", h, m, s % 60.0)
    }
}

pub fn fmt_num(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 10_000 {
        format!("{:.1}K", n as f64 / 1e3)
    } else {
        let s = n.to_string();
        let mut r = String::new();
        for (i, c) in s.chars().rev().enumerate() {
            if i > 0 && i % 3 == 0 {
                r.push(',');
            }
            r.push(c);
        }
        r.chars().rev().collect()
    }
}

pub fn colorize_insight(s: &str) -> String {
    if let Some(r) = s.strip_prefix("[CPU]") {
        format!(" {}{}", "[CPU]".cyan().bold(), r)
    } else if let Some(r) = s.strip_prefix("[MEM]") {
        format!(" {}{}", "[MEM]".green().bold(), r)
    } else if let Some(r) = s.strip_prefix("[I/O]") {
        format!(" {}{}", "[I/O]".yellow().bold(), r)
    } else if let Some(r) = s.strip_prefix("[THR]") {
        format!(" {}{}", "[THR]".magenta().bold(), r)
    } else if let Some(r) = s.strip_prefix("[PY]") {
        format!(" {}{}", "[PY] ".cyan().bold(), r)
    } else if let Some(r) = s.strip_prefix("[FD]") {
        format!(" {}{}", "[FD] ".blue().bold(), r)
    } else {
        format!(" {}", s)
    }
}

pub fn terminal_width() -> usize {
    term_size().map(|(w, _)| w.max(40)).unwrap_or(80)
}

fn term_size() -> Option<(usize, usize)> {
    use std::fs::File;
    use std::os::unix::io::AsRawFd;
    let fd = File::open("/dev/tty").ok()?;
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCGWINSZ, &mut ws) };
    if ret == 0 && ws.ws_col > 0 {
        Some((ws.ws_col as usize, ws.ws_row as usize))
    } else {
        None
    }
}

pub fn box_top(out: &mut impl Write, title: &str, w: usize, uc: bool) {
    let t = format!(" {} ", title);
    let fill = w.saturating_sub(3 + t.len());
    if uc {
        let _ = write!(out, "{}", "╭─".dimmed());
        let _ = write!(out, "{}", t.bold());
        let _ = write!(out, "{}", "─".repeat(fill).dimmed());
        let _ = writeln!(out, "{}", "╮".dimmed());
    } else {
        let _ = writeln!(out, "╭─{}{}╮", t, "─".repeat(fill));
    }
}

pub fn box_row(out: &mut impl Write, content: &str, w: usize, uc: bool) {
    let inner = w.saturating_sub(2);
    let pad = inner.saturating_sub(strip_ansi_len(content));
    if uc {
        let _ = write!(out, "{}", "│".dimmed());
    } else {
        let _ = write!(out, "│");
    }
    let _ = write!(out, "{}{:pad$}", content, "", pad = pad);
    if uc {
        let _ = writeln!(out, "{}", "│".dimmed());
    } else {
        let _ = writeln!(out, "│");
    }
}

pub fn box_bottom(out: &mut impl Write, w: usize, uc: bool) {
    let inner = w.saturating_sub(2);
    if uc {
        let _ = writeln!(
            out,
            "{}{}{}",
            "╰".dimmed(),
            "─".repeat(inner).dimmed(),
            "╯".dimmed()
        );
    } else {
        let _ = writeln!(out, "╰{}╯", "─".repeat(inner));
    }
}

pub fn strip_ansi_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_esc = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_esc = true;
        } else if in_esc {
            if c == 'm' {
                in_esc = false;
            }
        } else {
            len += 1;
        }
    }
    len
}
