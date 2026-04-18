use assert_cmd::Command;
use predicates::prelude::*;

fn viy() -> Command {
    Command::cargo_bin("viy").unwrap()
}

#[test]
fn basic_execution_passes_stdout() {
    viy()
        .args(["echo", "hello world"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hello world"));
}

#[test]
fn exit_code_forwarded() {
    viy().args(["false"]).assert().code(1);
}

#[test]
fn exit_code_zero() {
    viy().args(["true"]).assert().success();
}

#[test]
fn stderr_passthrough() {
    viy()
        .args(["sh", "-c", "echo error_msg >&2"])
        .assert()
        .success()
        .stderr(predicate::str::contains("error_msg"));
}

#[test]
fn report_printed_to_stderr() {
    viy()
        .args(["true"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Command:"))
        .stderr(predicate::str::contains("Exit code:"))
        .stderr(predicate::str::contains("Wall time:"))
        .stderr(predicate::str::contains("CPU:"));
}

#[test]
fn json_output_is_valid() {
    let output = viy().args(["--json", "--silent", "true"]).output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should be valid JSON
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stderr);
    assert!(parsed.is_ok(), "JSON parse failed: {}", stderr);

    let json = parsed.unwrap();
    assert!(json["command"].is_string());
    assert!(json["exit_code"].is_number());
    assert!(json["wall_time"].is_object());
    assert!(json["summary"].is_object());
    assert!(json["insights"].is_array());
}

#[test]
fn quiet_mode_suppresses_insights() {
    let output = viy().args(["--quiet", "true"]).output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("Insights:"));
    // But the report should still be there
    assert!(stderr.contains("Command:"));
}

#[test]
fn silent_mode_suppresses_report() {
    let output = viy().args(["--silent", "true"]).output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("Command:"));
    assert!(!stderr.contains("CPU usage:"));
}

#[test]
fn no_color_has_no_ansi_escapes() {
    let output = viy().args(["--no-color", "true"]).output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    // ANSI escape codes start with \x1b[
    assert!(
        !stderr.contains("\x1b["),
        "Found ANSI escape codes in --no-color output"
    );
}

#[test]
fn custom_interval() {
    // Just verify it doesn't crash with a custom interval
    viy()
        .args(["--interval", "10", "sleep", "0.1"])
        .assert()
        .success();
}

#[test]
fn samples_collected_for_longer_process() {
    let output = viy()
        .args(["--json", "--silent", "sleep", "0.3"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    let json: serde_json::Value = serde_json::from_str(&stderr).expect("valid JSON");

    let samples = json["samples"].as_array().expect("samples is array");
    // With 50ms default interval and 300ms sleep, we should get some samples
    assert!(
        samples.len() >= 2,
        "Expected at least 2 samples, got {}",
        samples.len()
    );
}

#[test]
fn process_tree_tracking() {
    let output = viy()
        .args([
            "--json",
            "--silent",
            "sh",
            "-c",
            "sleep 0.2 & sleep 0.2 & wait",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    let json: serde_json::Value = serde_json::from_str(&stderr).expect("valid JSON");

    let child_count = json["summary"]["child_processes_spawned"]
        .as_u64()
        .unwrap_or(0);
    assert!(
        child_count >= 2,
        "Expected at least 2 child processes, got {}",
        child_count
    );
}

#[test]
fn missing_command_shows_error() {
    viy()
        .args(["nonexistent_command_12345"])
        .assert()
        .code(127)
        .stderr(predicate::str::contains("failed to execute"));
}

#[test]
fn no_arguments_shows_help() {
    viy()
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage"));
}

#[test]
fn memory_reported_for_running_process() {
    let output = viy()
        .args(["--json", "--silent", "sleep", "0.3"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    let json: serde_json::Value = serde_json::from_str(&stderr).expect("valid JSON");

    let peak_rss = json["summary"]["peak_rss_bytes"].as_u64().unwrap_or(0);
    // sleep should use at least some memory
    assert!(peak_rss > 0, "Expected non-zero peak RSS");
}

#[test]
fn io_bytes_captured_for_writing_process() {
    // Python writing in a loop — slow enough to be sampled
    let output = viy()
        .args([
            "--json",
            "--silent",
            "python3",
            "-c",
            "import os,time; f=open('/tmp/viy_test_io','wb'); [f.write(b'x'*10000) for _ in range(1000)]; f.flush(); time.sleep(0.05); f.close(); os.unlink('/tmp/viy_test_io')",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    let json: serde_json::Value = serde_json::from_str(&stderr).expect("valid JSON");

    let total_wchar = json["summary"]["total_wchar"].as_u64().unwrap_or(0);
    assert!(
        total_wchar > 1_000_000,
        "Expected wchar > 1MB, got {}",
        total_wchar
    );
}

#[test]
fn io_bytes_captured_for_reading_process() {
    // Python reading in a loop — measurable I/O
    let output = viy()
        .args([
            "--json",
            "--silent",
            "python3",
            "-c",
            "import time; f=open('/dev/zero','rb'); [f.read(10000) for _ in range(1000)]; time.sleep(0.05); f.close()",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    let json: serde_json::Value = serde_json::from_str(&stderr).expect("valid JSON");

    let total_rchar = json["summary"]["total_rchar"].as_u64().unwrap_or(0);
    assert!(
        total_rchar > 1_000_000,
        "Expected rchar > 1MB, got {}",
        total_rchar
    );
}

#[test]
fn io_watermarks_survive_child_exit() {
    // Python process doing measurable I/O, then exiting — watermarks should persist
    let output = viy()
        .args([
            "--json",
            "--silent",
            "python3",
            "-c",
            "import os,time; f=open('/tmp/viy_wm','wb'); [f.write(b'y'*10000) for _ in range(500)]; f.flush(); time.sleep(0.05); f.close(); os.unlink('/tmp/viy_wm')",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    let json: serde_json::Value = serde_json::from_str(&stderr).expect("valid JSON");

    let total_wchar = json["summary"]["total_wchar"].as_u64().unwrap_or(0);
    assert!(
        total_wchar > 1_000_000,
        "Expected wchar > 1MB from python write, got {}",
        total_wchar
    );
}

#[test]
fn network_shows_host_label() {
    let output = viy().args(["--no-color", "sleep", "0.1"]).output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Network line should indicate it's host-level
    assert!(
        stderr.contains("(host)"),
        "Network line should contain '(host)' label"
    );
}

#[test]
fn io_chart_shows_in_report() {
    let output = viy().args(["--no-color", "sleep", "0.2"]).output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("I/O") && stderr.contains("Rate"),
        "Report should contain I/O rate chart"
    );
}

#[test]
#[ignore] // slow (~5s): run with `cargo test -- --ignored`
fn workload_metrics() {
    let workload = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/test_workload.py");
    let output = viy()
        .args(["--json", "--silent", "python3", "-W", "ignore", workload])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    let json: serde_json::Value = serde_json::from_str(&stderr).expect("valid JSON");
    let summary = &json["summary"];

    // ~50MB written
    let wchar = summary["total_wchar"].as_u64().unwrap_or(0);
    assert!(wchar > 40_000_000, "expected >40MB wchar, got {wchar}");

    // ~50MB read
    let rchar = summary["total_rchar"].as_u64().unwrap_or(0);
    assert!(rchar > 40_000_000, "expected >40MB rchar, got {rchar}");

    // peak RSS ~100MB
    let peak_rss = summary["peak_rss_bytes"].as_u64().unwrap_or(0);
    assert!(
        peak_rss > 50_000_000,
        "expected >50MB peak RSS, got {peak_rss}"
    );

    // child processes spawned
    let children = summary["child_processes_spawned"].as_u64().unwrap_or(0);
    assert!(
        children >= 5,
        "expected >=5 child processes, got {children}"
    );
}

#[test]
fn default_sampling_interval_is_10ms() {
    // With 10ms default and 500ms sleep, should get plenty of samples
    let output = viy()
        .args(["--json", "--silent", "sleep", "0.5"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    let json: serde_json::Value = serde_json::from_str(&stderr).expect("valid JSON");

    let samples = json["samples"].as_array().expect("samples is array");
    assert!(
        samples.len() >= 10,
        "Expected >= 10 samples with 10ms interval over 500ms, got {}",
        samples.len()
    );
}

#[test]
fn python_tracing_produces_py_insights() {
    let output = viy()
        .args([
            "--no-color",
            "python3",
            "-c",
            "import time; end=time.time()+0.5\nwhile time.time()<end: pass",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[PY]"),
        "Expected [PY] insights for python3 command, got:\n{stderr}"
    );
}

#[test]
fn py_filter_shows_timeline() {
    let workload = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/test_workload.py");
    let output = viy()
        .args([
            "--no-color",
            "--py-filter",
            "*test_workload.py",
            "python3",
            workload,
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Python Timeline"),
        "Expected Python Timeline section with --py-filter"
    );
    // Timeline rows with --py-filter on a directly-run script show __main__
    let tag_lines: Vec<_> = stderr
        .lines()
        .filter(|l| {
            l.contains("__main__")
                && (l.contains("[CPU]") || l.contains("[MEM]") || l.contains("[I/O]"))
        })
        .collect();
    assert!(
        !tag_lines.is_empty(),
        "Expected at least one tagged timeline row, stderr:\n{stderr}"
    );
}

#[test]
fn py_filter_restricts_to_matching_file() {
    let workload = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/test_workload.py");
    let output = viy()
        .args([
            "--no-color",
            "--py-filter",
            "*test_workload.py",
            "python3",
            workload,
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    // When run directly, Python sets __name__ = '__main__'
    for line in stderr.lines().filter(|l| l.contains("[PY]")) {
        assert!(
            line.contains("__main__"),
            "Expected __main__ module in filtered [PY] line: {line}"
        );
    }
}

#[test]
fn non_python_command_has_no_py_insights() {
    let output = viy().args(["--no-color", "sleep", "0.1"]).output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("[PY]"),
        "Expected no [PY] insights for non-Python command"
    );
    assert!(
        !stderr.contains("Python Timeline"),
        "Expected no Python Timeline for non-Python command"
    );
}

#[test]
fn py_filter_no_match_produces_no_py_insights() {
    let output = viy()
        .args([
            "--no-color",
            "--py-filter",
            "*nonexistent_file.py",
            "python3",
            "-c",
            "import time; time.sleep(0.1)",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("[PY]"),
        "Expected no [PY] insights when filter matches nothing"
    );
}
