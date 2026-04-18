mod insights;
mod metrics;
mod process_tree;
mod py_trace;
mod sampler;
mod terminal;

use clap::Parser;
use metrics::{ProcessResult, Summary};
use nix::sys::signal::{self, Signal};
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::Pid;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

#[derive(Parser)]
#[command(name = "viy", version, about = "Lightweight process monitoring tool")]
struct Cli {
    /// Output report as JSON
    #[arg(long)]
    json: bool,

    /// Suppress insights section
    #[arg(long)]
    quiet: bool,

    /// Suppress entire report (useful with --json)
    #[arg(long)]
    silent: bool,

    /// Sampling interval in milliseconds
    #[arg(long, default_value = "10")]
    interval: u64,

    /// Disable colored output
    #[arg(long)]
    no_color: bool,

    /// Only trace Python frames whose filename matches this glob (e.g. "*train.py")
    #[arg(long)]
    py_filter: Option<String>,

    /// Max number of functions shown in Python Timeline (default: 10)
    #[arg(long, default_value = "10")]
    py_top: usize,

    /// Command to run
    #[arg(required = true, trailing_var_arg = true)]
    command: Vec<String>,
}

fn main() {
    let cli = Cli::parse();

    let use_color = !cli.no_color && std::io::IsTerminal::is_terminal(&std::io::stderr());

    let (program, args) = cli.command.split_first().expect("command is required");

    let mut cmd = Command::new(program);
    cmd.args(args);
    let python_tracing =
        py_trace::setup_python_tracing(&mut cmd, program, cli.py_filter.as_deref());

    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            eprintln!("viy: failed to execute '{}': {}", program, e);
            std::process::exit(127);
        }
    };

    let child_pid = child.id() as i32;
    let start = Instant::now();

    // Capture baseline network counters (namespace-wide)
    let net_baseline = sampler::read_net_dev(child_pid);

    // Set up signal forwarding
    let stop_flag = Arc::new(AtomicBool::new(false));
    setup_signal_forwarding(child_pid);

    // Start sampler thread
    let sampler_stop = stop_flag.clone();
    let interval = std::time::Duration::from_millis(cli.interval);
    let baseline = net_baseline;
    let sampler_handle = std::thread::spawn(move || {
        sampler::run_sampler(
            sampler::SamplerConfig {
                pid: child_pid,
                base_interval: interval,
                stop_flag: sampler_stop,
            },
            baseline,
        )
    });

    // Wait for child to exit
    let (exit_code, signal_num) = wait_for_child(child_pid);
    let wall_time = start.elapsed();

    // Capture final network counters
    // Child is dead, but we can read our own /proc/self/net/dev (same namespace)
    let net_final = sampler::read_net_dev(std::process::id() as i32);

    // Stop sampler
    stop_flag.store(true, Ordering::Relaxed);
    let (samples, child_count, io_totals) = sampler_handle.join().unwrap_or_else(|_| {
        (
            metrics::SampleColumns::default(),
            0,
            sampler::PidIoTotals {
                read_bytes: 0,
                write_bytes: 0,
                rchar: 0,
                wchar: 0,
                syscr: 0,
                syscw: 0,
            },
        )
    });

    let net_rx = net_final.0.saturating_sub(net_baseline.0);
    let net_tx = net_final.1.saturating_sub(net_baseline.1);
    let samples = samples.compress(1000);
    let py_epoch_offset_ms = 0.0;
    let summary =
        Summary::from_samples(&samples, wall_time, child_count, &io_totals, net_rx, net_tx);
    let insights_list = insights::generate_insights(&summary, &samples, wall_time);

    let py_insights = python_tracing
        .as_ref()
        .map(|setup| {
            py_trace::correlate(
                &samples,
                &setup.trace_path,
                wall_time,
                cli.py_filter.as_deref(),
                py_epoch_offset_ms,
            )
        })
        .unwrap_or_default();

    let command_str = cli.command.join(" ");
    let result = ProcessResult {
        command: command_str,
        exit_code,
        signal: signal_num,
        wall_time,
        samples,
        summary,
        insights: insights_list,
        py_insights,
        py_trace_path: python_tracing.as_ref().map(|s| s.trace_path.clone()),
        py_filter: cli.py_filter.clone(),
        py_top: cli.py_top,
        py_epoch_offset_ms,
    };

    // Output
    if cli.json {
        let json = serde_json::to_string_pretty(&result).unwrap_or_else(|e| {
            eprintln!("viy: JSON serialization error: {}", e);
            "{}".into()
        });
        eprintln!("{}", json);
    }

    if !cli.silent {
        terminal::report::print_report(&result, use_color, cli.quiet);
    }

    if let Some(ref setup) = python_tracing {
        py_trace::cleanup(setup);
    }

    std::process::exit(exit_code.unwrap_or(1));
}

fn wait_for_child(pid: i32) -> (Option<i32>, Option<i32>) {
    let nix_pid = Pid::from_raw(pid);
    loop {
        match waitpid(nix_pid, None) {
            Ok(WaitStatus::Exited(_, code)) => return (Some(code), None),
            Ok(WaitStatus::Signaled(_, sig, _)) => return (None, Some(sig as i32)),
            Ok(_) => continue,
            Err(nix::errno::Errno::ECHILD) => return (None, None),
            Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => return (None, None),
        }
    }
}

fn setup_signal_forwarding(child_pid: i32) {
    unsafe {
        signal::sigaction(
            Signal::SIGINT,
            &signal::SigAction::new(
                signal::SigHandler::Handler(forward_signal),
                signal::SaFlags::empty(),
                signal::SigSet::empty(),
            ),
        )
        .ok();

        signal::sigaction(
            Signal::SIGTERM,
            &signal::SigAction::new(
                signal::SigHandler::Handler(forward_signal),
                signal::SaFlags::empty(),
                signal::SigSet::empty(),
            ),
        )
        .ok();
    }

    CHILD_PID.store(child_pid, Ordering::Relaxed);
}

static CHILD_PID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

extern "C" fn forward_signal(sig: i32) {
    let pid = CHILD_PID.load(Ordering::Relaxed);
    if pid > 0 {
        let _ = signal::kill(
            Pid::from_raw(pid),
            Signal::try_from(sig).unwrap_or(Signal::SIGTERM),
        );
    }
}
