//! Finds the game folder and the addon's SavedVariables files.

use std::path::{Path, PathBuf};

pub const ADDON_FOLDER: &str = "WowTracker";
pub const SAVED_VARIABLES_FILE: &str = "WowTracker.lua";

/// Returns the child of `dir` whose name matches `name` case-insensitively.
/// (Matters on Linux, where the game runs under Wine.)
fn child_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.exists() {
        return Some(exact);
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|e| e.path())
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    v.sort();
    v
}

/// A game flavor folder (`_forever_`, `_retail_`, ...) has a WTF or Interface folder.
fn is_flavor_dir(p: &Path) -> bool {
    child_ci(p, "WTF").is_some() || child_ci(p, "Interface").is_some()
}

fn has_addon(flavor: &Path) -> bool {
    child_ci(flavor, "Interface")
        .and_then(|i| child_ci(&i, "AddOns"))
        .and_then(|a| child_ci(&a, ADDON_FOLDER))
        .is_some()
}

/// Picks the best flavor folder under a "World of Warcraft" folder: one with
/// the addon installed, preferring Forever.
fn best_flavor(root: &Path) -> Option<PathBuf> {
    let flavors: Vec<PathBuf> = subdirs(root)
        .into_iter()
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('_')) && is_flavor_dir(p))
        .collect();
    let score = |p: &PathBuf| {
        let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let mut s = 0;
        if has_addon(p) {
            s += 10;
        }
        if name.contains("forever") || name.contains("camelot") {
            s += 5;
        }
        if child_ci(p, "WTF").is_some() {
            s += 1;
        }
        s
    };
    flavors.into_iter().max_by_key(score)
}

/// Accepts either a flavor folder (contains WTF) or the "World of Warcraft" folder.
pub fn resolve_game_dir(path: &Path) -> Option<PathBuf> {
    if child_ci(path, "WTF").is_some() {
        return Some(path.to_path_buf());
    }
    best_flavor(path)
}

/// Reads the `InstallPath` value from `reg query` output. Battle.net stores the
/// last-used flavor folder there (e.g. `E:\Games\World of Warcraft\_forever_\`).
pub fn parse_reg_install_path(output: &str) -> Option<PathBuf> {
    output.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix("InstallPath")?;
        let value = rest.trim_start().strip_prefix("REG_SZ")?.trim();
        (!value.is_empty()).then(|| PathBuf::from(value.trim_end_matches('\\')))
    })
}

/// Install folder recorded by Battle.net in the Windows registry, if any.
fn registry_root() -> Option<PathBuf> {
    let out = std::process::Command::new("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\WOW6432Node\Blizzard Entertainment\World of Warcraft",
            "/v",
            "InstallPath",
        ])
        .output()
        .ok()?;
    let path = parse_reg_install_path(&String::from_utf8_lossy(&out.stdout))?;
    // The value usually points at a flavor folder; its parent holds all flavors.
    if is_flavor_dir(&path) {
        path.parent().map(Path::to_path_buf)
    } else {
        Some(path)
    }
}

fn install_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if cfg!(windows) {
        roots.extend(registry_root());
        for drive in b'C'..=b'H' {
            let d = format!("{}:\\", drive as char);
            if !Path::new(&d).exists() {
                continue;
            }
            for rel in [
                "Program Files (x86)\\World of Warcraft",
                "Program Files\\World of Warcraft",
                "World of Warcraft",
                "Games\\World of Warcraft",
                "Blizzard\\World of Warcraft",
                "Battle.net\\World of Warcraft",
            ] {
                roots.push(Path::new(&d).join(rel));
            }
        }
    } else if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let wow = Path::new("drive_c/Program Files (x86)/World of Warcraft");
        for prefix in [
            "Games/battlenet",
            "Games/battle-net",
            "Games/Battle.net",
            "Games/world-of-warcraft",
            ".wine",
        ] {
            roots.push(home.join(prefix).join(wow));
        }
        // Bottles and Steam/Proton prefixes: one wildcard level.
        for (parent, inner) in [
            (".local/share/bottles/bottles", ""),
            (".var/app/com.usebottles.bottles/data/bottles/bottles", ""),
            (".steam/steam/steamapps/compatdata", "pfx"),
            (".local/share/Steam/steamapps/compatdata", "pfx"),
        ] {
            for d in subdirs(&home.join(parent)) {
                roots.push(d.join(inner).join(wow));
            }
        }
        roots.push(PathBuf::from("/Applications/World of Warcraft"));
    }
    roots
}

/// Looks in the usual install locations.
pub fn auto_detect_game_dir() -> Option<PathBuf> {
    let found: Vec<PathBuf> = install_roots().iter().filter(|r| r.is_dir()).filter_map(|r| best_flavor(r)).collect();
    found.iter().find(|p| has_addon(p)).cloned().or_else(|| found.into_iter().next())
}

/// `<game>/WTF/Account/<ACCOUNT>/SavedVariables/WowTracker.lua` for every account.
pub fn saved_variable_files(game_dir: &Path) -> Vec<PathBuf> {
    let Some(accounts) = child_ci(game_dir, "WTF").and_then(|w| child_ci(&w, "Account")) else {
        return Vec::new();
    };
    subdirs(&accounts)
        .iter()
        .filter_map(|acct| child_ci(acct, "SavedVariables"))
        .filter_map(|sv| child_ci(&sv, SAVED_VARIABLES_FILE))
        .filter(|f| f.is_file())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_files_in_a_fake_install() {
        let root = std::env::temp_dir().join(format!("wowtracker-discover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let flavor = root.join("_forever_");
        std::fs::create_dir_all(flavor.join("Interface/AddOns/WowTracker")).unwrap();
        std::fs::create_dir_all(flavor.join("WTF/Account/ACC1/SavedVariables")).unwrap();
        std::fs::create_dir_all(flavor.join("WTF/Account/ACC2/SavedVariables")).unwrap();
        std::fs::create_dir_all(root.join("_retail_/WTF")).unwrap();
        std::fs::write(flavor.join("WTF/Account/ACC1/SavedVariables/WowTracker.lua"), "WowTrackerDB = {}").unwrap();

        assert_eq!(resolve_game_dir(&root), Some(flavor.clone()));
        assert_eq!(resolve_game_dir(&flavor), Some(flavor.clone()));
        let files = saved_variable_files(&flavor);
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("WowTracker.lua"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parses_registry_install_path() {
        let out = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\Blizzard Entertainment\\World of Warcraft\r\n    \
                   InstallPath    REG_SZ    E:\\Wowclassic\\World of Warcraft\\_forever_\\\r\n\r\n";
        assert_eq!(
            parse_reg_install_path(out),
            Some(PathBuf::from("E:\\Wowclassic\\World of Warcraft\\_forever_"))
        );
        assert_eq!(parse_reg_install_path("ERROR: The system was unable to find the specified registry key"), None);
    }
}
