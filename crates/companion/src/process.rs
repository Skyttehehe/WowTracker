//! "Is the game running?" — used to mark the character offline when the game
//! closes or crashes without a clean logout.

/// `Some(true/false)` when the process list could be read, `None` otherwise.
pub fn game_running(process_names: &[String]) -> Option<bool> {
    let names: Vec<String> = process_names.iter().map(|n| n.to_lowercase()).collect();
    if names.is_empty() {
        return None;
    }
    imp::running_processes().map(|procs| procs.iter().any(|p| matches(&names, p)))
}

fn matches(names: &[String], process: &str) -> bool {
    let p = process.to_lowercase();
    // Wine shows the Windows path in the command line, e.g. "C:\...\Wow.exe".
    let base = p.rsplit(['\\', '/']).next().unwrap_or(&p);
    names.iter().any(|n| base == n || base.split_whitespace().next() == Some(n.as_str()))
}

#[cfg(windows)]
mod imp {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn running_processes() -> Option<Vec<String>> {
        let out = Command::new("tasklist")
            .args(["/FO", "CSV", "/NH"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        Some(
            text.lines()
                .filter_map(|l| l.strip_prefix('"').and_then(|l| l.split('"').next()))
                .map(str::to_string)
                .collect(),
        )
    }
}

#[cfg(target_os = "linux")]
mod imp {
    pub fn running_processes() -> Option<Vec<String>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir("/proc").ok()?.flatten() {
            let name = entry.file_name();
            if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            if let Ok(comm) = std::fs::read_to_string(entry.path().join("comm")) {
                out.push(comm.trim().to_string());
            }
            if let Ok(cmd) = std::fs::read(entry.path().join("cmdline")) {
                if let Some(first) = cmd.split(|&b| b == 0).next() {
                    out.push(String::from_utf8_lossy(first).into_owned());
                }
            }
        }
        Some(out)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    pub fn running_processes() -> Option<Vec<String>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_plain_and_wine_names() {
        let names = vec!["wow.exe".to_string()];
        assert!(matches(&names, "Wow.exe"));
        assert!(matches(&names, "C:\\Program Files (x86)\\World of Warcraft\\_forever_\\Wow.exe"));
        assert!(!matches(&names, "WowUp.exe"));
        assert!(!matches(&names, "explorer.exe"));
    }
}
