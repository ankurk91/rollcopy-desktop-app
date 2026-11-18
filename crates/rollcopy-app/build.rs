//! Adds the short git commit to the version shown in the About dialog
//! (PLAN §8b) as `ROLLCOPY_GIT_HASH`, or "unknown" outside a git checkout.

use std::process::Command;

/// `git <args>`'s output, trimmed, if it succeeded.
fn git(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn main() {
    let hash = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=ROLLCOPY_GIT_HASH={hash}");
    // Rebuild when the checked-out commit changes: HEAD, the branch it
    // points at, and packed-refs (where refs go after `git gc`). Paths come
    // from git, so worktrees (where `.git` is a file) work too (review B3).
    let Some(git_dir) = git(&["rev-parse", "--absolute-git-dir"]) else {
        return;
    };
    let common = git(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .unwrap_or_else(|| git_dir.clone());
    println!("cargo:rerun-if-changed={git_dir}/HEAD");
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"]) {
        println!("cargo:rerun-if-changed={common}/{branch}");
    }
    println!("cargo:rerun-if-changed={common}/packed-refs");
}
