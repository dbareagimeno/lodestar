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

/// `lodestar pull` / `lodestar push`: red por el binario `git`. Un fallo de red/upstream es
/// runtime (exit 3), NO el `1` congelado para «bundle no conforme» — un CI que trate el 1 como
/// veredicto de conformidad se confundiría.
pub fn sync(root: &Path, push: bool) -> anyhow::Result<ExitCode> {
    let ws = open(root)?;
    let outcome =
        if push { ws.push() } else { ws.pull() }.map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!("{}", outcome.summary);
    Ok(if outcome.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(3)
    })
}

/// `lodestar check --staged`: juzga el árbol staged (exit 0/1).
pub fn check_staged(root: &Path, json: bool, sarif: bool) -> anyhow::Result<ExitCode> {
    let ws = open(root)?;
    let analysis = ws
        .analyze_staged()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let blocked = load_gate(root)?.gate_blocked(&analysis);
    crate::commands::render_analysis(&analysis, json, sarif, blocked)
}

/// Config con error explícito: un `lodestar.toml` roto no puede relajar la puerta en silencio.
fn load_gate(root: &Path) -> anyhow::Result<lodestar_workspace::Config> {
    lodestar_workspace::Config::load(root).map_err(|e| anyhow::anyhow!(e))
}

/// `lodestar check --rev <REV>`: juzga el árbol de una revisión (exit 0/1).
pub fn check_rev(root: &Path, rev: &str, json: bool, sarif: bool) -> anyhow::Result<ExitCode> {
    let ws = open(root)?;
    let analysis = ws
        .analyze_rev(rev)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let blocked = load_gate(root)?.gate_blocked(&analysis);
    crate::commands::render_analysis(&analysis, json, sarif, blocked)
}

/// `lodestar switch <name>`: cambia de rama por el único escritor (checkpoint previo).
pub fn switch(root: &Path, name: &str, create: bool) -> anyhow::Result<ExitCode> {
    let ws = open(root)?;
    if create {
        ws.create_branch(name, None)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    }
    let report = ws
        .switch(name)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!(
        "en la rama {name} ({} escritos, {} eliminados)",
        report.written, report.removed
    );
    Ok(ExitCode::SUCCESS)
}

/// `lodestar merge <name>`: merge local por el único escritor.
pub fn merge(root: &Path, name: &str) -> anyhow::Result<ExitCode> {
    let ws = open(root)?;
    let report = ws.merge(name).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    if report.up_to_date {
        println!("ya está al día");
    } else if !report.conflicted.is_empty() {
        eprintln!("merge con conflictos en:");
        for p in &report.conflicted {
            eprintln!("  {p}");
        }
        eprintln!("resuelve los marcadores y commitea para completar el merge");
        return Ok(ExitCode::from(1));
    } else if report.fast_forward {
        println!("merge fast-forward completado");
    } else {
        println!("merge completado ({} ficheros)", report.report.written);
    }
    Ok(ExitCode::SUCCESS)
}

/// `lodestar hooks install`: instala el hook `pre-commit`.
pub fn hooks(root: &Path) -> anyhow::Result<ExitCode> {
    let ws = open(root)?;
    let path = ws
        .install_hooks()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!("hook instalado en {}", path.display());
    Ok(ExitCode::SUCCESS)
}

/// `lodestar reindex`: reconstruye la cache `.lodestar/index.db` desde disco.
pub fn reindex(root: &Path) -> anyhow::Result<ExitCode> {
    let mut ws = open(root)?;
    ws.enable_cache()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!(
        "cache reconstruida en {}",
        root.join(".lodestar/index.db").display()
    );
    Ok(ExitCode::SUCCESS)
}
