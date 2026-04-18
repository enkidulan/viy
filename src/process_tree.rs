use std::collections::HashSet;

/// Discovers all descendant PIDs of the given root PID by scanning /proc.
/// Returns a set of PIDs including the root.
fn discover_tree(root_pid: i32) -> HashSet<i32> {
    let mut pids = HashSet::new();
    pids.insert(root_pid);

    // Iteratively discover children until no new ones are found
    let mut changed = true;
    while changed {
        changed = false;
        if let Ok(all_procs) = procfs::process::all_processes() {
            for proc_result in all_procs {
                let Ok(proc) = proc_result else { continue };
                let Ok(stat) = proc.stat() else { continue };

                let ppid = stat.ppid;
                let pid = stat.pid;

                if pids.contains(&ppid) && !pids.contains(&pid) {
                    pids.insert(pid);
                    changed = true;
                }
            }
        }
    }

    pids
}

/// Count total unique child PIDs seen (excluding the root).
pub struct TreeTracker {
    root_pid: i32,
    seen_pids: HashSet<i32>,
}

impl TreeTracker {
    pub fn new(root_pid: i32) -> Self {
        Self {
            root_pid,
            seen_pids: HashSet::new(),
        }
    }

    /// Scan for new descendants. Returns the current set of live descendant PIDs.
    pub fn update(&mut self) -> HashSet<i32> {
        let tree = discover_tree(self.root_pid);
        for &pid in &tree {
            if pid != self.root_pid {
                self.seen_pids.insert(pid);
            }
        }
        tree
    }

    /// Total unique child processes ever seen.
    pub fn total_children_seen(&self) -> u32 {
        self.seen_pids.len() as u32
    }
}
