use std::process::Command;

#[test]
fn test_cli_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_wyrd-wallpaper"))
        .arg("--help")
        .output()
        .expect("Failed to execute wyrd-wallpaper binary");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Wyrd Wayland wallpaper daemon"));
    assert!(stdout.contains("run"));
    assert!(stdout.contains("set"));
    assert!(stdout.contains("list"));
    assert!(stdout.contains("current"));
    assert!(stdout.contains("select"));
    assert!(stdout.contains("color"));
}

#[test]
fn test_subcommands_help() {
    for subcmd in &["run", "set", "list", "current", "select", "color"] {
        let output = Command::new(env!("CARGO_BIN_EXE_wyrd-wallpaper"))
            .arg(subcmd)
            .arg("--help")
            .output()
            .unwrap_or_else(|_| panic!("Failed to run wyrd-wallpaper {subcmd} --help"));

        assert!(output.status.success(), "Command '{subcmd} --help' failed");
    }
}
