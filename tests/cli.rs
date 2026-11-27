//! The command line, without an I2C bus.

use std::process::Command;

fn rtc_probe(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rtc-probe"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn help_is_printed() {
    let output = rtc_probe(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage: rtc-probe"));
    assert!(stdout.contains("--bind"));
}

#[test]
fn the_version_is_printed() {
    let output = rtc_probe(&["--version"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("rtc-probe {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn an_unknown_argument_is_an_error() {
    let output = rtc_probe(&["--frobnicate"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("unknown argument `--frobnicate`"));
}

#[test]
fn a_bad_bus_number_is_an_error() {
    let output = rtc_probe(&["--bus", "one"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("bad bus `one`"));
}

#[test]
fn a_missing_bus_is_an_error() {
    let output = rtc_probe(&["--bus", "4242"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("/dev/i2c-4242"));
}
