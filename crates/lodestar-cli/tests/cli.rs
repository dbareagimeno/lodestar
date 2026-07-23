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

#[test]
fn index_drift_exit_4_luego_0() {
    let dir = temp_dir("drift");
    write(
        &dir,
        "a.md",
        "---\ntype: Concept\ntitle: A\ndescription: d\n---\n\n# H\n",
    );
    // Sin index.md generado → drift.
    let drift = bin()
        .arg("--path")
        .arg(&dir)
        .args(["index", "--check"])
        .status()
        .unwrap();
    assert_eq!(drift.code(), Some(4));
    // Genera y vuelve a comprobar → 0.
    let gen = bin().arg("--path").arg(&dir).arg("index").status().unwrap();
    assert_eq!(gen.code(), Some(0));
    let ok = bin()
        .arg("--path")
        .arg(&dir)
        .args(["index", "--check"])
        .status()
        .unwrap();
    assert_eq!(ok.code(), Some(0));
}

#[test]
fn export_genera_zip() {
    let dir = temp_dir("export");
    write(
        &dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n",
    );
    let out = dir.join("salida.zip");
    let status = bin()
        .arg("--path")
        .arg(&dir)
        .args(["export", "--out"])
        .arg(&out)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(0));
    assert!(out.is_file());
}

#[test]
fn init_scaffold() {
    let dir = temp_dir("init");
    let target = dir.join("nuevo");
    let status = bin().arg("init").arg(&target).status().unwrap();
    assert_eq!(status.code(), Some(0));
    assert!(target.join("index.md").is_file());
    assert!(target.join(".gitignore").is_file());
}

// `check_staged_sin_git_exit_3` y `check_rev_head_tras_init` se retiran en E9-H02: probaban
// `check --staged`/`--rev`, retirados de la superficie de la CLI (el crate `vcs` queda dormido).
// El contrato nuevo lo cubren `check_rev_es_uso` y `check_working_tree_conforme` más abajo.

#[test]
fn import_desde_zip_del_prototipo() {
    // Exporta un bundle a .zip y lo reimporta en un directorio nuevo (roundtrip).
    let dir = temp_dir("import-src");
    write(
        &dir,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n",
    );
    let zip = dir.join("bundle.zip");
    assert_eq!(
        bin()
            .arg("--path")
            .arg(&dir)
            .args(["export", "--out"])
            .arg(&zip)
            .status()
            .unwrap()
            .code(),
        Some(0)
    );
    let dest = temp_dir("import-dest");
    let status = bin()
        .arg("--path")
        .arg(&dest)
        .arg("import")
        .arg(&zip)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(0));
    assert!(dest.join("a.md").is_file());
}

// --- E9-H02: retirar los subcomandos git de la CLI (conservando `check`) ---

/// E9-H02 `help_sin_subcomandos_git`: **Dado** `lodestar --help`, **Entonces** NO aparecen los 8
/// subcomandos git. Retiramos exposición, no capacidad (el crate `vcs` queda dormido).
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
/// flag retirado con el crate `vcs` dormido — D-check). No juzga ningún árbol git.
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

#[test]
fn import_rechaza_zip_slip() {
    // Un zip con una ruta con `..` no debe escribir fuera del bundle (chokepoint RelPath).
    let dir = temp_dir("zipslip");
    let zip_path = dir.join("evil.zip");
    {
        let f = std::fs::File::create(&zip_path).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        use zip::write::SimpleFileOptions;
        zw.start_file("../evil.md", SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zw, b"---\ntype: X\n---\n\n# H\n").unwrap();
        zw.finish().unwrap();
    }
    let dest = temp_dir("zipslip-dest");
    let status = bin()
        .arg("--path")
        .arg(&dest)
        .arg("import")
        .arg(&zip_path)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(0)); // no falla, pero...
                                        // ...la ruta insegura se ignora: no se escribe fuera del destino.
    assert!(!dest.parent().unwrap().join("evil.md").exists());
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
