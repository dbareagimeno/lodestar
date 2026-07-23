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
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
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
    write(&dir, "malo.md", "# sin frontmatter\n");
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
    assert!(v.get("concepts").is_some());
    assert!(v.get("hardFail").is_some(), "wire camelCase");
}

#[test]
fn check_sarif_es_valido() {
    let dir = temp_dir("sarif");
    write(&dir, "malo.md", "# sin frontmatter\n");
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
        .any(|r| r["ruleId"] == "OKF-FM01"));
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
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
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

/// E9-H02 `check_working_tree_conforme`: **Dado** `lodestar check` sobre un bundle conforme,
/// **Entonces** exit `0`. La puerta de CI sobre el working tree sigue viva (no-regresión).
#[test]
fn check_working_tree_conforme() {
    let dir = temp_dir("check-wt-conforme");
    write(
        &dir,
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
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

// --- E14-H01: `knowledge_check` como puerta de CI (CLI, sobre el working tree) ---
//
// `lodestar check` (working tree, sin flags git) debe juzgar con el MISMO motor que
// `knowledge_check` scope `workspace`: OKF + SCHEMA-* + REL-* + refs externas. Hoy el comando
// solo corre `Bundle::analyze()` (los 15 checks OKF) y NO carga `.lodestar/schema.yaml`, así que
// una violación schema-driven pasa desapercibida. Estos tres tests fijan el contrato de la puerta.

/// Escribe el schema de bundle en `.lodestar/schema.yaml` (loader `WorkspaceSchema::load`, wire
/// camelCase). Aquí: el `DocType` «Nota» exige el campo obligatorio `owner` (un extra), cuya
/// ausencia dispara `SCHEMA-REQFIELD` (E10-H07, `core::schema::validate_schema`).
fn write_schema_nota_requiere_owner(dir: &std::path::Path) {
    write(
        dir,
        ".lodestar/schema.yaml",
        "types:\n  Nota:\n    requiredFields:\n      - owner\n",
    );
}

/// E14-H01 `check_falla_schema`: **Dado** un bundle con un `SCHEMA-REQFIELD`, **Cuando** se corre
/// `lodestar check`, **Entonces** exit `1`. El bundle es OKF-conforme (frontmatter válido, sin
/// hard-fail OKF): el ÚNICO motivo de bloqueo es la conformidad schema-driven, así que si `check`
/// no la corriese sobre el working tree saldría `0` (rojo actual).
#[test]
fn check_falla_schema() {
    let dir = temp_dir("falla-schema");
    write(
        &dir,
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    // Concepto de tipo Nota, OKF-conforme, pero SIN el campo `owner` que el schema exige.
    write(
        &dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo\n",
    );
    write_schema_nota_requiere_owner(&dir);

    let status = bin().arg("--path").arg(&dir).arg("check").status().unwrap();
    assert_eq!(
        status.code(),
        Some(1),
        "un SCHEMA-REQFIELD sobre el working tree debe bloquear la puerta de CI (exit 1)"
    );
}

/// E14-H01 `check_conforme_json`: **Dado** un bundle conforme con schema, **Cuando** se corre
/// `lodestar check --json`, **Entonces** exit `0` y JSON con `conformant: true`. El concepto
/// satisface el `requiredFields` del schema (tiene `owner`), demostrando que el motor schema-driven
/// SÍ se ejecuta y da veredicto conforme. Hoy el JSON serializa un `Analysis` sin campo
/// `conformant` → rojo por aserción.
#[test]
fn check_conforme_json() {
    let dir = temp_dir("conforme-json");
    write(
        &dir,
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    // Concepto de tipo Nota que SÍ trae `owner` → satisface el schema.
    write(
        &dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\nowner: alguien\n---\n\n# H\n\ncuerpo\n",
    );
    write_schema_nota_requiere_owner(&dir);

    let out = bin()
        .arg("--path")
        .arg(&dir)
        .args(["check", "--json"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "bundle conforme con schema → exit 0"
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v.get("conformant").and_then(serde_json::Value::as_bool),
        Some(true),
        "el JSON de `check` debe exponer el veredicto `conformant: true` (mismo motor que knowledge_check)"
    );
}

/// E14-H01 `check_caza_edicion_directa`: **Dado** un `.md` editado a mano e inválido (schema-driven),
/// **Cuando** corre CI, **Entonces** la puerta lo caza (exit `1`). Escenario §17 del benchmark
/// «Editar directamente un Markdown inválido → detectado»: se parte de un concepto válido (con
/// `owner`) y se SOBRESCRIBE a mano por una versión sin `owner`, simulando una edición directa del
/// fichero que rompe el schema. `check` sobre el working tree debe detectarlo.
#[test]
fn check_caza_edicion_directa() {
    let dir = temp_dir("edicion-directa");
    write(
        &dir,
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    write_schema_nota_requiere_owner(&dir);
    // Estado inicial válido (satisface el schema).
    write(
        &dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\nowner: alguien\n---\n\n# H\n\ncuerpo\n",
    );
    // Edición directa del Markdown a mano → queda inválido (borra el campo obligatorio `owner`).
    write(
        &dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo editado a mano\n",
    );

    let status = bin().arg("--path").arg(&dir).arg("check").status().unwrap();
    assert_eq!(
        status.code(),
        Some(1),
        "la puerta debe cazar el Markdown editado a mano que rompe el schema (exit 1)"
    );
}

/// Monta el bundle con `SCHEMA-REQFIELD` de `check_falla_schema` (concepto Nota SIN `owner`,
/// schema que lo exige) en `dir`. Reutilizado por los tests de surfaceo en `--sarif`/`--json`.
fn bundle_con_schema_reqfield(dir: &std::path::Path) {
    write(
        dir,
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    write(
        dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo\n",
    );
    write_schema_nota_requiere_owner(dir);
}

/// E14-H01 (reserva del juez) `check_sarif_lista_schema`: la puerta bloquea (exit 1) con el motor
/// schema-driven, pero el SARIF debe además SURFACEAR el diagnóstico que dispara ese fallo, no solo
/// los checks OKF. **Dado** el bundle con `SCHEMA-REQFIELD`, **Cuando** `lodestar check --sarif`,
/// **Entonces** exit 1 Y `runs[0].results` contiene al menos un result con
/// `ruleId == "SCHEMA-REQFIELD"` (misma forma SARIF que `check_sarif_es_valido`, que usa `ruleId`).
/// Hoy `to_sarif(&Analysis)` solo itera `per_file`, y `analyze()` no coloca los SCHEMA-* ahí → el
/// SARIF sale con cero results de schema (inútil para anotar el fallo en CI) → rojo por aserción.
#[test]
fn check_sarif_lista_schema() {
    let dir = temp_dir("sarif-schema");
    bundle_con_schema_reqfield(&dir);

    let out = bin()
        .arg("--path")
        .arg(&dir)
        .args(["check", "--sarif"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "SCHEMA-REQFIELD bloquea la puerta (exit 1)"
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let results = v["runs"][0]["results"].as_array().unwrap();
    assert!(
        results.iter().any(|r| r["ruleId"] == "SCHEMA-REQFIELD"),
        "el SARIF debe surfacear el diagnóstico de schema que dispara el exit 1, no solo los OKF; \
         results = {results:#?}"
    );
}

/// E14-H01 (reserva del juez) `check_json_lista_schema`: análogo en `--json`. **Dado** el bundle con
/// `SCHEMA-REQFIELD`, **Cuando** `lodestar check --json`, **Entonces** exit 1 Y el JSON expone el
/// diagnóstico de forma accionable. La salida serializa un `Analysis` cuyo `perFile`
/// (`BTreeMap<RelPath, Vec<Check>>`) lista los `Check`, cada uno con su campo `code` (wire
/// `"SCHEMA-REQFIELD"`). Aseveramos que algún check de algún fichero tiene `code == "SCHEMA-REQFIELD"`
/// — campo ya existente en el wire (`check_json_es_valido` fija `concepts`/`hardFail`), sin inventar
/// campos nuevos: el implementador solo debe INYECTAR los SCHEMA-*/REL- en `perFile` de forma
/// aditiva. Hoy `analyze()` no los coloca ahí → rojo por aserción.
#[test]
fn check_json_lista_schema() {
    let dir = temp_dir("json-schema");
    bundle_con_schema_reqfield(&dir);

    let out = bin()
        .arg("--path")
        .arg(&dir)
        .args(["check", "--json"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "SCHEMA-REQFIELD bloquea la puerta (exit 1)"
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let per_file = v["perFile"].as_object().unwrap();
    let lista_schema = per_file
        .values()
        .filter_map(|checks| checks.as_array())
        .flatten()
        .any(|c| c["code"] == "SCHEMA-REQFIELD");
    assert!(
        lista_schema,
        "el JSON debe listar el diagnóstico SCHEMA-REQFIELD en `perFile`, no solo los checks OKF; \
         perFile = {per_file:#?}"
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
///   · el ANCESTRO contiene `malo.md` (sin frontmatter ⇒ `OKF-FM01`, hard fail) ⇒ juzgarlo da exit 1;
///   · el SUBDIRECTORIO contiene solo un `a.md` conforme ⇒ juzgarlo da exit 0.
/// Además se comprueba el inventario juzgado (`concepts` del `--json`, campo ya existente en el
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
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    // Hard fail OKF que vive SOLO en el ancestro.
    write(&proyecto, "malo.md", "# sin frontmatter\n");
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
    let concepts: Vec<&str> = v["concepts"]
        .as_array()
        .expect("`check --json` expone `concepts`")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    assert_eq!(
        concepts,
        vec!["a.md"],
        "`check` debe juzgar el cwd (solo `a.md`), no ascender al ancestro"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "el subdirectorio es conforme por sí mismo → exit 0 (el hard fail es del ancestro)"
    );
}
