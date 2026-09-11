use std::path::PathBuf;
use std::process::Command;

const MISSING_LOGNAME: &str = "SYSERR: File name to log to expected after option -o.";

struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

#[test]
fn bare_log_option_without_file_name_exits_with_syserr() {
    let out = Command::new(env!("CARGO_BIN_EXE_circle")).arg("-o").output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "stderr: {stderr}");
    assert!(stderr.contains(MISSING_LOGNAME), "stderr: {stderr}");
}

#[test]
fn log_option_with_file_name_is_accepted_by_option_parser() {
    let root = TempDir(std::env::temp_dir().join(format!("rustmud-logopt-{}", std::process::id())));
    std::fs::create_dir_all(&root.0).unwrap();
    let logfile = root.0.join("syslog");
    // -c and a nonexistent lib dir make the boot fail fast after parsing;
    // only the option parser's verdict on -o is under test here.
    let out = Command::new(env!("CARGO_BIN_EXE_circle"))
        .args(["-c", "-d"]).arg(root.0.join("no-such-lib"))
        .arg("-o").arg(&logfile)
        .output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains(MISSING_LOGNAME), "stderr: {stderr}");
    assert!(logfile.exists(), "-o did not open the log file");
}
