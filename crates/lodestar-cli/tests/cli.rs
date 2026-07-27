//! Tests de integración de la CLI (E2): exit codes congelados y formatos de salida.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lodestar"))
}

/// Directorio temporal aislado para un test, **con limpieza automática**: el [`tempfile::TempDir`]
/// borra el árbol al dropearse, al final del test.
///
/// UNIFICADO en E23-H08 con el arnés del resto del repo (`lodestar-mcp`, `lodestar-store`,
/// `lodestar-workspace` ya usaban `tempfile`). Antes el nombre se derivaba del PID
/// (`std::env::temp_dir().join("lodestar-cli-<name>-<pid>")`) y **nunca** se limpiaba: los
/// directorios se acumulaban en `/tmp` ejecución tras ejecución, y dos tests del mismo binario que
/// pidieran el mismo `name` compartían directorio (todos los tests corren en el mismo proceso, así
/// que el PID no los distingue).
///
/// El `name` sobrevive como **prefijo** del directorio —sigue siendo útil para identificar restos si
/// un test se cuelga—, pero ya no es su identidad: `tempfile` añade el sufijo aleatorio.
fn temp_dir(name: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("lodestar-cli-{name}-"))
        .tempdir()
        .unwrap()
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
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    write(
        dir.path(),
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo\n",
    );
    let status = bin()
        .arg("--path")
        .arg(dir.path())
        .arg("check")
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(0));
}

#[test]
fn check_hard_fail_exit_1() {
    let dir = temp_dir("hardfail");
    // MIGRADO en E16-H05: el hard fail era «sin frontmatter» (`OKF-FM01`), que dejó de ser un
    // error. Hoy lo es un bloque que abre y no cierra (`FM-UNCLOSED`): Lodestar no puede
    // interpretar el documento.
    write(dir.path(), "malo.md", "---\ntype: Nota\n");
    let status = bin()
        .arg("--path")
        .arg(dir.path())
        .arg("check")
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(1));
}

