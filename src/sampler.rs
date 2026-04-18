use crate::metrics::{Sample, SampleColumns};
use crate::process_tree::TreeTracker;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Sampling configuration
pub struct SamplerConfig {
    pub pid: i32,
    pub base_interval: Duration,
    pub stop_flag: Arc<AtomicBool>,
}

/// Per-process I/O high watermarks — tracks the max cumulative values seen
/// for each PID so we don't lose data when children exit.
#[derive(Default)]
struct IoWatermarks {
    per_pid: HashMap<i32, PidIo>,
}

#[derive(Default, Clone)]
struct PidIo {
    read_bytes: u64,
    write_bytes: u64,
    rchar: u64,
    wchar: u64,
    syscr: u64,
    syscw: u64,
}

impl IoWatermarks {
    fn update(&mut self, pid: i32, io: &PidIo) {
        let entry = self.per_pid.entry(pid).or_default();
        entry.read_bytes = entry.read_bytes.max(io.read_bytes);
        entry.write_bytes = entry.write_bytes.max(io.write_bytes);
        entry.rchar = entry.rchar.max(io.rchar);
        entry.wchar = entry.wchar.max(io.wchar);
        entry.syscr = entry.syscr.max(io.syscr);
        entry.syscw = entry.syscw.max(io.syscw);
    }

    /// Sum of high watermarks across all PIDs ever seen.
    fn totals(&self) -> PidIo {
        let mut t = PidIo::default();
        for io in self.per_pid.values() {
            t.read_bytes += io.read_bytes;
            t.write_bytes += io.write_bytes;
            t.rchar += io.rchar;
            t.wchar += io.wchar;
            t.syscr += io.syscr;
            t.syscw += io.syscw;
        }
        t
    }
}

/// Run the sampling loop. Returns collected samples and child process count.
pub fn run_sampler(
    config: SamplerConfig,
    net_baseline: (u64, u64),
) -> (SampleColumns, u32, PidIoTotals) {
    let start = Instant::now();
    let mut samples = SampleColumns::with_capacity(1024);
    let mut tree_tracker = TreeTracker::new(config.pid);
    let mut io_watermarks = IoWatermarks::default();

    // Low-frequency counters
    let mut last_medium_sample = Instant::now() - Duration::from_secs(1); // trigger immediately
    let mut last_low_sample = Instant::now() - Duration::from_secs(1);

    let mut cached_vol_cs: u64 = 0;
    let mut cached_nonvol_cs: u64 = 0;
    let mut cached_fd_count: u32 = 0;

    while !config.stop_flag.load(Ordering::Relaxed) {
        let elapsed = start.elapsed();

        // Adaptive interval: 10ms for first second, then base_interval
        let interval = if elapsed < Duration::from_secs(1) {
            Duration::from_millis(10)
        } else {
            config.base_interval
        };

        // Discover process tree every sample (children can be short-lived)
        let pids = tree_tracker.update();

        // Medium frequency reads: context switches
        if last_medium_sample.elapsed() >= Duration::from_millis(250) {
            if let Some((vol, nonvol)) = read_context_switches(&pids) {
                cached_vol_cs = vol;
                cached_nonvol_cs = nonvol;
            }
            last_medium_sample = Instant::now();
        }

        // Low frequency: FD count
        if last_low_sample.elapsed() >= Duration::from_secs(1) {
            cached_fd_count = count_fds(&pids);
            last_low_sample = Instant::now();
        }

        // High frequency: stat + io + net
        if let Some(sample) = collect_sample(SampleCtx {
            pids: &pids,
            timestamp: elapsed,
            vol_cs: cached_vol_cs,
            nonvol_cs: cached_nonvol_cs,
            fd_count: cached_fd_count,
            io_watermarks: &mut io_watermarks,
            root_pid: config.pid,
            net_baseline: &net_baseline,
        }) {
            samples.push(sample);
        }

        std::thread::sleep(interval);
    }

    let io_totals = io_watermarks.totals();
    let totals = PidIoTotals {
        read_bytes: io_totals.read_bytes,
        write_bytes: io_totals.write_bytes,
        rchar: io_totals.rchar,
        wchar: io_totals.wchar,
        syscr: io_totals.syscr,
        syscw: io_totals.syscw,
    };

    (samples, tree_tracker.total_children_seen(), totals)
}

/// Final I/O totals from watermark tracking (survives child exits).
pub struct PidIoTotals {
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub rchar: u64,
    pub wchar: u64,
    pub syscr: u64,
    pub syscw: u64,
}

