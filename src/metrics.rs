use serde::Serialize;
use std::time::Duration;

/// A single point-in-time snapshot (used transiently during collection).
pub struct Sample {
    pub timestamp: Duration,
    pub rss_bytes: u64,
    pub vsize_bytes: u64,
    pub utime_ticks: u64,
    pub stime_ticks: u64,
    pub minflt: u64,
    pub majflt: u64,
    pub threads: u32,
    pub voluntary_ctxt_switches: u64,
    pub nonvoluntary_ctxt_switches: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub rchar: u64,
    pub wchar: u64,
    pub syscr: u64,
    pub syscw: u64,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    pub fd_count: u32,
}

/// Columnar storage for all samples — one Vec per metric.
/// Serializes as a JSON array of objects to preserve the documented API.
#[derive(Debug, Clone, Default)]
pub struct SampleColumns {
    pub timestamp: Vec<Duration>,
    pub rss_bytes: Vec<u64>,
    pub vsize_bytes: Vec<u64>,
    pub utime_ticks: Vec<u64>,
    pub stime_ticks: Vec<u64>,
    pub minflt: Vec<u64>,
    pub majflt: Vec<u64>,
    pub threads: Vec<u32>,
    pub voluntary_ctxt_switches: Vec<u64>,
    pub nonvoluntary_ctxt_switches: Vec<u64>,
    pub read_bytes: Vec<u64>,
    pub write_bytes: Vec<u64>,
    pub rchar: Vec<u64>,
    pub wchar: Vec<u64>,
    pub syscr: Vec<u64>,
    pub syscw: Vec<u64>,
    pub net_rx_bytes: Vec<u64>,
    pub net_tx_bytes: Vec<u64>,
    pub fd_count: Vec<u32>,
}

impl SampleColumns {
    pub fn with_capacity(n: usize) -> Self {
        Self {
            timestamp: Vec::with_capacity(n),
            rss_bytes: Vec::with_capacity(n),
            vsize_bytes: Vec::with_capacity(n),
            utime_ticks: Vec::with_capacity(n),
            stime_ticks: Vec::with_capacity(n),
            minflt: Vec::with_capacity(n),
            majflt: Vec::with_capacity(n),
            threads: Vec::with_capacity(n),
            voluntary_ctxt_switches: Vec::with_capacity(n),
            nonvoluntary_ctxt_switches: Vec::with_capacity(n),
            read_bytes: Vec::with_capacity(n),
            write_bytes: Vec::with_capacity(n),
            rchar: Vec::with_capacity(n),
            wchar: Vec::with_capacity(n),
            syscr: Vec::with_capacity(n),
            syscw: Vec::with_capacity(n),
            net_rx_bytes: Vec::with_capacity(n),
            net_tx_bytes: Vec::with_capacity(n),
            fd_count: Vec::with_capacity(n),
        }
    }

    pub fn push(&mut self, s: Sample) {
        self.timestamp.push(s.timestamp);
        self.rss_bytes.push(s.rss_bytes);
        self.vsize_bytes.push(s.vsize_bytes);
        self.utime_ticks.push(s.utime_ticks);
        self.stime_ticks.push(s.stime_ticks);
        self.minflt.push(s.minflt);
        self.majflt.push(s.majflt);
        self.threads.push(s.threads);
        self.voluntary_ctxt_switches.push(s.voluntary_ctxt_switches);
        self.nonvoluntary_ctxt_switches
            .push(s.nonvoluntary_ctxt_switches);
        self.read_bytes.push(s.read_bytes);
        self.write_bytes.push(s.write_bytes);
        self.rchar.push(s.rchar);
        self.wchar.push(s.wchar);
        self.syscr.push(s.syscr);
        self.syscw.push(s.syscw);
        self.net_rx_bytes.push(s.net_rx_bytes);
        self.net_tx_bytes.push(s.net_tx_bytes);
        self.fd_count.push(s.fd_count);
    }

    pub fn len(&self) -> usize {
        self.timestamp.len()
    }

    pub fn is_empty(&self) -> bool {
        self.timestamp.is_empty()
    }

