use assert_cmd::Command;
use std::error::Error;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use tempfile::TempDir;

type TestResult = Result<(), Box<dyn Error>>;

fn write_stub(path: &Path, body: &str) -> TestResult {
    let mut file = File::create(path)?;
    writeln!(file, "#!/usr/bin/env bash\n{}", body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = file.metadata()?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[test]
fn uninstall_removes_installed_artifacts() -> TestResult {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let temp = TempDir::new()?;
    let prefix = temp.path().join("prefix");
    let bin_dir = prefix.join("bin");
    let share_dir = prefix.join("share/codex-waybar");
    let examples_dir = share_dir.join("examples");
    let lib_dir = prefix.join("lib/waybar");
    let lib64_dir = prefix.join("lib64/waybar");
    let systemd_dir = temp.path().join("systemd");

    fs::create_dir_all(&bin_dir)?;
    fs::create_dir_all(&examples_dir)?;
    fs::create_dir_all(&share_dir)?;
    fs::create_dir_all(&lib_dir)?;
    fs::create_dir_all(&lib64_dir)?;
    fs::create_dir_all(&systemd_dir)?;
    eprintln!("directories prepared");

    fs::write(bin_dir.join("codex-waybar"), b"binary")?;
    fs::write(share_dir.join("README.md"), b"docs")?;
    eprintln!("seeded binary and README");

    for sample in [
        "codex-waybar.service",
        "waybar-config-snippet.jsonc",
        "waybar-style.css",
    ] {
        let source = repo_root.join("examples").join(sample);
        let dest = examples_dir.join(sample);
        fs::copy(&source, &dest).unwrap_or_else(|e| {
            panic!("failed to copy {} to {}: {}", source.display(), dest.display(), e)
        });
    }
    eprintln!("copied example files");

    fs::write(lib_dir.join("wb_codex_shimmer.so"), b"plugin")?;
    fs::write(lib64_dir.join("wb_codex_shimmer.so"), b"plugin64")?;
    eprintln!("seeded plugin files");

    let service_src = repo_root.join("systemd/codex-waybar.service");
    let service_dest = systemd_dir.join("codex-waybar.service");
    fs::copy(&service_src, &service_dest).unwrap_or_else(|e| {
        panic!("failed to copy {} to {}: {}", service_src.display(), service_dest.display(), e)
    });
    eprintln!("copied systemd unit");

    let stubs_dir = temp.path().join("stubs");
    fs::create_dir_all(&stubs_dir)?;

    let systemctl_log = stubs_dir.join("systemctl.log");
    let waybar_log = stubs_dir.join("waybar.log");
    let pkill_log = stubs_dir.join("pkill.log");

    File::create(&systemctl_log)?;
    File::create(&waybar_log)?;
    File::create(&pkill_log)?;

    write_stub(
        &stubs_dir.join("systemctl"),
        "echo systemctl $@ >> \"${SYSTEMCTL_LOG}\"",
    )?;
    write_stub(
        &stubs_dir.join("waybar"),
        "echo waybar $@ >> \"${WAYBAR_LOG}\"",
    )?;
    write_stub(
        &stubs_dir.join("pkill"),
        "echo pkill $@ >> \"${PKILL_LOG}\"",
    )?;
    eprintln!("stubs prepared");

    let path_env = format!(
        "{}:{}",
        stubs_dir.display(),
        std::env::var("PATH")?
    );
    eprintln!("PATH for script: {}", path_env);

    let which_output = Command::new("bash")
        .arg("-lc")
        .arg("command -v waybar")
        .env("PATH", &path_env)
        .output()?;
    eprintln!(
        "which waybar status: {:?}, stdout: {}",
        which_output.status.code(),
        String::from_utf8_lossy(&which_output.stdout)
    );

    let run_uninstall = |prefix: &Path, bin_dir: &Path, share_dir: &Path, systemd_dir: &Path| -> TestResult {
        let mut cmd = Command::new("scripts/uninstall.sh");
        let output = cmd
            .current_dir(repo_root)
            .env("PREFIX", prefix)
            .env("BIN_DIR", bin_dir)
            .env("SHARE_DIR", share_dir)
            .env("SYSTEMD_USER_DIR", systemd_dir)
            .env("SYSTEMCTL_LOG", &systemctl_log)
            .env("WAYBAR_LOG", &waybar_log)
            .env("PKILL_LOG", &pkill_log)
            .env("PATH", &path_env)
            .output();

        match output {
            Ok(out) => {
                if !out.status.success() {
                    panic!(
                        "uninstall.sh failed: status {:?}\nstdout: {}\nstderr: {}",
                        out.status.code(),
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    );
                }
                Ok(())
            }
            Err(err) => panic!("failed to run uninstall.sh: {}", err),
        }
    };

    run_uninstall(&prefix, &bin_dir, &share_dir, &systemd_dir)?;
    std::thread::sleep(std::time::Duration::from_millis(50));

    assert!(!bin_dir.join("codex-waybar").exists(), "binary should be removed");
    assert!(!share_dir.exists(), "share directory should be removed when empty");
    assert!(!lib_dir.join("wb_codex_shimmer.so").exists());
    assert!(!lib64_dir.join("wb_codex_shimmer.so").exists());
    assert!(!systemd_dir.join("codex-waybar.service").exists());

    let systemctl_calls = fs::read_to_string(&systemctl_log)?;
    eprintln!("systemctl calls: {}", systemctl_calls);
    assert!(systemctl_calls.contains("systemctl --user stop codex-waybar.service"));
    assert!(systemctl_calls.contains("systemctl --user disable codex-waybar.service"));
    assert!(systemctl_calls.contains("systemctl --user daemon-reload"));

    let waybar_calls = fs::read_to_string(&waybar_log)?;
    eprintln!("waybar calls: {}", waybar_calls);
    assert!(waybar_calls.contains("waybar"));

    let pkill_calls = fs::read_to_string(&pkill_log)?;
    eprintln!("pkill calls: {}", pkill_calls);
    assert!(pkill_calls.contains("pkill waybar"));

    // Second run should be idempotent and still succeed.
    run_uninstall(&prefix, &bin_dir, &share_dir, &systemd_dir)?;
    std::thread::sleep(std::time::Duration::from_millis(50));

    Ok(())
}
