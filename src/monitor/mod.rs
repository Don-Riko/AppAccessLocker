pub mod activity;
pub mod presence;

use std::process::Command;

/// Congela el proceso objetivo (SIGSTOP).
pub fn freeze(pid: u32) {
    Command::new("kill").args(["-STOP", &pid.to_string()]).status().ok();
}

/// Reanuda el proceso objetivo (SIGCONT).
pub fn resume(pid: u32) {
    Command::new("kill").args(["-CONT", &pid.to_string()]).status().ok();
}
