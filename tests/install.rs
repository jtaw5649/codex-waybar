use predicates::prelude::*;
use std::error::Error;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn install_creates_waybar_backup() -> TestResult {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let temp = TempDir::new()?;
    let home = temp.path().join("home");
    let prefix = temp.path().join("prefix");
    let bin_dir = prefix.join("bin");
    let share_dir = prefix.join("share/codex-waybar");
    let backups_root = temp.path().join("backups");
    let systemd_dir = temp.path().join("systemd");

    let waybar_config = home.join(".config/waybar");
    fs::create_dir_all(&waybar_config)?;
    fs::write(waybar_config.join("config.jsonc"), b"{}")?;

    let target_dir = repo_root.join("target/release");
    fs::create_dir_all(&target_dir)?;
    let binary_path = target_dir.join("codex-waybar");
    fs::write(&binary_path, b"binary")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&binary_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&binary_path, perms)?;
    }

    let release_staging = temp.path().join("release");
    fs::create_dir_all(&release_staging)?;
    fs::write(release_staging.join("codex-waybar"), fs::read(&binary_path)?)?;
    fs::copy(repo_root.join("README.md"), release_staging.join("README.md"))?;
    let release_examples = release_staging.join("examples");
    fs::create_dir_all(&release_examples)?;
    for entry in fs::read_dir(repo_root.join("examples"))? {
        let entry = entry?;
        fs::copy(entry.path(), release_examples.join(entry.file_name()))?;
    }
    let release_systemd = release_staging.join("systemd");
    fs::create_dir_all(&release_systemd)?;
    if let Ok(_) = fs::metadata(repo_root.join("systemd/codex-waybar.service")) {
        fs::copy(
            repo_root.join("systemd/codex-waybar.service"),
            release_systemd.join("codex-waybar.service"),
        )?;
    }

    let release_archive = temp.path().join("codex-waybar-release.tar.gz");
    std::process::Command::new("tar")
        .arg("-czf")
        .arg(&release_archive)
        .arg("-C")
        .arg(&release_staging)
        .arg(".")
        .status()
        .map_err(|e| format!("failed to create release archive: {}", e))?
        .success()
        .then_some(())
        .ok_or_else(|| "tar command failed".to_string())
        .map_err(|e| -> Box<dyn Error> { e.into() })?;

    let output = std::process::Command::new("/usr/bin/env")
        .current_dir(repo_root)
        .arg("bash")
        .arg(repo_root.join("install.sh"))
        .env("HOME", &home)
        .env("PREFIX", &prefix)
        .env("BIN_DIR", &bin_dir)
        .env("SHARE_DIR", &share_dir)
        .env("SYSTEMD_USER_DIR", &systemd_dir)
        .env("WAYBAR_CONFIG_DIR", &waybar_config)
        .env("WAYBAR_BACKUP_ROOT", &backups_root)
        .env("CODEX_WAYBAR_SKIP_BUILD", "1")
        .env("CODEX_WAYBAR_SKIP_MESON", "1")
        .env("CODEX_WAYBAR_SKIP_SYSTEMD", "1")
        .env("CODEX_WAYBAR_SKIP_WAYBAR_RESTART", "1")
        .env("CODEX_WAYBAR_RELEASE_FILE", &release_archive)
        .output()
        .map_err(|e| format!("failed to run install.sh: {}", e))?;

    assert!(
        output.status.success(),
        "install script failed: {}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        predicate::str::contains("Waybar configuration backup stored at").eval(&stdout),
        "stdout missing backup message: {}",
        stdout
    );

    let backups: Vec<_> = fs::read_dir(&backups_root)?.collect();
    assert_eq!(backups.len(), 1, "expected exactly one backup directory");
    let backup_path = backups[0].as_ref().unwrap().path();
    assert!(
        backup_path.join("config.jsonc").exists(),
        "backup file missing"
    );

    assert!(
        bin_dir.join("codex-waybar").exists(),
        "binary should remain installed"
    );
    assert!(
        !systemd_dir.join("codex-waybar.service").exists(),
        "systemd unit should not exist in skip mode"
    );

    Ok(())
}
