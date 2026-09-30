use anyhow::{Context, Result};

use crate::output::{print_info, print_success, print_warning};

fn service_content(exe_path: &str) -> String {
    format!(
        "[Unit]\n\
         Description=binto update check\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         ExecStart={exe_path} check\n"
    )
}

const TIMER: &str = "[Unit]
Description=Run binto update check periodically

[Timer]
OnCalendar=daily
Persistent=true

[Install]
WantedBy=timers.target
";

fn systemd_user_dir() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".config"))
        .join("systemd/user")
}

pub fn cmd_setup_timer() -> Result<()> {
    let exe = std::env::current_exe().context("failed to resolve current executable path")?;
    let exe_path = exe
        .to_str()
        .context("executable path contains non-UTF-8 characters")?;

    let systemd_dir = systemd_user_dir();
    std::fs::create_dir_all(&systemd_dir)
        .with_context(|| format!("failed to create {}", systemd_dir.display()))?;

    let service_path = systemd_dir.join("binto.service");
    let timer_path = systemd_dir.join("binto.timer");

    std::fs::write(&service_path, service_content(exe_path))
        .with_context(|| format!("failed to write {}", service_path.display()))?;
    std::fs::write(&timer_path, TIMER)
        .with_context(|| format!("failed to write {}", timer_path.display()))?;

    print_success(&format!(
        "Written {} (ExecStart={})",
        service_path.display(),
        exe_path
    ));
    print_success(&format!("Written {}", timer_path.display()));

    let enable = dialoguer::Confirm::new()
        .with_prompt(
            "Enable and start binto.timer now? (systemctl --user enable --now binto.timer)",
        )
        .default(true)
        .interact()
        .context("failed to read user input")?;

    if enable {
        let status = std::process::Command::new("systemctl")
            .args(["--user", "enable", "--now", "binto.timer"])
            .status()
            .context("failed to run systemctl")?;

        if status.success() {
            print_success("binto.timer enabled and started.");
        } else {
            anyhow::bail!("systemctl exited with status {status}");
        }
    } else {
        print_info("To enable later: systemctl --user enable --now binto.timer");
    }

    Ok(())
}

pub fn cmd_disable_timer() -> Result<()> {
    let systemd_dir = systemd_user_dir();
    let service_path = systemd_dir.join("binto.service");
    let timer_path = systemd_dir.join("binto.timer");

    if !timer_path.exists() && !service_path.exists() {
        print_info("No binto timer files found — nothing to disable.");
        return Ok(());
    }

    let disable = dialoguer::Confirm::new()
        .with_prompt(
            "Disable and remove binto.timer now? (systemctl --user disable --now binto.timer)",
        )
        .default(false)
        .interact()
        .context("failed to read user input")?;

    if disable {
        // Stop and disable the timer unit first (best-effort; ignore if systemd unavailable)
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "disable", "--now", "binto.timer"])
            .status();

        for path in [&timer_path, &service_path] {
            if path.exists() {
                std::fs::remove_file(path)
                    .with_context(|| format!("failed to remove {}", path.display()))?;
                print_success(&format!("Removed {}", path.display()));
            } else {
                print_warning(&format!("{} not found, skipping.", path.display()));
            }
        }
        print_info("Run `systemctl --user daemon-reload` if you manage units manually.");
    }

    Ok(())
}
