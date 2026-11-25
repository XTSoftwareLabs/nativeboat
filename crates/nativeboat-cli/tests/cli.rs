use std::process::Command;

#[test]
fn the_binary_reports_its_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_nativeboat"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.starts_with("nativeboat "));
}