    /// Merge neighboring samples down to at most `target` buckets.
    /// Timestamps become the midpoint of each bucket.
    /// Cumulative counters (ticks, faults, io, net, ctx) take the max (last value in bucket).
    /// Instantaneous metrics (rss, vsize, threads, fd) take the max.
    pub fn compress(&self, target: usize) -> Self {
        let n = self.len();
        if n <= target || target == 0 {
            return self.clone();
        }
        let mut out = Self::with_capacity(target);
        for b in 0..target {
            let lo = b * n / target;
            let hi = ((b + 1) * n / target).min(n);
            if lo >= hi {
                continue;
            }
            // midpoint timestamp
            let ts = (self.timestamp[lo] + self.timestamp[hi - 1]) / 2;
            // cumulative: take last in bucket (monotonically increasing)
            let last = hi - 1;
            // instantaneous: take max in bucket
            macro_rules! col_max_u64 {
                ($col:ident) => {
                    self.$col[lo..hi].iter().copied().max().unwrap_or(0)
                };
            }
            macro_rules! col_max_u32 {
                ($col:ident) => {
                    self.$col[lo..hi].iter().copied().max().unwrap_or(0)
                };
            }
            out.timestamp.push(ts);
            out.rss_bytes.push(col_max_u64!(rss_bytes));
            out.vsize_bytes.push(col_max_u64!(vsize_bytes));
            out.utime_ticks.push(self.utime_ticks[last]);
            out.stime_ticks.push(self.stime_ticks[last]);
            out.minflt.push(self.minflt[last]);
            out.majflt.push(self.majflt[last]);
            out.threads.push(col_max_u32!(threads));
            out.voluntary_ctxt_switches
                .push(self.voluntary_ctxt_switches[last]);
            out.nonvoluntary_ctxt_switches
                .push(self.nonvoluntary_ctxt_switches[last]);
            out.read_bytes.push(self.read_bytes[last]);
            out.write_bytes.push(self.write_bytes[last]);
            out.rchar.push(self.rchar[last]);
            out.wchar.push(self.wchar[last]);
            out.syscr.push(self.syscr[last]);
            out.syscw.push(self.syscw[last]);
            out.net_rx_bytes.push(self.net_rx_bytes[last]);
            out.net_tx_bytes.push(self.net_tx_bytes[last]);
            out.fd_count.push(col_max_u32!(fd_count));
        }
        out
    }
}

impl Serialize for SampleColumns {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(self.len()))?;
        for i in 0..self.len() {
            seq.serialize_element(&serde_json::json!({
                "timestamp": { "secs": self.timestamp[i].as_secs(), "nanos": self.timestamp[i].subsec_nanos() },
                "rss_bytes": self.rss_bytes[i],
                "vsize_bytes": self.vsize_bytes[i],
                "utime_ticks": self.utime_ticks[i],
                "stime_ticks": self.stime_ticks[i],
                "minflt": self.minflt[i],
                "majflt": self.majflt[i],
                "threads": self.threads[i],
                "voluntary_ctxt_switches": self.voluntary_ctxt_switches[i],
                "nonvoluntary_ctxt_switches": self.nonvoluntary_ctxt_switches[i],
                "read_bytes": self.read_bytes[i],
                "write_bytes": self.write_bytes[i],
                "rchar": self.rchar[i],
                "wchar": self.wchar[i],
                "syscr": self.syscr[i],
                "syscw": self.syscw[i],
                "net_rx_bytes": self.net_rx_bytes[i],
                "net_tx_bytes": self.net_tx_bytes[i],
                "fd_count": self.fd_count[i],
            }))?;
        }
        seq.end()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessResult {
    pub command: String,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub wall_time: Duration,
    pub samples: SampleColumns,
    pub summary: Summary,
    pub insights: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub py_insights: Vec<String>,
    #[serde(skip)]
    pub py_trace_path: Option<std::path::PathBuf>,
    #[serde(skip)]
    pub py_filter: Option<String>,
    pub py_top: usize,
    /// Sampler-clock ms of the first sample — used to align py trace timestamps.
    #[serde(skip)]
    pub py_epoch_offset_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub user_time: Duration,
    pub system_time: Duration,
    pub cpu_percent: f64,
    pub peak_rss_bytes: u64,
    pub initial_rss_bytes: u64,
    pub final_rss_bytes: u64,
    pub peak_vsize_bytes: u64,
    pub total_minflt: u64,
    pub total_majflt: u64,
    pub total_read_bytes: u64,
    pub total_write_bytes: u64,
    pub total_rchar: u64,
    pub total_wchar: u64,
    pub total_syscr: u64,
    pub total_syscw: u64,
    pub total_net_rx_bytes: u64,
    pub total_net_tx_bytes: u64,
    pub initial_threads: u32,
    pub final_threads: u32,
    pub peak_threads: u32,
    pub total_voluntary_ctxt_switches: u64,
    pub total_nonvoluntary_ctxt_switches: u64,
    pub peak_fd_count: u32,
    pub final_fd_count: u32,
    pub child_processes_spawned: u32,
}

