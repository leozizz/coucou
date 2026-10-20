// Antigravity CLI runner for Mochi Chat (Task Mode).
// Dispatches tasks directly to the local agent CLI (agy), streaming output
// into the chat view while Coucou's hook relay reflects live tools on the island.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::chat::{ChatReply, ModelInfo};

#[allow(dead_code)]
pub const AGENT_CLI_PROVIDER: &str = "antigravity";

#[allow(dead_code)]
/// Checks if `agy` is available on PATH or in common user locations.
pub fn is_available() -> bool {
    find_agy_binary().is_some()
}

pub fn find_agy_binary() -> Option<std::path::PathBuf> {
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let exe = dir.join("agy.exe");
            if exe.is_file() {
                return Some(exe);
            }
            let bin = dir.join("agy");
            if bin.is_file() {
                return Some(bin);
            }
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let candidate = std::path::Path::new(&local).join("agy").join("bin").join("agy.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn models() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            id: "agy-auto".into(),
            label: "Antigravity (Auto Mode)".into(),
        },
        ModelInfo {
            id: "agy-plan".into(),
            label: "Antigravity (Plan Mode)".into(),
        },
        ModelInfo {
            id: "agy-accept-edits".into(),
            label: "Antigravity (Accept Edits)".into(),
        },
    ]
}

/// Runs a single turn prompt with Antigravity CLI, streaming output via `chat-delta`.
pub async fn send(
    app: &AppHandle,
    cwd: Option<&str>,
    model: &str,
    query: &str,
) -> Result<ChatReply, String> {
    let binary = find_agy_binary().ok_or_else(|| {
        "Antigravity CLI (agy) was not found on PATH. Run 'agy install' or verify your setup.".to_string()
    })?;

    let mode = if model.contains("plan") {
        "plan"
    } else if model.contains("accept-edits") {
        "accept-edits"
    } else {
        "auto"
    };

    let mut cmd = Command::new(binary);
    cmd.arg("-p")
        .arg(query)
        .arg("--mode")
        .arg(mode);

    if let Some(dir) = cwd {
        if !dir.is_empty() && std::path::Path::new(dir).is_dir() {
            cmd.current_dir(dir);
        }
    }

    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW = 0x08000000
        cmd.creation_flags(0x0800_0000);
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(100);

    let task = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn agy: {e}"))?;
        let stdout = child.stdout.take().ok_or("Failed to capture agy stdout")?;
        let reader = BufReader::new(stdout);
        let mut full_output = String::new();

        for line in reader.lines() {
            let line = line.map_err(|e| format!("Error reading agy output: {e}"))?;
            if !full_output.is_empty() {
                full_output.push('\n');
            }
            full_output.push_str(&line);
            let _ = tx.blocking_send(full_output.clone());
        }

        let status = child.wait().map_err(|e| format!("Error waiting on agy: {e}"))?;

        if !status.success() && full_output.trim().is_empty() {
            return Err(format!("Antigravity CLI exited with status {status}"));
        }

        Ok(full_output)
    });

    while let Some(delta) = rx.recv().await {
        let _ = app.emit("chat-delta", &delta);
    }

    let full_output = task.await.map_err(|e| format!("Task join error: {e}"))??;

    Ok(ChatReply {
        text: if full_output.trim().is_empty() {
            "Task completed.".to_string()
        } else {
            full_output
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn antigravity_models_offer_auto_plan_and_accept_edits() {
        let list = models();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].id, "agy-auto");
        assert_eq!(list[1].id, "agy-plan");
        assert_eq!(list[2].id, "agy-accept-edits");
    }
}
