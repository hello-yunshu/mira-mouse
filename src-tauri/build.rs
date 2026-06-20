// SPDX-License-Identifier: AGPL-3.0-or-later
fn main() {
    tauri_build::build();

    // 注入构建信息：Git commit 和构建日期
    let git_commit = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_COMMIT={}", git_commit);

    let build_date = std::process::Command::new("git")
        .args(["log", "-1", "--format=%cd", "--date=short"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=BUILD_DATE={}", build_date);

    println!("cargo:rerun-if-changed=.git/HEAD");
}