impl Summary {
    pub fn from_samples(
        samples: &SampleColumns,
        wall_time: Duration,
        child_count: u32,
        io_totals: &crate::sampler::PidIoTotals,
        net_rx: u64,
        net_tx: u64,
    ) -> Self {
        if samples.is_empty() {
            return Self::empty();
        }

        let n = samples.len();
        let ticks_per_sec = procfs::ticks_per_second() as f64;
        let total_utime = samples.utime_ticks[n - 1].saturating_sub(samples.utime_ticks[0]);
        let total_stime = samples.stime_ticks[n - 1].saturating_sub(samples.stime_ticks[0]);
        let user_secs = total_utime as f64 / ticks_per_sec;
        let sys_secs = total_stime as f64 / ticks_per_sec;
        let wall_secs = wall_time.as_secs_f64();
        let cpu_percent = if wall_secs > 0.0 {
            ((user_secs + sys_secs) / wall_secs) * 100.0
        } else {
            0.0
        };

        let total_read_bytes = if io_totals.read_bytes > 0 {
            io_totals.read_bytes
        } else {
            samples.read_bytes[n - 1].saturating_sub(samples.read_bytes[0])
        };
        let total_write_bytes = if io_totals.write_bytes > 0 {
            io_totals.write_bytes
        } else {
            samples.write_bytes[n - 1].saturating_sub(samples.write_bytes[0])
        };

        Summary {
            user_time: Duration::from_secs_f64(user_secs),
            system_time: Duration::from_secs_f64(sys_secs),
            cpu_percent,
            peak_rss_bytes: samples.rss_bytes.iter().copied().max().unwrap_or(0),
            initial_rss_bytes: samples.rss_bytes[0],
            final_rss_bytes: samples.rss_bytes[n - 1],
            peak_vsize_bytes: samples.vsize_bytes.iter().copied().max().unwrap_or(0),
            total_minflt: samples.minflt[n - 1].saturating_sub(samples.minflt[0]),
            total_majflt: samples.majflt[n - 1].saturating_sub(samples.majflt[0]),
            total_read_bytes,
            total_write_bytes,
            total_rchar: io_totals.rchar,
            total_wchar: io_totals.wchar,
            total_syscr: io_totals.syscr,
            total_syscw: io_totals.syscw,
            total_net_rx_bytes: net_rx,
            total_net_tx_bytes: net_tx,
            initial_threads: samples.threads[0],
            final_threads: samples.threads[n - 1],
            peak_threads: samples.threads.iter().copied().max().unwrap_or(0),
            total_voluntary_ctxt_switches: samples.voluntary_ctxt_switches[n - 1]
                .saturating_sub(samples.voluntary_ctxt_switches[0]),
            total_nonvoluntary_ctxt_switches: samples.nonvoluntary_ctxt_switches[n - 1]
                .saturating_sub(samples.nonvoluntary_ctxt_switches[0]),
            peak_fd_count: samples.fd_count.iter().copied().max().unwrap_or(0),
            final_fd_count: samples.fd_count[n - 1],
            child_processes_spawned: child_count,
        }
    }

    fn empty() -> Self {
        Summary {
            user_time: Duration::ZERO,
            system_time: Duration::ZERO,
            cpu_percent: 0.0,
            peak_rss_bytes: 0,
            initial_rss_bytes: 0,
            final_rss_bytes: 0,
            peak_vsize_bytes: 0,
            total_minflt: 0,
            total_majflt: 0,
            total_read_bytes: 0,
            total_write_bytes: 0,
            total_rchar: 0,
            total_wchar: 0,
            total_syscr: 0,
            total_syscw: 0,
            total_net_rx_bytes: 0,
            total_net_tx_bytes: 0,
            initial_threads: 0,
            final_threads: 0,
            peak_threads: 0,
            total_voluntary_ctxt_switches: 0,
            total_nonvoluntary_ctxt_switches: 0,
            peak_fd_count: 0,
            final_fd_count: 0,
            child_processes_spawned: 0,
        }
    }
}
