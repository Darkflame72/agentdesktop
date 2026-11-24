//! GitHub Copilot CLI discovery: the `copilot` executable on `PATH` (or in the
//! usual npm global locations) and its version from the npm package manifest
//! of `@github/copilot`. Nothing is executed: discovery also runs in a
//! system daemon, and a file found in a user's home must never run as root.
//! The MCP and skills inventory under `~/.copilot` stays with the VS Code
//! adapter, which already reads it.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use agentdesktop_core::model::Agent;

use super::Copilot;
use crate::provider::metadata;

const PACKAGE: &str = "@github/copilot";

pub(super) fn discover() -> Option<Agent> {
    let executable = metadata::find_executable("copilot", executable_candidates())?;
    Some(Agent {
        version: npm_version(&executable)
            .or_else(|| standalone_version(&executable))
            .or_else(|| homebrew_version(&executable)),
        executable,
        kind: Copilot::ID.to_owned(),
        mcp_servers: Vec::new(),
        skills: Vec::new(),
    })
}

fn executable_candidates() -> Vec<PathBuf> {
    let mut candidates = BTreeSet::new();
    for home in metadata::user_home_dirs() {
        candidates.insert(home.join(".npm-global/bin/copilot"));
        candidates.insert(home.join(".local/bin/copilot"));
        #[cfg(windows)]
        candidates.insert(home.join("AppData/Roaming/npm/copilot.cmd"));
    }
    #[cfg(target_os = "macos")]
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/copilot"),
        PathBuf::from("/usr/local/bin/copilot"),
    ]);
    candidates.into_iter().collect()
}

/// The version from the `@github/copilot` manifest next to the launcher: npm
/// links `<prefix>/bin/copilot` into `<prefix>/lib/node_modules/@github/copilot`,
/// so the manifest sits in an ancestor of the resolved launcher or in a
/// `node_modules` directory next to one.
pub(super) fn npm_version(executable: &Path) -> Option<String> {
    manifest_candidates(executable)
        .into_iter()
        .find_map(|path| metadata::json_package_version(&path, PACKAGE))
}

/// Standalone releases keep their version in the install directory. Reading
/// that path avoids executing the CLI from a system daemon running as root.
fn standalone_version(executable: &Path) -> Option<String> {
    ["releases", "versions"]
        .into_iter()
        .filter_map(|component| metadata::version_after_component(executable, component))
        .find_map(|release| {
            let version = [
                "-aarch64-",
                "-arm64-",
                "-x86_64-",
                "-x64-",
                "-i686-",
                "-ia32-",
                "-darwin-",
                "-linux-",
                "-win32-",
            ]
            .into_iter()
            .find_map(|target| release.split_once(target).map(|(version, _)| version))
            .unwrap_or(&release);
            is_version(version).then(|| version.to_owned())
        })
}

/// Homebrew links `copilot` into a versioned `Cellar` directory.
fn homebrew_version(executable: &Path) -> Option<String> {
    let resolved = executable.canonicalize().ok()?;
    let components: Vec<_> = resolved
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect();
    let cellar = components.iter().position(|component| *component == "Cellar")?;
    let formula = *components.get(cellar + 1)?;
    if !matches!(formula, "copilot" | "copilot-cli") {
        return None;
    }
    let version = *components.get(cellar + 2)?;
    is_version(version).then(|| version.to_owned())
}

fn is_version(version: &str) -> bool {
    let core = version
        .split_once(['-', '+'])
        .map_or(version, |(core, _)| core);
    let parts: Vec<_> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
        })
}

pub(super) fn manifest_candidates(executable: &Path) -> Vec<PathBuf> {
    let mut candidates = BTreeSet::new();
    for executable in [
        Some(executable.to_path_buf()),
        executable.canonicalize().ok(),
    ]
    .into_iter()
    .flatten()
    {
        let Some(parent) = executable.parent() else {
            continue;
        };
        for directory in parent.ancestors().take(4) {
            candidates.insert(directory.join("package.json"));
            candidates.insert(directory.join(format!("node_modules/{PACKAGE}/package.json")));
            candidates.insert(directory.join(format!("lib/node_modules/{PACKAGE}/package.json")));
        }
    }
    candidates.into_iter().collect()
}
