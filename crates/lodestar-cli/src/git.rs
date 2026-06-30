//! Subcomandos de git de la CLI (`§13.7`), vía `lodestar-workspace`.

use std::path::Path;
use std::process::ExitCode;

use lodestar_workspace::Workspace;

fn open(root: &Path) -> anyhow::Result<Workspace> {
    Workspace::open(root).map_err(|e| anyhow::anyhow!(e.to_string()))
}

/// `lodestar log`: historial con conformidad del HEAD.
pub fn log(root: &Path, limit: usize) -> anyhow::Result<ExitCode> {
    let ws = open(root)?;
    let rows = ws
        .vcs_log(limit)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    if rows.is_empty() {
        println!("(sin commits)");
    }
    for r in rows {
        println!(
            "{}  {}  {}",
            r.short,
            r.author.name,
            r.message.lines().next().unwrap_or("")
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// `lodestar last-conforming`: el `Sha` del último commit conforme.
pub fn last_conforming(root: &Path) -> anyhow::Result<ExitCode> {
    let ws = open(root)?;
    match ws
        .last_conforming()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?
    {
        Some(sha) => {
            println!("{sha}");
            Ok(ExitCode::SUCCESS)
        }
        None => {
            eprintln!("ningún commit conforme en el historial");
            Ok(ExitCode::from(1))
        }
    }
}

/// `lodestar branch`: lista las ramas locales.
pub fn branch(root: &Path) -> anyhow::Result<ExitCode> {
    let ws = open(root)?;
    for b in ws.branches().map_err(|e| anyhow::anyhow!(e.to_string()))? {
        let mark = if b.is_head { "*" } else { " " };
        let track = b
            .upstream
            .map(|u| format!(" [{u} +{}/-{}]", b.ahead, b.behind))
            .unwrap_or_default();
        println!("{mark} {}{track}", b.name);
    }
    Ok(ExitCode::SUCCESS)
}

/// `lodestar pull` / `lodestar push`: red por el binario `git`.
pub fn sync(root: &Path, push: bool) -> anyhow::Result<ExitCode> {
    let ws = open(root)?;
    let outcome =
        if push { ws.push() } else { ws.pull() }.map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!("{}", outcome.summary);
    Ok(if outcome.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}
