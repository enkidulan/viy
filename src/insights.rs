use crate::metrics::{SampleColumns, Summary};
use std::time::Duration;

/// Analyze collected metrics and produce human-readable insights.
pub fn generate_insights(
    summary: &Summary,
    samples: &SampleColumns,
    wall_time: Duration,
) -> Vec<String> {
    let mut insights = Vec::new();
    let wall_secs = wall_time.as_secs_f64();

    if wall_secs < 0.001 {
        return insights;
    }

    cpu_insights(summary, samples, wall_secs, &mut insights);
    io_insights(summary, wall_secs, &mut insights);
    thread_insights(summary, &mut insights);
    page_fault_insights(summary, wall_secs, &mut insights);

    if !samples.is_empty() {
        memory_insights(summary, samples, &mut insights);
        fd_insights(summary, samples, &mut insights);
    }

    insights
}

fn cpu_insights(
    summary: &Summary,
    _samples: &SampleColumns,
    wall_secs: f64,
    insights: &mut Vec<String>,
) {
    let total_cpu_secs = summary.user_time.as_secs_f64() + summary.system_time.as_secs_f64();

    // CPU-bound detection
    if total_cpu_secs / wall_secs > 0.9 {
        insights.push(format!(
            "[CPU]  CPU-bound workload ({:.0}% utilization)",
            summary.cpu_percent
        ));
    }

    // I/O-bound detection
    if total_cpu_secs / wall_secs < 0.5 && summary.total_voluntary_ctxt_switches > 100 {
        insights.push(
            "[CPU]  I/O-bound workload (low CPU with frequent voluntary context switches)".into(),
        );
    }

    // Kernel-heavy detection
    let sys_secs = summary.system_time.as_secs_f64();
    if total_cpu_secs > 0.0 && sys_secs / total_cpu_secs > 0.5 {
        insights.push(format!(
            "[CPU]  Kernel-heavy workload ({:.0}% system time -- high syscall overhead)",
            (sys_secs / total_cpu_secs) * 100.0
        ));
    }

    // High involuntary context switches
    let invol_rate = summary.total_nonvoluntary_ctxt_switches as f64 / wall_secs;
    if invol_rate > 1000.0 {
        insights.push(format!(
            "[CPU]  High CPU contention ({:.0} involuntary context switches/sec)",
            invol_rate
        ));
    }
}

fn memory_insights(summary: &Summary, samples: &SampleColumns, insights: &mut Vec<String>) {
    if summary.peak_rss_bytes > 0 && samples.len() > 2 {
        let peak_idx = samples
            .rss_bytes
            .iter()
            .enumerate()
            .max_by_key(|&(_, &v)| v)
            .map(|(i, _)| i)
            .unwrap_or(0);

        let peak_pct = (peak_idx as f64 / samples.len() as f64) * 100.0;

        if summary.peak_rss_bytes > summary.final_rss_bytes * 2
            && summary.peak_rss_bytes > 1024 * 1024
        {
            insights.push(format!(
                "[MEM]  Peak RSS {} at {:.0}% through execution, then declined",
                format_bytes(summary.peak_rss_bytes),
                peak_pct
            ));
        }
    }

    if samples.len() > 10 {
        let increasing = samples
            .rss_bytes
            .windows(2)
            .filter(|w| w[1] >= w[0])
            .count();
        let ratio = increasing as f64 / (samples.len() - 1) as f64;
        if ratio > 0.9
            && summary.final_rss_bytes > summary.initial_rss_bytes * 2
            && summary.final_rss_bytes > 1024 * 1024
        {
            insights
                .push("[MEM]  Memory grew monotonically -- possible leak if long-running".into());
        }
    }
}

fn page_fault_insights(summary: &Summary, wall_secs: f64, insights: &mut Vec<String>) {
    let majflt_rate = summary.total_majflt as f64 / wall_secs;
    if majflt_rate > 100.0 {
        insights.push(format!(
            "[MEM]  Severe disk paging ({:.0} major page faults/sec)",
            majflt_rate
        ));
    } else if majflt_rate > 10.0 {
        insights.push(format!(
            "[MEM]  Memory pressure detected ({:.0} major page faults/sec)",
            majflt_rate
        ));
    }
}

fn io_insights(summary: &Summary, wall_secs: f64, insights: &mut Vec<String>) {
    // Page cache hit ratio
    if summary.total_rchar > 0 && summary.total_read_bytes > 0 {
        let cache_ratio = (summary.total_rchar.saturating_sub(summary.total_read_bytes)) as f64
            / summary.total_rchar as f64;
        if cache_ratio > 0.5 && summary.total_rchar > 1024 * 1024 {
            insights.push(format!(
                "[I/O]  {:.0}% of reads served from page cache",
                cache_ratio * 100.0
            ));
        }
    }

    // Small I/O operations
    if summary.total_syscr > 100 {
        let bytes_per_read = summary.total_rchar / summary.total_syscr;
        if bytes_per_read < 512 {
            insights.push(format!(
                "[I/O]  Small read operations (avg {} bytes/syscall) -- buffering may help",
                bytes_per_read
            ));
        }
    }

    if summary.total_syscw > 100 {
        let bytes_per_write = summary.total_wchar / summary.total_syscw;
        if bytes_per_write < 512 {
            insights.push(format!(
                "[I/O]  Small write operations (avg {} bytes/syscall) -- buffering may help",
                bytes_per_write
            ));
        }
    }

    // Heavy I/O
    let read_rate = summary.total_read_bytes as f64 / wall_secs;
    let write_rate = summary.total_write_bytes as f64 / wall_secs;
    if read_rate > 100.0 * 1024.0 * 1024.0 {
        insights.push(format!(
            "[I/O]  Read-heavy: {}/sec throughput",
            format_bytes(read_rate as u64)
        ));
    }
    if write_rate > 100.0 * 1024.0 * 1024.0 {
        insights.push(format!(
            "[I/O]  Write-heavy: {}/sec throughput",
            format_bytes(write_rate as u64)
        ));
    }
}

