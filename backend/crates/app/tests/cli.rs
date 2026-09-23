//! The process contract from the design's section 3.

use std::process::Command;

fn fau() -> Command {
    Command::new(env!("CARGO_BIN_EXE_fau"))
}

fn without_environment(cmd: &mut Command) -> &mut Command {
    cmd.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
}

#[test]
fn version_prints_the_build_revision_and_exits_zero() {
    let out = fau().arg("--version").output().expect("run fau --version");
    assert!(out.status.success(), "status was {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("fau "), "unexpected output: {stdout}");
}

#[test]
fn version_reads_no_configuration() {
    // Section 3: --version reads no configuration and touches no secret. With every
    // required variable absent it must still succeed.
    let mut cmd = fau();
    let out = without_environment(cmd.arg("--version"))
        .output()
        .expect("run fau --version");
    assert!(out.status.success(), "status was {:?}", out.status);
}

#[test]
fn unknown_subcommand_exits_non_zero() {
    let out = fau().arg("frobnicate").output().expect("run fau");
    assert!(!out.status.success());
}

#[test]
fn version_is_global_and_works_after_a_subcommand() {
    // --version must win even when it follows `serve`, without reading serve's
    // configuration -- an operator reaching for `fau serve --version` should not
    // need to know the flag only "works" before the subcommand.
    let mut cmd = fau();
    cmd.args(["serve", "--version"]);
    let out = without_environment(&mut cmd)
        .output()
        .expect("run fau serve --version");
    assert!(out.status.success(), "status was {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("fau "), "unexpected output: {stdout}");
}
