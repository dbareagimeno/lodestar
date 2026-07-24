//! Tests de integración de la CLI (E2): exit codes congelados y formatos de salida.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lodestar"))
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lodestar-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &std::path::Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

#[test]
fn check_conforme_exit_0() {
    let dir = temp_dir("conforme");
    write(
        &dir,
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    write(
        &dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo\n",
    );
    let status = bin().arg("--path").arg(&dir).arg("check").status().unwrap();
    assert_eq!(status.code(), Some(0));
}

#[test]
fn check_hard_fail_exit_1() {
    let dir = temp_dir("hardfail");
    // MIGRADO en E16-H05: el hard fail era «sin frontmatter» (`OKF-FM01`), que dejó de ser un
    // error. Hoy lo es un bloque que abre y no cierra (`FM-UNCLOSED`): Lodestar no puede
    // interpretar el documento.
    write(&dir, "malo.md", "---\ntype: Nota\n");
    let status = bin().arg("--path").arg(&dir).arg("check").status().unwrap();
    assert_eq!(status.code(), Some(1));
}

#[test]
fn check_json_es_valido() {
    let dir = temp_dir("json");
    write(
        &dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n",
    );
    let out = bin()
        .arg("--path")
        .arg(&dir)
        .args(["check", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v.get("documents").is_some());
    // MIGRADO en E17-H04: el wire del `Analysis` son los SEIS campos de `§20.7` y ninguno es un
    // contador — `hardFail`/`warnCount` pasaron a métodos derivados de `diagnostics`. El veredicto
    // que consume CI sigue viajando aparte, en `conformant` (lo añade la CLI).
    assert!(v.get("diagnostics").is_some(), "wire camelCase");
    assert!(v.get("outgoing").is_some() && v.get("incoming").is_some());
    assert_eq!(v.get("conformant"), Some(&serde_json::Value::Bool(true)));
    assert!(
        v.get("hardFail").is_none() && v.get("warnCount").is_none() && v.get("perFile").is_none(),
        "los campos retirados no reaparecen en el wire: {v}"
    );
}

#[test]
fn check_sarif_es_valido() {
    let dir = temp_dir("sarif");
    write(&dir, "malo.md", "---\ntype: Nota\n");
    let out = bin()
        .arg("--path")
        .arg(&dir)
        .args(["check", "--sarif"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["version"], "2.1.0");
    assert!(v["runs"][0]["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["ruleId"] == "FM-UNCLOSED"));
}

// --- E9-H02: retirar los subcomandos git de la CLI (conservando `check`) ---

/// E9-H02 `help_sin_subcomandos_git`: **Dado** `lodestar --help`, **Entonces** NO aparecen los 8
/// subcomandos git. E9-H02 retiró la exposición; E15-H01 borró también el crate `vcs`.
#[test]
fn help_sin_subcomandos_git() {
    let out = bin().arg("--help").output().unwrap();
    // `--help` sale con 0 y escribe el listado de comandos en stdout.
    assert_eq!(out.status.code(), Some(0), "`--help` sale 0");
    let help = String::from_utf8(out.stdout).unwrap();
    for sub in [
        "log",
        "last-conforming",
        "branch",
        "switch",
        "merge",
        "pull",
        "push",
        "hooks",
    ] {
        assert!(
            !help.contains(sub),
            "el subcomando git `{sub}` no debe aparecer en `--help`, pero sigue:\n{help}"
        );
    }
}

/// E9-H02 `check_rev_es_uso`: **Dado** `lodestar check --rev HEAD`, **Entonces** exit `2` (uso:
/// flag retirado — D-check). No juzga ningún árbol git (que ya no existe: E15-H01).
#[test]
fn check_rev_es_uso() {
    let dir = temp_dir("check-rev-uso");
    write(
        &dir,
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    let status = bin()
        .arg("--path")
        .arg(&dir)
        .args(["check", "--rev", "HEAD"])
        .status()
        .unwrap();
    assert_eq!(
        status.code(),
        Some(2),
        "`--rev` retirado → error de uso (exit 2), no juzgar el rev"
    );
}

/// E9-H02 `check_working_tree_conforme`: **Dado** `lodestar check` sobre un workspace conforme,
/// **Entonces** exit `0`. La puerta de CI sobre el working tree sigue viva (no-regresión).
#[test]
fn check_working_tree_conforme() {
    let dir = temp_dir("check-wt-conforme");
    write(
        &dir,
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    write(
        &dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo\n",
    );
    let status = bin().arg("--path").arg(&dir).arg("check").status().unwrap();
    assert_eq!(
        status.code(),
        Some(0),
        "la puerta sobre el working tree vive"
    );
}

// --- E14-H01: `lodestar check` como puerta de CI (CLI, sobre el working tree) ---
//
// `lodestar check` (working tree, sin flags git) juzga con el MISMO motor que `knowledge_check`
// scope `workspace`: los diagnósticos de `DocumentSet::analyze()` (`§20.9`). Estos tests fijan el
// contrato de la puerta.
//
// RECOMPUESTOS en E20-H03: antes disparaban el bloqueo con `SCHEMA-REQFIELD` (un `DocType` de
// `.lodestar/schema.yaml` con `requiredFields`). Con el retiro de `core::schema` (modelo universal,
// `§20.10`) ese código muere; el bloqueo se recompone con un código VIVO de `§20.9`,
// `LINK-TARGET-MISSING` (un enlace a un `.md` inexistente es un hard-fail duro), igual que el
// escenario 15 del benchmark hizo con `FM-YAML-INVALID`.

/// Monta un workspace cuyo `a.md` enlaza a un `.md` inexistente ⇒ `LINK-TARGET-MISSING` (Err), un
/// hard-fail que bloquea la puerta de CI. Reutilizado por los tests de surfaceo en `--sarif`/`--json`.
fn workspace_con_enlace_roto(dir: &std::path::Path) {
    write(
        dir,
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    write(
        dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\n[roto](no-existe.md)\n",
    );
}

/// E14-H01 `check_falla` (RECOMPUESTO E20-H03): **Dado** un workspace con un `LINK-TARGET-MISSING`,
/// **Cuando** se corre `lodestar check`, **Entonces** exit `1`. El ÚNICO motivo de bloqueo es ese
/// hard-fail de enlace roto sobre el working tree.
#[test]
fn check_falla() {
    let dir = temp_dir("falla-check");
    workspace_con_enlace_roto(&dir);

    let status = bin().arg("--path").arg(&dir).arg("check").status().unwrap();
    assert_eq!(
        status.code(),
        Some(1),
        "un LINK-TARGET-MISSING sobre el working tree debe bloquear la puerta de CI (exit 1)"
    );
}

/// E14-H01 `check_conforme_json`: **Dado** un workspace conforme, **Cuando** se corre
/// `lodestar check --json`, **Entonces** exit `0` y JSON con `conformant: true`. El documento no
/// tiene enlaces rotos ni ningún otro hard-fail, así que el motor da veredicto conforme.
#[test]
fn check_conforme_json() {
    let dir = temp_dir("conforme-json");
    write(
        &dir,
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    // Documento sin enlaces rotos → conforme.
    write(
        &dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo sin enlaces\n",
    );

    let out = bin()
        .arg("--path")
        .arg(&dir)
        .args(["check", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "workspace conforme → exit 0");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v.get("conformant").and_then(serde_json::Value::as_bool),
        Some(true),
        "el JSON de `check` debe exponer el veredicto `conformant: true` (mismo motor que knowledge_check)"
    );
}

/// E14-H01 `check_caza_edicion_directa` (RECOMPUESTO E20-H03): **Dado** un `.md` editado a mano e
/// inválido, **Cuando** corre CI, **Entonces** la puerta lo caza (exit `1`). Escenario §17 del
/// benchmark «Editar directamente un Markdown inválido → detectado»: se parte de un documento válido
/// y se SOBRESCRIBE a mano por una versión con un enlace roto, simulando una edición directa que
/// deja el workspace no conforme. `check` sobre el working tree debe detectarlo.
#[test]
fn check_caza_edicion_directa() {
    let dir = temp_dir("edicion-directa");
    write(
        &dir,
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    // Estado inicial válido (sin enlaces rotos).
    write(
        &dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo\n",
    );
    // Edición directa del Markdown a mano → queda inválido (añade un enlace a un `.md` inexistente).
    write(
        &dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo editado a mano: [roto](no-existe.md)\n",
    );

    let status = bin().arg("--path").arg(&dir).arg("check").status().unwrap();
    assert_eq!(
        status.code(),
        Some(1),
        "la puerta debe cazar el Markdown editado a mano que deja un enlace roto (exit 1)"
    );
}

/// E14-H01 (reserva del juez) `check_sarif_lista_diagnostico` (RECOMPUESTO E20-H03): la puerta
/// bloquea (exit 1), y el SARIF debe además SURFACEAR el diagnóstico que dispara ese fallo. **Dado**
/// el workspace con `LINK-TARGET-MISSING`, **Cuando** `lodestar check --sarif`, **Entonces** exit 1 Y
/// `runs[0].results` contiene al menos un result con `ruleId == "LINK-TARGET-MISSING"`.
#[test]
fn check_sarif_lista_diagnostico() {
    let dir = temp_dir("sarif-diag");
    workspace_con_enlace_roto(&dir);

    let out = bin()
        .arg("--path")
        .arg(&dir)
        .args(["check", "--sarif"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "LINK-TARGET-MISSING bloquea la puerta (exit 1)"
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let results = v["runs"][0]["results"].as_array().unwrap();
    assert!(
        results.iter().any(|r| r["ruleId"] == "LINK-TARGET-MISSING"),
        "el SARIF debe surfacear el diagnóstico que dispara el exit 1; results = {results:#?}"
    );
}

/// E14-H01 (reserva del juez) `check_json_lista_diagnostico` (RECOMPUESTO E20-H03): análogo en
/// `--json`. **Dado** el workspace con `LINK-TARGET-MISSING`, **Cuando** `lodestar check --json`,
/// **Entonces** exit 1 Y el JSON expone el diagnóstico en `diagnostics` con `code ==
/// "LINK-TARGET-MISSING"`.
#[test]
fn check_json_lista_diagnostico() {
    let dir = temp_dir("json-diag");
    workspace_con_enlace_roto(&dir);

    let out = bin()
        .arg("--path")
        .arg(&dir)
        .args(["check", "--json"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "LINK-TARGET-MISSING bloquea la puerta (exit 1)"
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let per_file = v["diagnostics"].as_object().unwrap();
    let lista = per_file
        .values()
        .filter_map(|checks| checks.as_array())
        .flatten()
        .any(|c| c["code"] == "LINK-TARGET-MISSING");
    assert!(
        lista,
        "el JSON debe listar el diagnóstico LINK-TARGET-MISSING en `diagnostics`; \
         diagnostics = {per_file:#?}"
    );
}

// ---------------------------------------------------------------------------
// E15-H02 / E15-H03 — la CLI queda en `check` + `reindex`
// (`requirements/epica-15-workspace-universal.md`)
// ---------------------------------------------------------------------------

/// Nombres de los subcomandos que anuncia `lodestar --help`, parseados de la sección `Commands:`
/// de clap (primer token de cada línea, hasta la línea en blanco que cierra la sección).
///
/// Se parsea en vez de buscar subcadenas porque `index` es subcadena de `reindex` y de la propia
/// descripción de `reindex` («la cache `.lodestar/index.db`»): un `help.contains("index")` sería a
/// la vez falso-positivo y test vacuo.
fn subcomandos_del_help() -> Vec<String> {
    let out = bin().arg("--help").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "`--help` sale 0");
    let help = String::from_utf8(out.stdout).unwrap();
    let mut subs = Vec::new();
    let mut dentro = false;
    for linea in help.lines() {
        if linea.trim_end() == "Commands:" {
            dentro = true;
            continue;
        }
        if dentro {
            if linea.trim().is_empty() {
                break;
            }
            // Las líneas de continuación de una descripción larga van más indentadas; el nombre
            // del subcomando es el primer token de una línea con indentación de dos espacios.
            if let Some(nombre) = linea.split_whitespace().next() {
                if linea.starts_with("  ") && !linea.starts_with("      ") {
                    subs.push(nombre.to_string());
                }
            }
        }
    }
    assert!(
        !subs.is_empty(),
        "no se pudo parsear la sección `Commands:` del help:\n{help}"
    );
    subs
}

/// `help_sin_generadores` (E15-H02) — **Dado** `lodestar --help`, **Cuando** se imprime,
/// **Entonces** no aparecen los subcomandos `index` ni `tags`: sin generadores no hay catálogo.
///
/// Fase ROJA: hoy ambos siguen en el enum de clap (`main.rs`), así que el listado los incluye.
#[test]
fn help_sin_generadores() {
    let subs = subcomandos_del_help();
    for generador in ["index", "tags"] {
        assert!(
            !subs.iter().any(|s| s == generador),
            "el subcomando generador `{generador}` no debe existir; el help ofrece: {subs:?}"
        );
    }
}

/// `index_es_uso` (E15-H02) — **Dado** `lodestar index`, **Cuando** se ejecuta, **Entonces** exit
/// code `2` (uso: subcomando retirado).
///
/// Se ejecuta con el cwd en un directorio temporal para que, mientras el subcomando siga vivo, la
/// generación no escriba un `index.md` dentro del repo.
///
/// Fase ROJA: hoy `index` genera el índice del directorio y sale `0`.
#[test]
fn index_es_uso() {
    let dir = temp_dir("index-es-uso");
    write(
        &dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n",
    );
    let status = bin().current_dir(&dir).arg("index").status().unwrap();
    assert_eq!(
        status.code(),
        Some(2),
        "`index` retirado → error de uso (exit 2), no generar el índice"
    );
    assert!(
        !dir.join("index.md").exists(),
        "un subcomando retirado no debe haber escrito nada en disco"
    );
}

/// `help_solo_check_y_reindex` (E15-H03) — **Dado** `lodestar --help`, **Cuando** se imprime,
/// **Entonces** los únicos subcomandos son `check` y `reindex` (más el `help` que añade clap).
///
/// Fase ROJA: hoy el help ofrece además `init`, `index`, `tags`, `export` e `import`.
#[test]
fn help_solo_check_y_reindex() {
    let mut subs = subcomandos_del_help();
    subs.sort();
    subs.dedup();
    let esperados = vec![
        "check".to_string(),
        "help".to_string(),
        "reindex".to_string(),
    ];
    assert_eq!(
        subs, esperados,
        "la CLI debe quedar en `check` + `reindex` (más `help` de clap); ofrece: {subs:?}"
    );
}

/// `init_es_uso` (E15-H03) — **Dado** `lodestar init`, **Cuando** se ejecuta, **Entonces** exit
/// code `2`: no hay ceremonia de creación, cualquier directorio vale desde el principio.
///
/// Fase ROJA: hoy `init` monta el scaffold (index raíz + `.gitignore` + repo) y sale `0`.
#[test]
fn init_es_uso() {
    let dir = temp_dir("init-es-uso");
    let status = bin().current_dir(&dir).arg("init").status().unwrap();
    assert_eq!(
        status.code(),
        Some(2),
        "`init` retirado → error de uso (exit 2), no crear scaffold"
    );
    assert!(
        !dir.join("index.md").exists(),
        "un subcomando retirado no debe haber creado el scaffold"
    );
}

// ---------------------------------------------------------------------------
// E15-H06 — La raíz del workspace es el `cwd`
// (`requirements/epica-15-workspace-universal.md`, `ARCHITECTURE.md §20.5`).
//
// `resolve_root` (`crates/lodestar-cli/src/main.rs:46`) deja de SUBIR por los ancestros buscando
// `index.md`/`.lodestar`: usa `--path`, y si no hay, el cwd tal cual.
// ---------------------------------------------------------------------------

/// `cli_no_asciende` (E15-H06) — **Dado** un cwd que es subdirectorio de un proyecto con `index.md`
/// en un ancestro, **Cuando** se corre `lodestar check`, **Entonces** juzga el cwd, no el ancestro.
///
/// El escenario está montado para que el veredicto sea **distinto** en cada caso, de modo que el
/// test no pueda pasar por casualidad:
///   · el ANCESTRO contiene `malo.md` (frontmatter sin cerrar ⇒ `FM-UNCLOSED`, hard fail) ⇒
///     juzgarlo da exit 1;
///   · el SUBDIRECTORIO contiene solo un `a.md` conforme ⇒ juzgarlo da exit 0.
/// Además se comprueba el inventario juzgado (`documents` del `--json`, campo ya existente en el
/// wire): desde el subdirectorio debe ser exactamente `["a.md"]`, no `["malo.md","sub/a.md"]`.
///
/// Fase ROJA: hoy `resolve_root` sube hasta el ancestro (tiene `index.md`), juzga el proyecto
/// entero y sale con 1.
#[test]
fn cli_no_asciende() {
    let proyecto = temp_dir("no-asciende");
    write(
        &proyecto,
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    // Hard fail que vive SOLO en el ancestro. MIGRADO en E16-H05: «sin frontmatter» dejó de
    // serlo, así que la premisa del escenario —que el ancestro dé exit 1— se sostiene ahora con
    // un bloque sin cerrar.
    write(&proyecto, "malo.md", "---\ntype: Nota\n");
    // El subdirectorio, juzgado por sí mismo, es conforme.
    let sub = proyecto.join("sub");
    write(
        &sub,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo\n",
    );
    // Precondición: el subdirectorio no tiene marcas de lodestar. Si las tuviera, el
    // `resolve_root` de hoy pararía ahí y el test sería vacuo.
    assert!(
        !sub.join("index.md").exists() && !sub.join(".lodestar").exists(),
        "el escenario exige un subdirectorio sin marcas de lodestar"
    );

    let out = bin()
        .current_dir(&sub)
        .args(["check", "--json"])
        .output()
        .unwrap();

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let documents: Vec<&str> = v["documents"]
        .as_array()
        .expect("`check --json` expone `documents`")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    assert_eq!(
        documents,
        vec!["a.md"],
        "`check` debe juzgar el cwd (solo `a.md`), no ascender al ancestro"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "el subdirectorio es conforme por sí mismo → exit 0 (el hard fail es del ancestro)"
    );
}
