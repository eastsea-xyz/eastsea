//! Child setup callbacks force Rust's macOS spawn path through a fork, which
//! is unsafe once a multithreaded node has initialized Network.framework.

use std::path::Path;
use std::process::Command;

#[test]
fn workspace_sources_never_install_child_exec_hooks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let mut command = Command::new("git");
    command.arg("-C").arg(root).args([
        "grep",
        "-n",
        "-I",
        "-w",
        "-e",
        "pre_exec",
        "-e",
        "before_exec",
    ]);
    // The same lint can demonstrate the release baseline failing without
    // checking out old code or changing the running worktree.
    if let Some(revision) = std::env::var_os("AETHER_SPAWN_LINT_REVISION") {
        command.arg(revision);
    } else {
        command.args(["--untracked", "--exclude-standard"]);
    }
    let output = command
        .args([
            "--",
            "*.rs",
            "*.swift",
            "*.c",
            "*.h",
            "*.cc",
            "*.cpp",
            "*.m",
            "*.mm",
            ":(exclude,glob)**/tests/**",
            ":(exclude,glob)**/Tests/**",
            ":(exclude,glob)tmp/**",
        ])
        .output()
        .expect("run the workspace source lint");
    assert_eq!(
        output.status.code(),
        Some(1),
        "workspace sources must use fork-free spawn APIs; child exec hooks found:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