fn thread_insights(summary: &Summary, insights: &mut Vec<String>) {
    if summary.peak_threads > summary.initial_threads * 2 && summary.peak_threads > 4 {
        insights.push(format!(
            "[THR]  Dynamic thread creation detected (peak {} threads from initial {})",
            summary.peak_threads, summary.initial_threads
        ));
    }
}

fn fd_insights(_summary: &Summary, samples: &SampleColumns, insights: &mut Vec<String>) {
    if samples.len() > 10 {
        let increasing = samples.fd_count.windows(2).filter(|w| w[1] >= w[0]).count();
        let ratio = increasing as f64 / (samples.len() - 1) as f64;
        let first_fd = samples.fd_count.first().copied().unwrap_or(0);
        let last_fd = samples.fd_count.last().copied().unwrap_or(0);
        if ratio > 0.9 && last_fd > first_fd + 10 {
            insights.push(format!(
                "[FD]   File descriptor count grew monotonically ({} -> {}) -- possible leak",
                first_fd, last_fd
            ));
        }
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1}GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}KB", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::SampleColumns;

    fn make_summary(overrides: impl FnOnce(&mut Summary)) -> Summary {
        let mut s = Summary {
            user_time: Duration::from_secs(5),
            system_time: Duration::from_secs(1),
            cpu_percent: 60.0,
            peak_rss_bytes: 100 * 1024 * 1024,
            initial_rss_bytes: 10 * 1024 * 1024,
            final_rss_bytes: 50 * 1024 * 1024,
            peak_vsize_bytes: 200 * 1024 * 1024,
            total_minflt: 1000,
            total_majflt: 0,
            total_read_bytes: 1024 * 1024,
            total_write_bytes: 512 * 1024,
            total_rchar: 10 * 1024 * 1024,
            total_wchar: 1024 * 1024,
            total_syscr: 100,
            total_syscw: 50,
            total_net_rx_bytes: 0,
            total_net_tx_bytes: 0,
            initial_threads: 1,
            final_threads: 1,
            peak_threads: 1,
            total_voluntary_ctxt_switches: 50,
            total_nonvoluntary_ctxt_switches: 10,
            peak_fd_count: 5,
            final_fd_count: 5,
            child_processes_spawned: 0,
        };
        overrides(&mut s);
        s
    }

    #[test]
    fn detect_cpu_bound() {
        let summary = make_summary(|s| {
            s.user_time = Duration::from_secs(9);
            s.system_time = Duration::from_secs(1);
            s.cpu_percent = 95.0;
        });
        let insights =
            generate_insights(&summary, &SampleColumns::default(), Duration::from_secs(10));
        assert!(insights.iter().any(|i| i.contains("CPU-bound")));
    }

    #[test]
    fn detect_io_bound() {
        let summary = make_summary(|s| {
            s.user_time = Duration::from_secs(2);
            s.system_time = Duration::from_secs(1);
            s.total_voluntary_ctxt_switches = 5000;
        });
        let insights =
            generate_insights(&summary, &SampleColumns::default(), Duration::from_secs(10));
        assert!(insights.iter().any(|i| i.contains("I/O-bound")));
    }

    #[test]
    fn detect_kernel_heavy() {
        let summary = make_summary(|s| {
            s.user_time = Duration::from_secs(1);
            s.system_time = Duration::from_secs(4);
        });
        let insights =
            generate_insights(&summary, &SampleColumns::default(), Duration::from_secs(10));
        assert!(insights.iter().any(|i| i.contains("Kernel-heavy")));
    }

    #[test]
    fn detect_major_page_faults() {
        let summary = make_summary(|s| {
            s.total_majflt = 500;
        });
        let insights =
            generate_insights(&summary, &SampleColumns::default(), Duration::from_secs(10));
        assert!(
            insights
                .iter()
                .any(|i| i.contains("page fault") || i.contains("paging"))
        );
    }

    #[test]
    fn detect_high_contention() {
        let summary = make_summary(|s| {
            s.total_nonvoluntary_ctxt_switches = 20000;
        });
        let insights =
            generate_insights(&summary, &SampleColumns::default(), Duration::from_secs(10));
        assert!(insights.iter().any(|i| i.contains("contention")));
    }

    #[test]
    fn format_bytes_units() {
        assert_eq!(format_bytes(500), "500B");
        assert_eq!(format_bytes(1536), "1.5KB");
        assert_eq!(format_bytes(1572864), "1.5MB");
        assert_eq!(format_bytes(1610612736), "1.5GB");
    }
}