#[test]
fn check_json_es_valido() {
    let dir = temp_dir("json");
    write(
        dir.path(),
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n",
    );
    let out = bin()
        .arg("--path")
        .arg(dir.path())
        .args(["check", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v.get("documents").is_some());
    // MIGRADO en E17-H04: el wire del `Analysis` son los SEIS campos de `§20.7` y ninguno es un
    // contador — `hardFail`/`warnCount` pasaron a métodos derivados de `diagnostics`. El veredicto
    // que consume CI sigue viajando aparte, en `valid` (lo añade la CLI).
    assert!(v.get("diagnostics").is_some(), "wire camelCase");
    assert!(v.get("outgoing").is_some() && v.get("incoming").is_some());
    assert_eq!(v.get("valid"), Some(&serde_json::Value::Bool(true)));
    assert!(
        v.get("hardFail").is_none() && v.get("warnCount").is_none() && v.get("perFile").is_none(),
        "los campos retirados no reaparecen en el wire: {v}"
    );
}

#[test]
fn check_sarif_es_valido() {
    let dir = temp_dir("sarif");
    write(dir.path(), "malo.md", "---\ntype: Nota\n");
    let out = bin()
        .arg("--path")
        .arg(dir.path())
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
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    let status = bin()
        .arg("--path")
        .arg(dir.path())
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
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    write(
        dir.path(),
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo\n",
    );
    let status = bin()
        .arg("--path")
        .arg(dir.path())
        .arg("check")
        .status()
        .unwrap();
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
    workspace_con_enlace_roto(dir.path());

    let status = bin()
        .arg("--path")
        .arg(dir.path())
        .arg("check")
        .status()
        .unwrap();
    assert_eq!(
        status.code(),
        Some(1),
        "un LINK-TARGET-MISSING sobre el working tree debe bloquear la puerta de CI (exit 1)"
    );
}

/// E14-H01 `check_conforme_json`: **Dado** un workspace conforme, **Cuando** se corre
/// `lodestar check --json`, **Entonces** exit `0` y JSON con `valid: true`. El documento no
/// tiene enlaces rotos ni ningún otro hard-fail, así que el motor da veredicto conforme.
#[test]
fn check_conforme_json() {
    let dir = temp_dir("conforme-json");
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    // Documento sin enlaces rotos → conforme.
    write(
        dir.path(),
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo sin enlaces\n",
    );

    let out = bin()
        .arg("--path")
        .arg(dir.path())
        .args(["check", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "workspace conforme → exit 0");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v.get("valid").and_then(serde_json::Value::as_bool),
        Some(true),
        "el JSON de `check` debe exponer el veredicto `valid: true` (mismo motor que knowledge_check)"
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
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    // Estado inicial válido (sin enlaces rotos).
    write(
        dir.path(),
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo\n",
    );
    // Edición directa del Markdown a mano → queda inválido (añade un enlace a un `.md` inexistente).
    write(
        dir.path(),
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo editado a mano: [roto](no-existe.md)\n",
    );

    let status = bin()
        .arg("--path")
        .arg(dir.path())
        .arg("check")
        .status()
        .unwrap();
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
    workspace_con_enlace_roto(dir.path());

    let out = bin()
        .arg("--path")
        .arg(dir.path())
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
    workspace_con_enlace_roto(dir.path());

    let out = bin()
        .arg("--path")
        .arg(dir.path())
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
        dir.path(),
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n",
    );
    let status = bin().current_dir(dir.path()).arg("index").status().unwrap();
    assert_eq!(
        status.code(),
        Some(2),
        "`index` retirado → error de uso (exit 2), no generar el índice"
    );
    assert!(
        !dir.path().join("index.md").exists(),
        "un subcomando retirado no debe haber escrito nada en disco"
    );
}

/// `help_solo_check_y_reindex` (E15-H03; ampliado en E22-H01) — **Dado** `lodestar --help`,
/// **Cuando** se imprime, **Entonces** los subcomandos son `check`, `reindex` y `migrate-from-okf`
/// (más el `help` que añade clap). Ninguno de OKF (`init`/`index`/`tags`/`export`/`import`).
///
/// `migrate-from-okf` (E22-H01) es un diagnóstico de cortesía para repos OKF legados, no un
/// generador ni ceremonia de creación — no reintroduce la superficie retirada en E15.
#[test]
fn help_solo_check_y_reindex() {
    let mut subs = subcomandos_del_help();
    subs.sort();
    subs.dedup();
    let esperados = vec![
        "check".to_string(),
        "help".to_string(),
        "migrate-from-okf".to_string(),
        "reindex".to_string(),
    ];
    assert_eq!(
        subs, esperados,
        "la CLI debe quedar en `check` + `reindex` + `migrate-from-okf` (más `help` de clap); ofrece: {subs:?}"
    );
}

/// `init_es_uso` (E15-H03) — **Dado** `lodestar init`, **Cuando** se ejecuta, **Entonces** exit
/// code `2`: no hay ceremonia de creación, cualquier directorio vale desde el principio.
///
/// Fase ROJA: hoy `init` monta el scaffold (index raíz + `.gitignore` + repo) y sale `0`.
#[test]
fn init_es_uso() {
    let dir = temp_dir("init-es-uso");
    let status = bin().current_dir(dir.path()).arg("init").status().unwrap();
    assert_eq!(
        status.code(),
        Some(2),
        "`init` retirado → error de uso (exit 2), no crear scaffold"
    );
    assert!(
        !dir.path().join("index.md").exists(),
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
        proyecto.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    // Hard fail que vive SOLO en el ancestro. MIGRADO en E16-H05: «sin frontmatter» dejó de
    // serlo, así que la premisa del escenario —que el ancestro dé exit 1— se sostiene ahora con
    // un bloque sin cerrar.
    write(proyecto.path(), "malo.md", "---\ntype: Nota\n");
    // El subdirectorio, juzgado por sí mismo, es conforme.
    let sub = proyecto.path().join("sub");
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

// ---------------------------------------------------------------------------
// E22-H01 — `migrate-from-okf --dry-run`: diagnóstico de convenciones OKF legadas
// (`requirements/epica-22-migracion-publicacion.md`, `REFACTOR_PHASE_2 §Fase 14`,
//  `ARCHITECTURE.md §20.13`).
//
// El comando es un DIAGNÓSTICO de cortesía: recorre el workspace, LISTA las convenciones OKF que
// §Fase 14 enumera y afirma explícitamente que NO modificó ningún fichero. Nunca es una puerta —
// mientras pueda leer el workspace sale 0 (no exit 1 por «detectó OKF»). La salida informativa va
// a STDOUT; los errores, a stderr.
//
// CRITERIOS DE DETECCIÓN FIJADOS POR EL AUTOR DE TESTS (los generadores se borraron en E15, así que
// no queda rastro del generador: se detecta por convención heurística de cortesía, no tiene que ser
// perfecta):
//   · index.md raíz        → un `index.md` en la raíz del workspace.
//   · índice anidado       → un `index.md` que NO está en la raíz (`<dir>/index.md`).
//   · metadata okf_version → un documento cuyo frontmatter lleva la clave `okf_version`.
//   · índice de tags       → un `.md` bajo el directorio `tags/` (lo que producía `gen_tag_indexes`).
//
// SIN `--dry-run` (decisión del autor): `migrate-from-okf` sin el flag es ERROR DE USO (exit 2), NO
// un alias del dry-run. En v0.3 solo existe la forma diagnóstica; exigir `--dry-run` explícito deja
// la palabra libre para una futura forma «aplicadora» sin invocarla por accidente. Se fija en
// `migrate_sin_dry_run_es_uso`.

/// Monta bajo `dir` un workspace con las convenciones OKF que `§Fase 14` enumera: `index.md` raíz
/// (con `okf_version`), un índice anidado (`seccion/index.md`), un índice de tags generado
/// (`tags/algo.md`) y un documento normal cualquiera.
fn workspace_okf(dir: &std::path::Path) {
    // `index.md` raíz + metadata `okf_version`.
    write(
        dir,
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    // Índice anidado: un `index.md` que NO está en la raíz.
    write(
        dir,
        "seccion/index.md",
        "---\ntype: Index\ntitle: Sección\ndescription: Índice anidado\n---\n\n# Sección\n",
    );
    // Índice de tags generado: un `.md` bajo `tags/`.
    write(
        dir,
        "tags/algo.md",
        "---\ntype: Index\ntitle: \"Tag: algo\"\n---\n\n# Tag: algo\n",
    );
    // Documento normal (para que el workspace no sea solo índices).
    write(
        dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo\n",
    );
}

/// Snapshot determinista del árbol de ficheros bajo `dir`: lista ordenada de `(ruta relativa,
/// bytes)`. Captura contenido Y existencia, así detecta tanto una modificación de contenido como un
/// fichero creado o borrado (p. ej. un `.gitignore` o un `.lodestar/` que el comando escribiera sin
/// querer).
///
/// Solo recoge FICHEROS: un directorio vacío (el caso de `.lodestar/runtime/{plans,receipts,
/// staging}`) no deja rastro aquí, así que quien quiera excluir también scaffolds vacíos tiene que
/// aseverar además su no-existencia (lo hace `check_no_ensucia_el_working_tree`).
fn snapshot_arbol(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    fn recorrer(
        base: &std::path::Path,
        actual: &std::path::Path,
        acc: &mut Vec<(String, Vec<u8>)>,
    ) {
        let mut entradas: Vec<std::path::PathBuf> = std::fs::read_dir(actual)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entradas.sort();
        for p in entradas {
            if p.is_dir() {
                recorrer(base, &p, acc);
            } else {
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                acc.push((rel, std::fs::read(&p).unwrap()));
            }
        }
    }
    let mut acc = Vec::new();
    recorrer(dir, dir, &mut acc);
    acc.sort();
    acc
}

/// E22-H01 `dry_run_detecta`: **Dado** un workspace con `index.md` raíz + `okf_version` + un índice
/// de tags (más un índice anidado), **Cuando** se corre `migrate-from-okf --dry-run`, **Entonces**
/// los detecta y los lista en su salida, y declara que no modificó nada.
///
/// Fase ROJA: hoy `migrate-from-okf` no existe como subcomando ⇒ clap responde «unrecognized
/// subcommand» con exit 2 y stdout vacío, así que fallan tanto el exit 0 como las aserciones de
/// listado.
#[test]
fn dry_run_detecta() {
    let dir = temp_dir("mfo-detecta");
    workspace_okf(dir.path());

    let out = bin()
        .arg("--path")
        .arg(dir.path())
        .args(["migrate-from-okf", "--dry-run"])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(0),
        "el diagnóstico sale 0 mientras pueda leer el workspace (no es puerta); stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lower = stdout.to_lowercase();

    // index.md raíz: una línea que menciona `index.md` SIN separador de path (el anidado lleva `/`,
    // así que esta comprobación no la satisface el `seccion/index.md`).
    assert!(
        stdout
            .lines()
            .any(|l| l.contains("index.md") && !l.contains('/')),
        "el informe debe listar el `index.md` raíz; stdout=\n{stdout}"
    );
    // Índice anidado.
    assert!(
        stdout.contains("seccion/index.md"),
        "el informe debe listar el índice anidado `seccion/index.md`; stdout=\n{stdout}"
    );
    // Metadata `okf_version`: clave de frontmatter, token estable en cualquier idioma de salida.
    assert!(
        stdout.contains("okf_version"),
        "el informe debe señalar la metadata `okf_version`; stdout=\n{stdout}"
    );
    // Índice de tags generado.
    assert!(
        stdout.contains("tags/algo.md"),
        "el informe debe listar el índice de tags `tags/algo.md`; stdout=\n{stdout}"
    );
    // Declara explícitamente que no modificó nada. Ancla `modif`, común a «modified»/«modificó»/
    // «modificar» — deliberadamente laxa en el texto: la garantía dura de «cero cambios» la aporta
    // `dry_run_no_modifica` comparando el árbol byte a byte.
    assert!(
        lower.contains("modif"),
        "el informe debe declarar que no modificó ningún fichero; stdout=\n{stdout}"
    );
}

/// E22-H01 `dry_run_no_modifica`: **Dado** ese workspace OKF, **Cuando** se corre
/// `migrate-from-okf --dry-run`, **Entonces** ningún fichero cambia (snapshot del árbol idéntico
/// antes/después: mismo conjunto de rutas y mismos bytes).
///
/// Fase ROJA: `migrate-from-okf` no existe ⇒ exit 2. La aserción de exit 0 falla; el árbol queda
/// intacto pero eso NO basta (ver la guarda anti-vacuo abajo).
#[test]
fn dry_run_no_modifica() {
    let dir = temp_dir("mfo-no-modifica");
    workspace_okf(dir.path());

    let antes = snapshot_arbol(dir.path());

    let out = bin()
        .arg("--path")
        .arg(dir.path())
        .args(["migrate-from-okf", "--dry-run"])
        .output()
        .unwrap();

    // GUARDA ANTI-VACUO: sin esta aserción, un subcomando inexistente (exit 2, no escribe nada)
    // dejaría el árbol intacto y el test pasaría SIN implementación. Exigir el exit 0 obliga a que
    // el diagnóstico EXISTA y CORRA antes de que la comparación del árbol signifique algo.
    assert_eq!(
        out.status.code(),
        Some(0),
        "el diagnóstico debe ejecutarse (exit 0) para que comparar el árbol sea significativo; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let despues = snapshot_arbol(dir.path());
    assert_eq!(
        antes, despues,
        "`migrate-from-okf --dry-run` no debe modificar, crear ni borrar ningún fichero del árbol"
    );
}

/// E22-H01 `dry_run_workspace_limpio`: **Dado** un workspace SIN convenciones OKF (la estructura
/// arbitraria de `§Resultado esperado`: sin `index.md`, sin `okf_version`, sin `tags/`), **Cuando**
/// se corre `migrate-from-okf --dry-run`, **Entonces** reporta que no hay nada que migrar y sale 0.
///
/// El discriminante robusto frente a `dry_run_detecta` es `okf_version`: sobre un workspace limpio
/// el informe NO debe reportarla (mientras que sobre el OKF sí). Una sola implementación no puede
/// satisfacer ambos salvo que detecte de verdad la convención.
///
/// Fase ROJA: `migrate-from-okf` no existe ⇒ exit 2 (falla el exit 0) y stdout vacío (falla el
/// «reporta algo»).
#[test]
fn dry_run_workspace_limpio() {
    let dir = temp_dir("mfo-limpio");
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();

    let out = bin()
        .arg("--path")
        .arg(dir.path())
        .args(["migrate-from-okf", "--dry-run"])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(0),
        "un workspace sin OKF es diagnosticable y sale 0; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        !stdout.is_empty(),
        "el diagnóstico debe reportar algo aunque no haya nada que migrar"
    );
    // Sin convenciones OKF, el informe no debe reportar `okf_version` como detectada (el ancla más
    // específica: es una clave de frontmatter que solo aparecería si se hubiera encontrado).
    assert!(
        !stdout.contains("okf_version"),
        "sobre un workspace limpio el informe no debe reportar `okf_version` detectada; stdout=\n{stdout}"
    );
}

/// E22-H01 `migrate_sin_dry_run_es_uso` (decisión del autor sobre «sin `--dry-run`»):
/// **Dado** `migrate-from-okf` **sin** `--dry-run`, **Cuando** se ejecuta, **Entonces** es error de
/// uso (exit 2) y el mensaje guía hacia `--dry-run` — no es un alias del dry-run.
///
/// Fase ROJA (no vacuo): hoy el rojo es «unrecognized subcommand», cuyo mensaje **no** contiene
/// `dry-run`; el exit 2 accidental del subcomando inexistente NO basta para pasar, porque además se
/// exige que el error mencione `dry-run` — que solo aparecerá cuando el subcomando exista con el
/// flag requerido.
#[test]
fn migrate_sin_dry_run_es_uso() {
    let dir = temp_dir("mfo-sin-flag");
    workspace_okf(dir.path());

    let out = bin()
        .arg("--path")
        .arg(dir.path())
        .arg("migrate-from-okf")
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(2),
        "`migrate-from-okf` sin `--dry-run` es error de uso (exit 2), no un alias del dry-run"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("dry-run"),
        "el error de uso debe guiar hacia `--dry-run`; stderr=\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// E23-H01 — Una sola verdad de validación
// (`requirements/epica-23-cierre-migracion.md`, `CLAUDE.md` invariante #3, `§20.9`)
//
// SÍNTOMA REPRODUCIDO (no deducido): con `.lodestar/config.yaml` →
// `validation: {danglingDocumentLinks: ignore, malformedFrontmatter: ignore}` sobre un workspace con
// un enlace roto y un YAML ilegible, `lodestar check` imprime `NO CONFORME` y sale **1** mientras que
// `knowledge_check` responde `valid: true` con `summary.errors: 0`. Dos veredictos
// contradictorios sobre el MISMO workspace, con el mismo motor debajo.
//
// CAUSA: `App::full_analysis()` va por `document_set().analyze()` a secas —sin la política de
// severidad de `ValidationSection::effective_severity` y sin los diagnósticos de descubrimiento—,
// mientras que `App::knowledge_check` sí pasa por `document_set_with_discovery()` + la política.
//
// UBICACIÓN de los tests: aquí, en la fachada CLI, y no solo en `lodestar-app`, porque el criterio de
// aceptación compara **dos superficies**: el exit code del binario real (`lodestar check`) contra el
// veredicto de `knowledge_check`. `lodestar-app` es dependencia de `lodestar-cli`, así que el mismo
// test puede ejecutar el binario y llamar a la capa de servicios en proceso. (En
// `crates/lodestar-app/tests/validacion.rs` vive el test hermano que fija que la corrección va en
// `full_analysis`, el punto compartido, y no en `commands.rs`.)
// ---------------------------------------------------------------------------

use lodestar_app::{App, CheckScope};
use lodestar_core::types::Severity;

/// Semilla del síntoma de E23-H01: un workspace con **dos** defectos, uno por cada familia de
/// `§20.9` que el síntoma configura:
/// - `notas/rota.md` enlaza a un `.md` inexistente → `LINK-TARGET-MISSING` con `related[0]`
///   markdown ⇒ familia `danglingDocumentLinks` (severidad intrínseca `err`).
/// - `notas/ilegible.md` abre un bloque de frontmatter con YAML no interpretable →
///   `FM-YAML-INVALID` ⇒ familia `malformedFrontmatter` (severidad intrínseca `err`).
///
/// Los dos defectos viven en documentos **distintos** a propósito: así el veredicto no depende de
/// que un mismo fichero acumule diagnósticos de dos familias.
fn workspace_dos_defectos(dir: &std::path::Path) {
    write(
        dir,
        "notas/rota.md",
        "# Rota\n\nEnlace a un documento inexistente: [falta](inexistente.md).\n",
    );
    write(
        dir,
        "notas/ilegible.md",
        "---\ntitulo: [sin cerrar\notra: \"comilla\n---\n\n# Ilegible\n\nCuerpo.\n",
    );
}

/// Corre `lodestar check --json` sobre `dir` y devuelve `(exit code, JSON de la salida)`.
fn check_json(dir: &std::path::Path) -> (Option<i32>, serde_json::Value) {
    let out = bin()
        .arg("--path")
        .arg(dir)
        .args(["check", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "`check --json` debe emitir JSON válido por stdout ({e}); stdout=\n{}\nstderr=\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    });
    (out.status.code(), v)
}

/// Llama a `knowledge_check(scope: workspace)` **en proceso** sobre el mismo directorio (umbral
/// `Info`, sin fixes, límite holgado) y devuelve el reporte.
fn knowledge_check_workspace(dir: &std::path::Path) -> lodestar_app::CheckReport {
    let app = App::open(dir).expect("el workspace de prueba debe abrir");
    app.knowledge_check(
        &CheckScope::Workspace,
        Some(Severity::Info),
        false,
        Some(1000),
        None,
    )
    .expect("knowledge_check(workspace) debe responder")
}

/// Resumen legible de los diagnósticos de un `CheckReport`, para los mensajes de fallo.
fn resumen_reporte(report: &lodestar_app::CheckReport) -> String {
    report
        .diagnostics
        .iter()
        .map(|c| format!("{}/{:?}", c.code.as_str(), c.level))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Los códigos de todos los diagnósticos que viajan en el `diagnostics` del `check --json`
/// (`{path: [Check]}`), en orden de aparición.
fn codigos_del_json(v: &serde_json::Value) -> Vec<String> {
    v["diagnostics"]
        .as_object()
        .map(|m| {
            m.values()
                .filter_map(serde_json::Value::as_array)
                .flatten()
                .filter_map(|c| c["code"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// **E23-H01** · Criterio `check_y_knowledge_check_coinciden_con_ignore`:
/// **Dado** un workspace con un enlace roto y un YAML ilegible, con
/// `validation: {danglingDocumentLinks: ignore, malformedFrontmatter: ignore}`, **Cuando** se corre
/// `lodestar check` y se llama a `knowledge_check(scope: workspace)`, **Entonces** ambos dicen
/// **conforme** (exit 0 / `valid: true`).
///
/// ROJO hoy (reproducido con los binarios, no deducido): `knowledge_check` ya responde
/// `valid: true` con `errors: 0` —aplica `ValidationSection::effective_severity`, que suprime
/// las dos familias puestas a `ignore`—, pero `lodestar check` sale **1** con `valid: false`,
/// porque `App::full_analysis()` va por `document_set().analyze()` a secas y ve las severidades
/// intrínsecas. La aserción que falla es la del exit code / `valid` de la CLI.
#[test]
fn check_y_knowledge_check_coinciden_con_ignore() {
    let dir = temp_dir("h01-coinciden-ignore");
    workspace_dos_defectos(dir.path());
    write(
        dir.path(),
        ".lodestar/config.yaml",
        "validation:\n  danglingDocumentLinks: ignore\n  malformedFrontmatter: ignore\n",
    );

    // (1) El veredicto del motor de servicios: las dos familias están suprimidas ⇒ conforme.
    let report = knowledge_check_workspace(dir.path());
    assert_eq!(
        report.summary.errors,
        0,
        "precondición del síntoma: con las dos familias a `ignore`, knowledge_check no debe contar \
         errores. Diagnósticos: [{}]",
        resumen_reporte(&report)
    );
    assert!(
        report.valid,
        "precondición del síntoma: knowledge_check debe declarar el workspace conforme con las dos \
         familias a `ignore`. Diagnósticos: [{}]",
        resumen_reporte(&report)
    );

    // (2) El veredicto de la puerta de CI sobre el MISMO workspace debe ser el MISMO.
    let (code, json) = check_json(dir.path());
    assert_eq!(
        code,
        Some(0),
        "`lodestar check` debe coincidir con knowledge_check (conforme ⇒ exit 0) sobre el mismo \
         workspace y la misma config: la sección `validation` no se está aplicando en el camino de \
         la CLI (`full_analysis`). Códigos vistos en el JSON: {:?}",
        codigos_del_json(&json)
    );
    assert_eq!(
        json.get("valid").and_then(serde_json::Value::as_bool),
        Some(true),
        "el `valid` del `check --json` debe coincidir con el de knowledge_check (true): {json}"
    );
    // Y los diagnósticos suprimidos tampoco deben viajar en la salida: una familia a `ignore` no se
    // reporta (`§20.9`), ni siquiera como informativa.
    let codigos = codigos_del_json(&json);
    assert!(
        !codigos
            .iter()
            .any(|c| c == "LINK-TARGET-MISSING" || c == "FM-YAML-INVALID"),
        "una familia a `ignore` se SUPRIME: `check --json` no debe listar sus diagnósticos; \
         códigos = {codigos:?}"
    );
}

/// **E23-H01** · Criterio `check_y_knowledge_check_coinciden_con_error` (**control anti-vacuo**):
/// **Dado** el mismo workspace con `validation: {danglingDocumentLinks: error}`, **Cuando** se corre
/// lo mismo, **Entonces** ambos dicen **NO conforme** (exit 1 / `valid: false`).
///
/// Su función es impedir que «coinciden» se satisfaga devolviendo siempre `conforme`: una
/// implementación que hiciera `valid = true` a secas pasaría el test del `ignore` y **rompería
/// este**. Es una GUARDA, verde hoy (el gate absoluto actual ya bloquea con cualquier `Err`): en
/// aislamiento no puede ir roja mientras `full_analysis` ignore la config, porque la severidad
/// configurada aquí coincide con la intrínseca. Su valor es de regresión sobre la implementación
/// futura, igual que `rechaza_errores_nuevos` en `lodestar-app/tests/validacion.rs`.
#[test]
fn check_y_knowledge_check_coinciden_con_error() {
    let dir = temp_dir("h01-coinciden-error");
    workspace_dos_defectos(dir.path());
    write(
        dir.path(),
        ".lodestar/config.yaml",
        "validation:\n  danglingDocumentLinks: error\n",
    );

    let report = knowledge_check_workspace(dir.path());
    assert!(
        report.summary.errors >= 1,
        "con `danglingDocumentLinks: error` el enlace roto debe contar como error en \
         knowledge_check. Diagnósticos: [{}]",
        resumen_reporte(&report)
    );
    assert!(
        !report.valid,
        "con `danglingDocumentLinks: error` knowledge_check NO debe declarar conforme el \
         workspace. Diagnósticos: [{}]",
        resumen_reporte(&report)
    );

    let (code, json) = check_json(dir.path());
    assert_eq!(
        code,
        Some(1),
        "`lodestar check` debe coincidir con knowledge_check (no conforme ⇒ exit 1): {json}"
    );
    assert_eq!(
        json.get("valid").and_then(serde_json::Value::as_bool),
        Some(false),
        "el `valid` del `check --json` debe coincidir con el de knowledge_check (false): {json}"
    );
    // Y el diagnóstico que dispara el bloqueo se surfacea (no solo el veredicto).
    let codigos = codigos_del_json(&json);
    assert!(
        codigos.iter().any(|c| c == "LINK-TARGET-MISSING"),
        "el `check --json` debe surfacear el LINK-TARGET-MISSING que dispara el exit 1; \
         códigos = {codigos:?}"
    );
}

/// **E23-H01** · Criterio `check_ve_diagnosticos_de_descubrimiento`:
/// **Dado** un workspace con un `.md` no UTF-8, **Cuando** se corre `lodestar check --json`,
/// **Entonces** el diagnóstico de descubrimiento (`DOC-NOT-UTF8`) aparece en la salida.
///
/// `notas/binario.md` **no** entra en el inventario (no se pudo interpretar), así que su diagnóstico
/// no lo produce el recorrido por `Analysis::documents`: lo produce el descubrimiento
/// (`Workspace::document_set_with_discovery`), que hoy `full_analysis` no consulta. `knowledge_check`
/// sí lo ve (E20-H04), y de ahí la contradicción que cierra esta historia.
///
/// El exit sigue siendo **0**: `DOC-NOT-UTF8` es `Warn` y su código no pertenece a ninguna de las 5
/// familias configurables, así que no bloquea la puerta — y ese es justamente el veredicto que da
/// `knowledge_check` sobre el mismo workspace (`errors == 0`). Surfacear el diagnóstico no puede
/// convertirse en un bloqueo nuevo de CI.
///
/// ROJO hoy: `check --json` emite `diagnostics: {"notas/uno.md": []}` — ni rastro del `DOC-NOT-UTF8`.
#[test]
fn check_ve_diagnosticos_de_descubrimiento() {
    let dir = temp_dir("h01-descubrimiento");
    write(dir.path(), "notas/uno.md", "# Uno\n\nCuerpo sin enlaces.\n");
    // `.md` no UTF-8: 0xF0 abre una secuencia de 4 bytes y 0x28 no es continuación válida.
    std::fs::write(
        dir.path().join("notas/binario.md"),
        [0xF0, 0x28, 0x8C, 0xBC],
    )
    .unwrap();

    // Precondición no vacua: `knowledge_check` SÍ ve el diagnóstico de descubrimiento (E20-H04). Si
    // esto fallara, el fixture no estaría produciendo el DOC-NOT-UTF8 y el test sería vacuo.
    let report = knowledge_check_workspace(dir.path());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|c| c.code.as_str() == "DOC-NOT-UTF8"),
        "precondición: knowledge_check debe ver el DOC-NOT-UTF8 del `.md` no UTF-8. \
         Diagnósticos: [{}]",
        resumen_reporte(&report)
    );

    let (code, json) = check_json(dir.path());
    let codigos = codigos_del_json(&json);
    assert!(
        codigos.iter().any(|c| c == "DOC-NOT-UTF8"),
        "`lodestar check --json` debe surfacear el diagnóstico de descubrimiento DOC-NOT-UTF8 (hoy \
         `full_analysis` descarta los diagnósticos de descubrimiento); códigos = {codigos:?}, \
         json = {json}"
    );
    assert!(
        serde_json::to_string(&json["diagnostics"])
            .unwrap()
            .contains("binario.md"),
        "el DOC-NOT-UTF8 debe señalar al fichero culpable `notas/binario.md`: {}",
        json["diagnostics"]
    );
    assert_eq!(
        code,
        Some(0),
        "un `.md` no UTF-8 es un AVISO (`DOC-NOT-UTF8`), no un hard-fail: surfacearlo no debe \
         convertir la puerta en roja — knowledge_check declara el mismo workspace conforme \
         (errors == {}). json = {json}",
        report.summary.errors
    );
}

// ---------------------------------------------------------------------------
// E23-H08 — Cobertura de `lodestar reindex`
// (`requirements/epica-23-cierre-migracion.md`, invariante #1 y #5 de `CLAUDE.md`)
//
// HUECO QUE CIERRAN: `reindex` es 1 de los 3 subcomandos del producto y hasta E23-H08 **ningún test
// lo invocaba** — solo aparecía en la aserción de `--help` (`help_solo_check_y_reindex`), que
// comprueba que se anuncia, no que funcione. El hueco estaba declarado como «Pendiente» en el ledger
// desde E15.
//
// NO son fase roja: la implementación existe (`commands::reindex`) y los tests salen VERDES. Su
// valor es de cobertura y regresión, así que la exigencia se traslada a la NO-VACUIDAD: cada uno
// asevera algo que un `reindex` roto (o degradado a no-op) rompería.
//
// CÓMO SE OBSERVA LA CACHE: con `lodestar_store::Store::open`, que aplica el DDL pero **no** indexa.
// Es deliberado: `Workspace::enable_cache()` —el camino que usa el propio subcomando— llama a
// `rebuild()`, así que responder por ahí daría el inventario correcto AUNQUE `reindex` no hubiera
// escrito una sola fila; sería un observable vacuo. Lo que devuelve `documentos_en_cache` es,
// literalmente, lo que el subcomando dejó persistido en `.lodestar/index.db`.
//
// POR QUÉ NO SE USA `check --json` COMO OBSERVABLE (lo que sugería el criterio de la historia):
// `check` no lee la cache en ningún momento —recorre el disco por `document_set()`—, de modo que su
// salida es idéntica antes y después de cualquier `reindex`, incluso de uno que no hiciera nada.
// Comparar dos `check` sería trivialmente cierto.
// ---------------------------------------------------------------------------

/// El bloque que `lodestar` gestiona dentro del `.gitignore` del proyecto
/// (`crates/lodestar-workspace/src/gitignore.rs`), en su forma canónica.
///
/// Los tests de `reindex` parten de un proyecto que **ya lo tiene** (el caso normal de cualquier
/// repo que haya usado lodestar antes): `reindex` activa la cache y `enable_cache()` es uno de los
/// cuatro chokepoints de escritura que E23-H12 **conserva** —ahí nace `index.db`, así que ahí toca
/// ignorarlo—, de modo que sin el bloque ya presente la primera pasada lo añadiría y comparar el
/// árbol canónico antes/después mediría ese ajuste legítimo en vez de lo que se quiere medir. Con el
/// bloque presente, `ensure_gitignore` sale por su rama idempotente y la comparación pasa a
/// aseverar algo más fuerte: reconstruir la cache **no toca ni un byte** del `.gitignore` del
/// usuario (ni lo reordena, ni le duplica líneas, ni le normaliza los finales de línea).
const BLOQUE_GITIGNORE_GESTIONADO: &str =
    "# lodestar: cache y runtime desechables (no versionar)\n.lodestar/index.db\n.lodestar/runtime/\n";

/// Monta el workspace de los tests de `reindex` y devuelve los paths de sus documentos, ordenados.
///
/// Incluye un fichero que **no** es documento (`src/main.rs`): así la lista esperada no es «todo lo
/// que hay en el árbol» y una cache que indexara de más también fallaría.
fn workspace_para_reindex(dir: &std::path::Path) -> Vec<String> {
    write(
        dir,
        ".gitignore",
        &format!("target/\n\n{BLOQUE_GITIGNORE_GESTIONADO}"),
    );
    write(
        dir,
        "guia.md",
        "---\nestado: vigente\n---\n\n# Guía\n\nVer [alfa](notas/alfa.md).\n",
    );
    write(dir, "notas/alfa.md", "# Alfa\n\nCuerpo de la nota alfa.\n");
    write(
        dir,
        "notas/beta.md",
        "---\ntags: [uno, dos]\n---\n\n# Beta\n",
    );
    write(dir, "src/main.rs", "fn main() {}\n");
    vec![
        "guia.md".to_string(),
        "notas/alfa.md".to_string(),
        "notas/beta.md".to_string(),
    ]
}

/// La ruta de la cache derivada de un workspace.
fn ruta_cache(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".lodestar/index.db")
}

/// Los documentos indexados en `.lodestar/index.db`, leídos **sin reconstruir la cache**
/// (`Store::open` aplica el DDL pero no indexa — ver la cabecera del bloque).
fn documentos_en_cache(root: &std::path::Path) -> Vec<String> {
    let store = lodestar_store::Store::open(root).expect("la cache debe poder abrirse");
    let mut docs: Vec<String> = store
        .documents()
        .expect("la cache debe poder consultarse")
        .iter()
        .map(|p| p.as_str().to_string())
        .collect();
    docs.sort();
    docs
}

/// Corre `lodestar --path <root> reindex` y devuelve `(exit code, stdout)`.
fn corre_reindex(root: &std::path::Path) -> (Option<i32>, String) {
    let out = bin()
        .arg("--path")
        .arg(root)
        .arg("reindex")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.code() != Some(101),
        "`reindex` no debe panicar; stderr=\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code(), stdout)
}

/// Snapshot del árbol **canónico** (todo menos lo derivado): los `.md` y demás ficheros del
/// proyecto —**el `.gitignore` incluido**—, sin `.lodestar/` (cache + runtime, derivados y
/// desechables).
///
/// El `.gitignore` estuvo excluido a propósito mientras `Workspace::open` lo reescribía al abrir;
/// **E23-H12** cerró ese defecto (`check_no_ensucia_el_working_tree`), así que vuelve al snapshot:
/// es un fichero del usuario como cualquier otro y `reindex` no puede churnearlo.
fn snapshot_canonico(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    snapshot_arbol(dir)
        .into_iter()
        .filter(|(rel, _)| !rel.starts_with(".lodestar"))
        .collect()
}

/// **E23-H08** · Criterio `reindex_crea_cache`: **Dado** un workspace sin cache, **Cuando** se corre
/// `lodestar reindex`, **Entonces** existe `.lodestar/index.db` y el exit es `0`.
///
/// NO-VACUIDAD: no basta con que el fichero exista —`Store::open` crea un `index.db` vacío con solo
/// aplicar el DDL—, así que se exige además que quede **poblado** con exactamente los 3 documentos
/// del workspace (y no con `src/main.rs`, que no es documento). Un `reindex` que abriera la cache y
/// no indexara pasaría la aserción de existencia y fallaría esta.
#[test]
fn reindex_crea_cache() {
    let dir = temp_dir("reindex-crea");
    let esperados = workspace_para_reindex(dir.path());
    let db = ruta_cache(dir.path());
    assert!(
        !db.exists(),
        "precondición: el workspace arranca SIN cache (si ya existiera, el test sería vacuo)"
    );

    let (code, stdout) = corre_reindex(dir.path());
    assert_eq!(code, Some(0), "`reindex` sale 0; stdout=\n{stdout}");
    assert!(
        db.exists(),
        "`reindex` debe crear la cache en `.lodestar/index.db`"
    );
    assert!(
        std::fs::metadata(&db).unwrap().len() > 0,
        "la cache creada no puede ser un fichero de 0 bytes"
    );
    assert_eq!(
        documentos_en_cache(dir.path()),
        esperados,
        "la cache debe quedar POBLADA con los documentos del workspace (y solo con ellos: \
         `src/main.rs` no es documento)"
    );
}

/// **E23-H08** · Criterio `reindex_es_idempotente`: **Dado** una cache recién creada, **Cuando** se
/// corre `reindex` otra vez, **Entonces** exit `0` y el resultado observable es idéntico.
///
/// OBSERVABLE ELEGIDO (y por qué no es trivialmente cierto):
///  1. el **contenido indexado** de la cache leído sin reconstruirla (`documentos_en_cache`) — no
///     un `check --json`, que jamás toca la cache y por tanto sería idéntico incluso si `reindex`
///     fuera un no-op;
///  2. el **árbol canónico byte a byte** antes y después de las dos pasadas: la cache es derivada y
///     desechable (invariante #1), así que reconstruirla no puede tocar ni un `.md` ni un fichero
///     del proyecto. Esta es la aserción que caza el fallo caro de verdad;
///  3. exit `0` y el **mismo stdout** en las dos pasadas (la segunda no puede degradarse a un error
///     por «la cache ya existe», que es el modo de fallo clásico de un reindex no idempotente).
///
/// GUARDA ANTI-VACUA (fase 3): tras comparar las dos pasadas se **añade** un documento y se vuelve a
/// reindexar, exigiendo que la cache pase a tener 4. Demuestra que el observable comparado es
/// SENSIBLE al estado del workspace: si `documentos_en_cache` devolviera siempre lo mismo (porque
/// `reindex` no reconstruye, o porque se estuviera leyendo un rebuild propio del test en vez de lo
/// persistido), la igualdad de las fases 1-2 no significaría nada y esta fase fallaría.
#[test]
fn reindex_es_idempotente() {
    let dir = temp_dir("reindex-idempotente");
    let esperados = workspace_para_reindex(dir.path());
    let canonico_inicial = snapshot_canonico(dir.path());

    let (code1, stdout1) = corre_reindex(dir.path());
    assert_eq!(code1, Some(0), "primera pasada: exit 0; stdout=\n{stdout1}");
    let docs1 = documentos_en_cache(dir.path());
    assert_eq!(docs1, esperados, "primera pasada: cache poblada");

    let (code2, stdout2) = corre_reindex(dir.path());
    assert_eq!(
        code2,
        Some(0),
        "segunda pasada sobre una cache YA existente: exit 0; stdout=\n{stdout2}"
    );
    let docs2 = documentos_en_cache(dir.path());

    assert_eq!(
        docs2, docs1,
        "correr `reindex` dos veces debe dejar la cache con el mismo inventario (ni duplicados ni \
         pérdidas)"
    );
    assert_eq!(
        stdout2, stdout1,
        "la segunda pasada debe reportar lo mismo que la primera"
    );
    assert_eq!(
        snapshot_canonico(dir.path()),
        canonico_inicial,
        "la cache es derivada y desechable: reconstruirla (dos veces) no puede modificar, crear ni \
         borrar ningún fichero canónico del proyecto"
    );

    // --- guarda anti-vacua: el observable es sensible al estado del workspace ------------------
    write(
        dir.path(),
        "notas/gamma.md",
        "# Gamma\n\nDocumento nuevo.\n",
    );
    let (code3, _) = corre_reindex(dir.path());
    assert_eq!(code3, Some(0), "tercera pasada: exit 0");
    let docs3 = documentos_en_cache(dir.path());
    assert_eq!(
        docs3.len(),
        esperados.len() + 1,
        "`reindex` reconstruye desde disco: un documento nuevo debe aparecer en la cache. Sin esto, \
         la igualdad de las dos primeras pasadas sería vacua. Cache = {docs3:?}"
    );
    assert!(
        docs3.contains(&"notas/gamma.md".to_string()),
        "la cache debe contener el documento añadido: {docs3:?}"
    );
}

/// **E23-H08** · Criterio `reindex_sobre_cache_corrupta`: **Dado** un `index.db` con bytes basura,
/// **Cuando** se corre `reindex`, **Entonces** exit `0` y la cache queda usable.
///
/// La cache es DESECHABLE (invariante #1): un fichero corrupto —un `git checkout` a medias, un disco
/// lleno, un kill durante una escritura— no puede dejar a `lodestar` sin poder abrirla nunca más. El
/// precedente a nivel de crate es `cache_corrupta_se_recrea_sola`
/// (`crates/lodestar-store/tests/store.rs`), que prueba `Store::open`; esto lo prueba **por la
/// fachada**, que es donde el usuario lo sufre.
///
/// NO-VACUIDAD: «usable» no se comprueba volviendo a abrir la cache y ya (`Store::open` borra y
/// recrea el fichero corrupto por su cuenta, así que eso pasaría aunque `reindex` hubiera fallado):
/// se exige que la cache quede **poblada** con los 3 documentos y que el fichero ya no sea la basura
/// que se escribió (cabecera SQLite real).
#[test]
fn reindex_sobre_cache_corrupta() {
    let dir = temp_dir("reindex-corrupta");
    let esperados = workspace_para_reindex(dir.path());

    // Cache corrupta: bytes que no son una base SQLite (incluye NUL y bytes no UTF-8).
    let db = ruta_cache(dir.path());
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let basura: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    std::fs::write(&db, &basura).unwrap();
    assert!(
        !std::fs::read(&db)
            .unwrap()
            .starts_with(b"SQLite format 3\0"),
        "precondición: el `index.db` de partida NO es una base SQLite"
    );

    let (code, stdout) = corre_reindex(dir.path());
    assert_eq!(
        code,
        Some(0),
        "`reindex` sobre una cache corrupta debe recrearla y salir 0, no propagar el error de \
         SQLite; stdout=\n{stdout}"
    );

    let bytes = std::fs::read(&db).unwrap();
    assert!(
        bytes.starts_with(b"SQLite format 3\0"),
        "tras el reindex, `index.db` debe ser una base SQLite real (los bytes basura se descartan)"
    );
    assert_eq!(
        documentos_en_cache(dir.path()),
        esperados,
        "la cache recreada debe quedar usable Y poblada con los documentos del workspace"
    );
}

// ---------------------------------------------------------------------------
// E23-H12 — Higiene de efectos secundarios: **abrir un workspace no modifica el proyecto**
// (`requirements/epica-23-cierre-migracion.md §E23-H12`).
//
// Fase ROJA: hoy `Workspace::open` (`crates/lodestar-workspace/src/lib.rs:83-84`) llama a
// `gitignore::ensure_gitignore(root)` y `runtime::ensure_runtime_scaffold(root)` **antes de leer
// nada**, así que la mera puerta de CI reescribe el `.gitignore` del usuario y le crea
// `.lodestar/runtime/{plans,receipts,staging}`. Los dos efectos pasan a ser perezosos (ocurren
// cuando se va a escribir de verdad); el `.gitignore` sobrevive en los cuatro chokepoints de
// escritura (`enable_cache`, `acquire_lock`, `persist_plan`, `try_append_audit`) y el scaffold
// desaparece sin sustituto.
// ---------------------------------------------------------------------------

/// **E23-H12** · Criterio `check_no_ensucia_el_working_tree`: **Dado** un proyecto con un
/// `.gitignore` propio, **Cuando** se corre `lodestar check`, **Entonces** el `.gitignore` queda
/// **byte a byte** igual.
///
/// POR QUÉ SE COMPARAN BYTES (y no «sigue conteniendo mis reglas»): `ensure_gitignore` es
/// idempotente byte a byte **a partir de la segunda vez** —sale antes si las dos entradas ya
/// están—, pero la PRIMERA reescritura reconstruye el fichero línea a línea: normaliza los CRLF a
/// `\n` y poda las líneas en blanco finales. Un `.gitignore` escrito en Windows conserva todas sus
/// reglas y aun así vuelve del `check` con otros bytes; para `git` eso es un fichero modificado, y
/// en CI, un working tree sucio. Un test que solo mirase el contenido lógico NO vería el defecto.
///
/// Se asevera además el árbol ENTERO (mismos ficheros, mismos bytes) y la **no existencia** de
/// `.lodestar/`: `snapshot_arbol` solo recoge ficheros, así que los tres subdirectorios vacíos del
/// scaffold de runtime solo se cazan preguntando por el directorio.
///
/// NO-VACUIDAD: se exige exit `0` y que el `--json` liste los **2 documentos** del proyecto. Sin
/// eso, un `check` que no llegara a abrir el workspace (o un binario roto) dejaría el árbol intacto
/// y pasaría el test sin haber hecho nada.
#[test]
fn check_no_ensucia_el_working_tree() {
    let dir = temp_dir("check-higiene");

    // `.gitignore` propio del usuario, con los dos detalles que la reescritura normaliza: finales
    // de línea CRLF y una línea en blanco al final.
    let gitignore_original: &[u8] = b"target/\r\n*.log\r\n\r\n";
    std::fs::write(dir.path().join(".gitignore"), gitignore_original).unwrap();
    write(
        dir.path(),
        "guia.md",
        "# Guía\n\nVer [alfa](notas/alfa.md).\n",
    );
    write(dir.path(), "notas/alfa.md", "# Alfa\n\nCuerpo.\n");

    let antes = snapshot_arbol(dir.path());

    let out = bin()
        .arg("--path")
        .arg(dir.path())
        .args(["check", "--json"])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(0),
        "el proyecto es válido: la puerta sale 0; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("`--json` es JSON");
    assert_eq!(
        json["documents"].as_array().map(Vec::len),
        Some(2),
        "guarda de no vacuidad: el `check` tiene que haber leído los 2 documentos para que comparar \
         el árbol signifique algo; json={json}"
    );

    let gitignore_tras_check = std::fs::read(dir.path().join(".gitignore")).unwrap();
    assert_eq!(
        gitignore_tras_check,
        gitignore_original,
        "`lodestar check` es una LECTURA: el `.gitignore` del proyecto debe quedar byte a byte igual \
         (ni CRLF normalizados, ni líneas en blanco podadas, ni bloque añadido). Era {:?} y quedó {:?}",
        String::from_utf8_lossy(gitignore_original),
        String::from_utf8_lossy(&gitignore_tras_check)
    );
    assert!(
        !dir.path().join(".lodestar").exists(),
        "abrir el workspace para leerlo no puede crear `.lodestar/` (ni el scaffold de runtime): en \
         un proyecto ajeno es una escritura no solicitada, y en CI deja el working tree sucio"
    );
    assert_eq!(
        snapshot_arbol(dir.path()),
        antes,
        "`lodestar check` no debe modificar, crear ni borrar NINGÚN fichero del proyecto"
    );
}
