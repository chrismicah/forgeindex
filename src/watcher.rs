use anyhow::{Context, Result};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tracing::{debug, info};

/// Start watching a directory for file changes. Calls `on_change` for each
/// changed file path after debouncing.
pub fn watch<F>(root: &Path, debounce_ms: u64, on_change: F) -> Result<()>
where
    F: Fn(&Path) + Send + 'static,
{
    let (tx, rx) = mpsc::channel();

    let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
        if let Ok(event) = res {
            match event.kind {
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                    for path in event.paths {
                        let _ = tx.send(path);
                    }
                }
                _ => {}
            }
        }
    })
    .context("Failed to create file watcher")?;

    watcher
        .watch(root, RecursiveMode::Recursive)
        .context("Failed to start watching")?;

    info!("Watching {} for changes", root.display());

    let debounce = Duration::from_millis(debounce_ms);
    let mut pending: std::collections::HashMap<std::path::PathBuf, Instant> =
        std::collections::HashMap::new();

    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(path) => {
                pending.insert(path, Instant::now());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                debug!("Watcher channel disconnected");
                break;
            }
        }

        // Process debounced events
        let now = Instant::now();
        let ready: Vec<std::path::PathBuf> = pending
            .iter()
            .filter(|(_, &time)| now.duration_since(time) >= debounce)
            .map(|(path, _)| path.clone())
            .collect();

        for path in ready {
            pending.remove(&path);
            debug!("File changed: {}", path.display());
            on_change(&path);
        }
    }

    Ok(())
}

/// Resolve the actual .git directory, following gitdir references in worktrees/submodules.
fn resolve_git_dir(repo_path: &Path) -> Result<std::path::PathBuf> {
    let git_path = repo_path.join(".git");
    if !git_path.exists() {
        anyhow::bail!("Not a git repository: {}", repo_path.display());
    }

    if git_path.is_dir() {
        return Ok(git_path);
    }

    // .git is a file (worktree or submodule) — read the gitdir pointer
    let content = std::fs::read_to_string(&git_path).context("Failed to read .git file")?;
    let gitdir = content.strip_prefix("gitdir: ").unwrap_or(&content).trim();
    let resolved = if Path::new(gitdir).is_absolute() {
        std::path::PathBuf::from(gitdir)
    } else {
        repo_path.join(gitdir)
    };

    if resolved.is_dir() {
        Ok(resolved)
    } else {
        anyhow::bail!(
            "gitdir reference points to non-existent directory: {}",
            resolved.display()
        )
    }
}

/// Install git hooks for auto-reindexing.
pub fn install_hooks(repo_path: &Path, hook_types: &[String]) -> Result<()> {
    let git_dir = resolve_git_dir(repo_path)?;

    let hooks_dir = git_dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir)?;

    let hook_script = r#"#!/bin/sh
# ForgeIndex auto-reindex hook
# Automatically re-indexes the codebase after git operations
if command -v forgeindex > /dev/null 2>&1; then
    forgeindex reindex > /dev/null 2>&1 &
fi
"#;

    let marker = "# ForgeIndex auto-reindex hook";

    for hook_name in hook_types {
        let hook_path = hooks_dir.join(hook_name);

        if hook_path.exists() {
            let existing = std::fs::read_to_string(&hook_path)?;
            if existing.contains(marker) {
                info!("Hook {} already installed, skipping", hook_name);
                continue;
            }
            // Append to existing hook
            let mut content = existing;
            content.push_str("\n\n");
            content.push_str(hook_script);
            std::fs::write(&hook_path, content)?;
        } else {
            std::fs::write(&hook_path, hook_script)?;
        }

        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755))?;
        }

        info!("Installed git hook: {}", hook_name);
    }

    Ok(())
}

/// Remove ForgeIndex git hooks.
pub fn uninstall_hooks(repo_path: &Path, hook_types: &[String]) -> Result<()> {
    let git_dir = match resolve_git_dir(repo_path) {
        Ok(d) => d,
        Err(_) => return Ok(()),
    };
    let hooks_dir = git_dir.join("hooks");
    if !hooks_dir.exists() {
        return Ok(());
    }

    let marker_start = "# ForgeIndex auto-reindex hook";

    for hook_name in hook_types {
        let hook_path = hooks_dir.join(hook_name);
        if !hook_path.exists() {
            continue;
        }

        let content = std::fs::read_to_string(&hook_path)?;
        if !content.contains(marker_start) {
            continue;
        }

        // Remove the ForgeIndex block
        let lines: Vec<&str> = content.lines().collect();
        let mut new_lines = Vec::new();
        let mut skip = false;

        for line in &lines {
            if line.contains(marker_start) {
                skip = true;
                continue;
            }
            if skip {
                // Skip until we find a line that doesn't look like part of our hook
                if line.starts_with('#')
                    || line.starts_with("if ")
                    || line.starts_with("fi")
                    || line.contains("forgeindex")
                    || line.trim().is_empty()
                {
                    continue;
                }
                skip = false;
            }
            if !skip {
                new_lines.push(*line);
            }
        }

        let new_content = new_lines.join("\n");
        if new_content.trim().is_empty() || new_content.trim() == "#!/bin/sh" {
            // Remove the hook file entirely if only our content was in it
            std::fs::remove_file(&hook_path)?;
        } else {
            std::fs::write(&hook_path, new_content)?;
        }

        info!("Removed git hook: {}", hook_name);
    }

    Ok(())
}
