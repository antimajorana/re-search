//! Integration test running the offline JS/CSS/KaTeX rendering test suite via `node --test`.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn desktop_ui_math_and_markdown_suite_passes() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("math_render.test.mjs");
    assert!(
        script.exists(),
        "expected JS math test suite at {}",
        script.display()
    );

    let output = Command::new("node")
        .arg("--test")
        .arg(&script)
        .output()
        .expect("failed to execute `node --test` for math_render.test.mjs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "math_render.test.mjs failed (status: {:?})\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        stdout,
        stderr
    );
}

#[test]
fn desktop_ui_math_parity_suite_passes() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("math_parity.test.mjs");
    assert!(
        script.exists(),
        "expected JS math parity test suite at {}",
        script.display()
    );

    let output = Command::new("node")
        .arg("--test")
        .arg(&script)
        .output()
        .expect("failed to execute `node --test` for math_parity.test.mjs");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "math_parity.test.mjs failed (status: {:?})\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        stdout,
        stderr
    );
}
