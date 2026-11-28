use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
};

use agentdesktop_core::model::Agent;

use crate::provider::metadata;

use super::Goose;

pub(super) fn discover() -> Option<Agent> {
    let executable = metadata::find_executable(Goose::ID, executable_candidates())?;
    Some(Agent {
        version: version(&executable),
        executable,
        kind: Goose::ID.to_owned(),
        mcp_servers: Vec::new(),
        skills: Vec::new(),
    })
}

fn executable_candidates() -> Vec<PathBuf> {
    let mut candidates = BTreeSet::new();
    for home in metadata::user_home_dirs() {
        candidates.insert(home.join(".local/bin/goose"));
        candidates.insert(home.join(".cargo/bin/goose"));
        #[cfg(windows)]
        {
            candidates.insert(home.join(".local/bin/goose.exe"));
            candidates.insert(home.join(".cargo/bin/goose.exe"));
        }
    }
    #[cfg(target_os = "macos")]
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/goose"),
        PathBuf::from("/usr/local/bin/goose"),
    ]);
    candidates.into_iter().collect()
}

fn version(executable: &Path) -> Option<String> {
    let output = Command::new(executable).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let output = String::from_utf8(output.stdout).ok()?;
    output.split_whitespace().find_map(|part| {
        let part = part
            .trim_start_matches('v')
            .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-' && c != '+');
        let core = part.split_once(['-', '+']).map_or(part, |(core, _)| core);
        let components: Vec<_> = core.split('.').collect();
        (components.len() == 3
            && components.iter().all(|component| {
                !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
            }))
        .then(|| part.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::version;

    #[cfg(unix)]
    #[test]
    fn reads_version_from_goose_executable() {
        use std::{fs, os::unix::fs::PermissionsExt};

        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("goose");
        fs::write(&executable, "#!/bin/sh\necho 'goose version v1.2.3'\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(version(&executable).as_deref(), Some("1.2.3"));
    }
}
