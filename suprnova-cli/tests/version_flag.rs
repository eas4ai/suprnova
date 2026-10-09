use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

fn combined(out: &Output) -> String {
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    s
}

fn run(flag: &str) -> String {
    let out = Command::new(BIN)
        .arg(flag)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {BIN} {flag}: {e}"));
    assert!(
        out.status.success(),
        "`suprnova {flag}` exited {:?}: {}",
        out.status.code(),
        combined(&out)
    );
    combined(&out)
}

#[test]
fn version_flags_report_the_crate_version() {
    let expected = format!("suprnova {}", env!("CARGO_PKG_VERSION"));
    for flag in ["--version", "-v", "-V"] {
        assert!(
            run(flag).contains(&expected),
            "`suprnova {flag}` must print {expected:?}"
        );
    }
}

#[test]
fn help_still_prints_the_banner() {
    let help = run("--help");
    assert!(help.contains("USAGE"), "{help}");
    assert!(!help.starts_with("suprnova 0."), "{help}");
}