struct SampleCtx<'a> {
    pids: &'a HashSet<i32>,
    timestamp: Duration,
    vol_cs: u64,
    nonvol_cs: u64,
    fd_count: u32,
    io_watermarks: &'a mut IoWatermarks,
    root_pid: i32,
    net_baseline: &'a (u64, u64),
}

fn collect_sample(ctx: SampleCtx) -> Option<Sample> {
    let SampleCtx {
        pids,
        timestamp,
        vol_cs,
        nonvol_cs,
        fd_count,
        io_watermarks,
        root_pid,
        net_baseline,
    } = ctx;
    let mut rss_bytes: u64 = 0;
    let mut vsize_bytes: u64 = 0;
    let mut utime_ticks: u64 = 0;
    let mut stime_ticks: u64 = 0;
    let mut minflt: u64 = 0;
    let mut majflt: u64 = 0;
    let mut threads: u32 = 0;
    let mut read_bytes: u64 = 0;
    let mut write_bytes: u64 = 0;
    let mut rchar: u64 = 0;
    let mut wchar: u64 = 0;
    let mut syscr: u64 = 0;
    let mut syscw: u64 = 0;

    let mut any_success = false;
    let page_size = procfs::page_size();

    for &pid in pids {
        let Ok(proc) = procfs::process::Process::new(pid) else {
            continue;
        };

        if let Ok(stat) = proc.stat() {
            rss_bytes += stat.rss * page_size;
            vsize_bytes += stat.vsize;
            utime_ticks += stat.utime;
            stime_ticks += stat.stime;
            minflt += stat.minflt;
            majflt += stat.majflt;
            threads += stat.num_threads as u32;
            any_success = true;
        }

        if let Ok(io) = proc.io() {
            let pid_io = PidIo {
                read_bytes: io.read_bytes,
                write_bytes: io.write_bytes,
                rchar: io.rchar,
                wchar: io.wchar,
                syscr: io.syscr,
                syscw: io.syscw,
            };
            io_watermarks.update(pid, &pid_io);

            read_bytes += io.read_bytes;
            write_bytes += io.write_bytes;
            rchar += io.rchar;
            wchar += io.wchar;
            syscr += io.syscr;
            syscw += io.syscw;
        }
    }

    if !any_success {
        return None;
    }

    Some(Sample {
        timestamp,
        rss_bytes,
        vsize_bytes,
        utime_ticks,
        stime_ticks,
        minflt,
        majflt,
        threads,
        voluntary_ctxt_switches: vol_cs,
        nonvoluntary_ctxt_switches: nonvol_cs,
        read_bytes,
        write_bytes,
        rchar,
        wchar,
        syscr,
        syscw,
        // Network: namespace-wide delta from baseline
        net_rx_bytes: {
            let (rx, _) = read_net_dev(root_pid);
            rx.saturating_sub(net_baseline.0)
        },
        net_tx_bytes: {
            let (_, tx) = read_net_dev(root_pid);
            tx.saturating_sub(net_baseline.1)
        },
        fd_count,
    })
}

fn read_context_switches(pids: &HashSet<i32>) -> Option<(u64, u64)> {
    let mut vol: u64 = 0;
    let mut nonvol: u64 = 0;
    let mut any = false;

    for &pid in pids {
        let Ok(proc) = procfs::process::Process::new(pid) else {
            continue;
        };
        if let Ok(status) = proc.status() {
            vol += status.voluntary_ctxt_switches.unwrap_or(0);
            nonvol += status.nonvoluntary_ctxt_switches.unwrap_or(0);
            any = true;
        }
    }

    any.then_some((vol, nonvol))
}

fn count_fds(pids: &HashSet<i32>) -> u32 {
    let mut total: u32 = 0;
    for &pid in pids {
        let Ok(proc) = procfs::process::Process::new(pid) else {
            continue;
        };
        if let Ok(fds) = proc.fd_count() {
            total += fds as u32;
        }
    }
    total
}

/// Read aggregate network stats from /proc/[pid]/net/dev.
/// NOTE: This returns namespace-wide counters, not per-process.
/// Only meaningful as a delta between two calls.
pub fn read_net_dev(pid: i32) -> (u64, u64) {
    let path = format!("/proc/{}/net/dev", pid);
    let Ok(content) = fs::read_to_string(&path) else {
        return (0, 0);
    };

    let mut rx_total: u64 = 0;
    let mut tx_total: u64 = 0;

    for line in content.lines().skip(2) {
        let line = line.trim();
        if line.starts_with("lo:") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 10
            && let (Ok(rx), Ok(tx)) = (parts[1].parse::<u64>(), parts[9].parse::<u64>())
        {
            rx_total += rx;
            tx_total += tx;
        }
    }

    (rx_total, tx_total)
}
