# viy

<img align="left" src="./docs/viy.jpg" alt="Viy - a creature inspired by Ukrainian folklore" width="280"/>

Lightweight **end-to-end process monitoring** for Linux. Unlike top, time, or single-metric profilers, viy captures CPU, memory, I/O, network, threads, and file descriptors together with time-series visualization and performance insights.

It correlates resource spikes with Python function calls, helping too see root causes and understand program behavior.

Named after **Viy** (Вій), a creature inspired by Ukrainian folklore known for its powerful gaze that sees through everything.

<br clear="left"/>

## Install

```bash
# Download binary
curl -L https://github.com/enkidulan/viy/releases/latest/download/viy -o ~/.local/bin/viy
chmod +x ~/.local/bin/viy
```

## Usage

```bash
$ viy python tests/test_workload.py
```

```
╭─ viy ────────────────────────────────────────────────────────────────────────╮
│ Command:     python tests/test_workload.py                                   │
│ Exit code:   0                                                               │
│ Wall time:   4.72s                                                           │
╰──────────────────────────────────────────────────────────────────────────────╯
╭─ Summary ────────────────────────────────────────────────────────────────────╮
│ CPU:         71.6% (user 3.12s, sys 260.0ms)    Memory:      9.0MB -> 11.8MB (peak 111.6MB)│
│ I/O:         477.1MB read, 477.3MB written    Network:     148B in, 0B out (host)│
│ Ctx switch:  52 vol, 118 invol    Page faults: 26.7K minor, 0 major          │
│ Threads:     1 -> 1 (peak 6)    File descs:  23 (peak 24)                    │
│ Processes:   5 spawned                                                       │
╰──────────────────────────────────────────────────────────────────────────────╯
╭─ CPU Utilization (%)  avg 71.6% ─────────────────────────────────────────────╮
│      116% ▄ █ ▆ ▆ ▇ ▇▅▆ ▆ ▆ ▇ ▇ █ ▇ ▇ █ ▇ ▇▇█▅▇ ▇ ▇▆█                   ▆ ▅  │
│           █ █ █ █ █ ███ █ █ █ █ █ █ █ █ █ █████ █ ███                   █ █  │
│       58% █▃█▄█▃█▃█▄███▃█▃█▂█▄█▄█▄█▃█▄█▄█▂█████▃█▄███▃  ▂     ▁   ▂   ▂▂█▄█  │
│           ████████████████████████████████████████████  █     █   █   █████  │
│        0% ████████████████████████████████████████████  █     █   █   █████  │
│           0             1.2s             2.4s            3.5s            4.7 │
╰──────────────────────────────────────────────────────────────────────────────╯
╭─ Memory RSS  peak 111.6MB ───────────────────────────────────────────────────╮
│   111.6MB                                                        ▁▆▇███      │
│                                                               ▄▅███████      │
│    55.8MB                                                 ▁▂▆▇█████████      │
│                                                         ▃▄█████████████      │
│        0B ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▆▇███████████████▄▄▄▄▄ │
│           0             1.2s             2.4s            3.5s            4.7 │
╰──────────────────────────────────────────────────────────────────────────────╯
╭─ I/O Rate  477.1MB read, 477.3MB written ────────────────────────────────────╮
│   6.6GB/s                                                                 █  │
│                                                                           █  │
│   3.3GB/s                                                                 █  │
│                                                                        ▄▆▅█  │
│      0B/s                                                             ▅████  │
│           0             1.2s             2.4s            3.5s            4.7 │
╰──────────────────────────────────────────────────────────────────────────────╯

╭─ Python Timeline  →  4.72s ──────────────────────────────────────────────────╮
│ cpu_burn (__main__)  [CPU]  163%                                             │
│            ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░                         │
│ memory_grow_and_shrink (__main__)  [MEM]  111.6MB                            │
│                                                       ░░░░▒▒▒▒▓▓▓▓████▒      │
│ io_write_read (__main__)  [I/O]  6.6GB/s                                     │
│                                                                       ░░░▒▓  │
│ spawn_children (__main__)  [CPU]  682%                                       │
│                                                                            ▒ │
│           0              1.2s            2.4s             3.5s            4.7│
╰──────────────────────────────────────────────────────────────────────────────╯
╭─ Insights ───────────────────────────────────────────────────────────────────╮
│ [THR]  Dynamic thread creation detected (peak 6 threads from initial 1)      │
│ [MEM]  Peak RSS 111.6MB at 91% through execution, then declined              │
│ [PY]   CPU peak (682%) → spawn_children (__main__)                           │
│ [PY]   Memory peak (111.6MB) → memory_grow_and_shrink (__main__)             │
│ [PY]   I/O peak (6.6GB/s) → io_write_read (__main__)                         │
╰──────────────────────────────────────────────────────────────────────────────╯
```

## CLI

```bash
viy [OPTIONS] <command> [args...]
```

### Options

- `--json` — Output JSON to stderr
- `--quiet` — Hide insights
- `--silent` — Hide entire report (use with `--json`)
- `--interval <ms>` — Sampling interval (default: 10)
- `--no-color` — Disable colors
- `--py-filter <glob>` — Trace Python frames matching pattern
- `--py-top <n>` — Show top N Python functions (default: 10)

### Examples

```bash
# Monitor a build
viy make -j8

# JSON output for CI
viy --json --silent ./benchmark 2> report.json

# Python profiling
viy --py-filter "*train.py" python train.py

# Custom sampling
viy --interval 100 ./long-job
```

## Output

Report goes to stderr (stdout is unmodified):

1. **Header** — command, exit code, wall time
2. **Summary** — CPU, memory, I/O, network, threads, file descriptors
3. **Charts** — time-series for CPU, memory, I/O, network, threads
4. **Python Timeline** — function-level profiling (when tracing Python)
5. **Insights** — automated analysis (e.g., "CPU-bound", "Memory leak")

Use `--json` for structured output to stderr.

## Metrics

- **CPU** — user/system time, utilization %
- **Memory** — RSS, VSZ, page faults
- **I/O** — bytes read/written, syscalls, cache hit ratio
- **Network** — bytes sent/received (host namespace)
- **Threads** — count over time
- **Context switches** — voluntary/involuntary
- **File descriptors** — count over time
- **Process tree** — aggregates all child processes

## Insights

Auto-generated performance analysis:

- `[CPU]` — CPU-bound, I/O-bound, kernel-heavy, high contention
- `[MEM]` — Memory leaks, peak timing, page faults
- `[I/O]` — Cache hit ratio, small ops, throughput patterns
- `[THR]` — Dynamic thread creation
- `[FD]` — File descriptor leaks

## How It Works

Spawns your command and polls `/proc/[pid]/{stat,io,status,fd}` at 10ms intervals. Aggregates metrics across the full process tree. Overhead < 0.1% CPU.

## Requirements

- Linux (uses `/proc` filesystem)
- Rust 1.85+ (for building)

## License

MIT
