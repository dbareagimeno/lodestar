//! Tests **end-to-end** de la CLI: viajes completos de usuario cruzando fachadas y procesos
//! reales (binario `lodestar`). Complementan `cli.rs` (que testea contratos puntuales).
//!
//! E15-H02/H03 dejaron la CLI en `check` + `reindex`: los viajes que encadenaban
//! `init`/generadores/`export`/`import` se retiraron con esos subcomandos, y lo que queda aquí son
//! los e2e de la puerta de CI que siguen vivos.

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lodestar"))
}

/// Directorio temporal aislado **con limpieza automática**, unificado con el resto del repo en
/// `E23-H08` (ver el helper gemelo de `cli.rs`). El arnés anterior derivaba el nombre del PID y
/// **nunca** limpiaba; además, como todos los tests de un binario corren en el mismo proceso, el
/// PID no los distinguía: dos tests que pidieran el mismo `name` compartían directorio.
fn temp_dir(name: &str) -> TempDir {
    tempfile::Builder::new()
        .prefix(&format!("lodestar-e2e-{name}-"))
        .tempdir()
        .expect("crear el directorio temporal del test")
}

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

const CONCEPT_B: &str =
    "---\ntype: Nota\ntitle: Beta\ndescription: la segunda\ntags: [demo]\n---\n\n# H\n\ncuerpo\n";

fn run(dir: &Path, args: &[&str]) -> i32 {
    bin()
        .arg("--path")
        .arg(dir)
        .args(args)
        .status()
        .unwrap()
        .code()
        .unwrap()
}

/// Como [`run`], pero devuelve `(exit code, stderr)`: los criterios de E29-H01 no se conforman con
/// el código de salida, exigen que el mensaje diga **qué** hay que arreglar.
fn run_stderr(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = bin().arg("--path").arg(dir).args(args).output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Un `.lodestar/config.yaml` inválido NO relaja la puerta en silencio: exit 3.
///
/// Migrado en E15-H08: hasta entonces el fichero de config era `lodestar.toml` y este e2e escribía
/// un TOML roto. Con el legado borrado, `lodestar.toml` es un fichero más del proyecto (ver
/// `lodestar_toml_ignorado`) y el fichero cuyo YAML roto **debe** abortar la puerta de CI es el
/// nuevo `.lodestar/config.yaml`: desde que gobierna el descubrimiento, degradar a defaults ante un
/// typo haría que la CI juzgara un conjunto de documentos distinto del declarado, sin avisar.
#[test]
fn config_invalida_es_error_de_runtime() {
    let dir = temp_dir("yaml-roto");
    write(dir.path(), "index.md", "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# B\n");
    // Secuencia de flujo YAML sin cerrar: parseo inválido garantizado.
    write(
        dir.path(),
        ".lodestar/config.yaml",
        "discovery:\n  exclude: [\"notas/**\"\n",
    );
    assert_eq!(run(dir.path(), &["check"]), 3);
}

/// Un `.md` no-UTF8 no aborta el check: se salta con aviso y el resto se juzga.
#[test]
fn md_no_utf8_no_aborta_el_check() {
    let dir = temp_dir("no-utf8");
    write(dir.path(), "index.md", "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# B\n");
    write(dir.path(), "buena.md", CONCEPT_B);
    std::fs::write(
        dir.path().join("latin1.md"),
        b"---\ntype: Nota\n---\n\n# a\xf1o\n",
    )
    .unwrap();
    assert_eq!(run(dir.path(), &["check"]), 0);
}

// ---------------------------------------------------------------------------
// E29-H01 — Config estricta, por la puerta de CI
// (`requirements/epica-29-honestidad-superficie.md`, `decisiones §16(e)` + `§23/A-08`).
//
// `config_invalida_es_error_de_runtime` (arriba) ya fija el exit 3 del YAML **malformado**. Lo que
// esta historia añade son los dos casos que hoy pasan de largo con exit 0:
//
//   · una clave que serde **no reconoce** y descarta en silencio;
//   · un `config.yaml` que existe pero **no se puede leer** y cae a defaults.
//
// Se ejercitan desde el binario porque `docs/user/ci.md` L295 promete literalmente *«A malformed
// `config.yaml` is exit 3, never a silent fallback to defaults»*: la promesa es de la CLI, y quien
// la incumple hoy es la CLI. El tercer test es el **control anti-vacuo**: la ausencia de config es
// legítima (`§20.1`, arranque sin ceremonia) y no puede volverse un error de camino.
// ---------------------------------------------------------------------------

/// **Dado** un `.lodestar/config.yaml` con `workspace: { writeableRoots: ["notas"] }` (typo de
/// `writableRoots`), **Cuando** se ejecuta `lodestar check`, **Entonces** el exit code es `3` y el
/// mensaje nombra la clave desconocida.
///
/// Es el criterio principal de E29-H01 por la fachada donde más duele. Hoy la puerta de CI sale `0`
/// y **VÁLIDO**: serde descarta `writeableRoots`, `writable_roots` queda vacío —que significa *«todo
/// el workspace es escribible»*— y el usuario obtiene una política **más permisiva** que la que
/// escribió, sin una palabra. Una CI que aprueba por un typo de la config de seguridad es peor que
/// no tener CI.
///
/// El escenario está construido para que el rojo NO pueda venir de otro sitio: el documento es
/// conforme, así que el único motivo posible de un exit distinto de `0` es la config. Y se exige el
/// nombre de la clave en stderr porque un exit 3 mudo obliga a adivinar cuál de las líneas del YAML
/// borrar.
#[test]
fn config_con_clave_desconocida_es_exit_3() {
    let dir = temp_dir("clave-desconocida");
    write(dir.path(), "beta.md", CONCEPT_B);
    write(
        dir.path(),
        ".lodestar/config.yaml",
        "workspace:\n  writeableRoots: [\"notas\"]\n",
    );

    let (code, stderr) = run_stderr(dir.path(), &["check"]);
    assert_eq!(
        code, 3,
        "una clave desconocida en `.lodestar/config.yaml` debe abortar la puerta de CI con exit 3, \
         no aprobarla con la política por defecto (más permisiva que la declarada); stderr=\n{stderr}"
    );
    assert!(
        stderr.contains("writeableRoots"),
        "el mensaje debe NOMBRAR la clave rechazada para que el usuario sepa qué línea arreglar; \
         stderr=\n{stderr}"
    );
    assert!(
        stderr.contains("config.yaml"),
        "…y nombrar el fichero, como ya hace el exit 3 del YAML malformado; stderr=\n{stderr}"
    );
}

/// **Dado** un `.lodestar/config.yaml` que existe pero **no se puede leer** (sustituido por un
/// directorio), **Cuando** se ejecuta `lodestar check`, **Entonces** el exit code es `3` y el
/// mensaje dice que la config no se pudo leer.
///
/// `WorkspaceConfig::load` trata hoy **cualquier** `Err` de `read_to_string` como «no hay config» y
/// devuelve los defaults, así que un fichero ilegible es indistinguible de un fichero ausente. La
/// historia obliga a distinguir `NotFound` (legítimo) de todo lo demás (el usuario declaró una
/// política que Lodestar no está aplicando).
///
/// El sustituto es un **directorio** —y no un fichero sin permisos de lectura— porque es la única
/// forma portable de provocar un error distinto de `NotFound` sin depender del uid con el que corra
/// la CI: `chmod 000` no detiene a `root`.
#[test]
fn config_ilegible_no_degrada_a_defaults() {
    let dir = temp_dir("config-ilegible");
    write(dir.path(), "beta.md", CONCEPT_B);
    std::fs::create_dir_all(dir.path().join(".lodestar/config.yaml"))
        .expect("crear el «config.yaml» ilegible (un directorio en su lugar)");
    assert!(
        dir.path().join(".lodestar/config.yaml").exists(),
        "precondición: la config tiene que EXISTIR, o el caso sería el de ausencia (legítimo)"
    );

    let (code, stderr) = run_stderr(dir.path(), &["check"]);
    assert_eq!(
        code, 3,
        "un `config.yaml` ilegible es exit 3, «never a silent fallback to defaults» \
         (`docs/user/ci.md` L295); stderr=\n{stderr}"
    );
    assert!(
        stderr.contains("config.yaml"),
        "el mensaje debe nombrar el fichero que no se pudo leer: un «error de IO» a secas es \
         indistinguible de un disco lleno; stderr=\n{stderr}"
    );
}

/// **Dado** un workspace **sin** `.lodestar/config.yaml`, **Cuando** se ejecuta `lodestar check`,
/// **Entonces** funciona con los defaults y sale `0`/`1` según los diagnósticos, igual que hoy.
///
/// **Control anti-vacuo**, y el más importante de la historia: endurecer la lectura de la config es
/// exactamente el cambio que puede convertir «no hay fichero» en «error al leer el fichero» de un
/// plumazo (basta con dejar de distinguir `ErrorKind::NotFound`). Eso rompería `§20.1` —arranque sin
/// ceremonia: `cd my-project && lodestar check`— y con él la premisa de toda la v0.3.
///
/// Se ejercitan los **dos** veredictos posibles sin config, porque un rechazo mal cerrado podría
/// dejar pasar el caso conforme y tumbar el otro (o al revés):
///
/// - un workspace conforme sale `0`;
/// - uno con un enlace roto a otro `.md` sale `1` (`danglingDocumentLinks` es `error` por defecto en
///   `§20.9`) — lo que además prueba que la política por defecto se está **aplicando**, no
///   simplemente que el binario no ha reventado.
#[test]
fn config_ausente_sigue_cayendo_a_defaults() {
    // --- (1) Sin config y conforme: exit 0 ------------------------------------------
    let ok = temp_dir("sin-config-ok");
    write(ok.path(), "beta.md", CONCEPT_B);
    assert!(
        !ok.path().join(".lodestar/config.yaml").exists(),
        "precondición: el escenario exige un workspace SIN config"
    );

    let (code, stderr) = run_stderr(ok.path(), &["check"]);
    assert_eq!(
        code, 0,
        "la ausencia de `.lodestar/config.yaml` es un estado legítimo y permanente (`§20.1`): \
         endurecer la config NO puede convertirla en un error de arranque; stderr=\n{stderr}"
    );

    // --- (2) Sin config y con un error: exit 1 (los defaults se APLICAN) -------------
    let roto = temp_dir("sin-config-roto");
    write(
        roto.path(),
        "beta.md",
        "---\ntype: Nota\ntitle: Beta\n---\n\n# H\n\n[a la nada](fantasma.md)\n",
    );

    let (code, stderr) = run_stderr(roto.path(), &["check"]);
    assert_eq!(
        code, 1,
        "sin config se juzga con los defaults de `§20.9`, donde un enlace a un `.md` inexistente \
         es error: si esto saliera 0, «caer a defaults» se habría vuelto «no validar nada»; \
         stderr=\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// E29-H06 — Un workspace vacío se distingue de un directorio equivocado
// (`requirements/epica-29-honestidad-superficie.md §E29-H06`, `decisiones §16(f)`).
//
// Un directorio sin `.md` (o cuya `discovery.include` los excluye todos) daba `lodestar check`
// exit 0 · VÁLIDO, indistinguible de un repo legítimamente vacío. Esta sección fija el diagnóstico
// `WORKSPACE-EMPTY` (severidad `warn`) por la fachada CLI, SIN cambiar ningún exit code salvo el
// que ya dependa de `gate.blockWarnings`.
//
// PUERTA DE DECISIÓN DE ANCLAJE: ver la nota completa en
// `crates/lodestar-app/tests/validacion.rs` (sección gemela E29-H06). Resumen: `RelPath::new("")`
// es inválido por diseño (invariante #6), así que anclar el diagnóstico a la raíz como `target` es
// inviable; se elige extender el indexado de `full_analysis` para los diagnósticos sin `target`. La
// forma concreta de la clave la decide el implementador — estos tests solo exigen el efecto
// observable: el código aparece en la salida de `lodestar check --json`.
//
// ROJO esperado HOY: por ASERCIÓN (no hay productor de `WORKSPACE-EMPTY` en ninguna parte).
// ---------------------------------------------------------------------------

/// Corre `lodestar check --json` sobre `dir` y devuelve `(exit code, JSON de stdout)`.
fn check_json(dir: &Path) -> (i32, serde_json::Value) {
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
    (out.status.code().unwrap(), v)
}

/// Los códigos de todos los diagnósticos que viajan en `diagnostics` del `check --json`
/// (`{path: [Check]}`), aplanados.
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

/// **Criterio `check_en_workspace_vacio_avisa_con_exit_0`**: **Dado** un directorio temporal sin
/// ningún `.md`, **Cuando** se ejecuta `lodestar check`, **Entonces** el exit code sigue siendo `0`
/// y la salida (`--json`) incluye un aviso `WORKSPACE-EMPTY`.
///
/// Un fichero no-Markdown en el directorio (`LEEME.txt`) prueba que el aviso depende del inventario
/// de DOCUMENTOS, no de si el directorio está vacío a secas.
#[test]
fn check_en_workspace_vacio_avisa_con_exit_0() {
    let dir = temp_dir("vacio");
    write(dir.path(), "LEEME.txt", "esto no es un documento OKF\n");

    let (code, json) = check_json(dir.path());
    let codigos = codigos_del_json(&json);
    assert_eq!(
        code, 0,
        "un workspace vacío SIN otros diagnósticos debe seguir pasando la puerta de CI (exit 0): \
         json={json}"
    );
    assert!(
        codigos.iter().any(|c| c == "WORKSPACE-EMPTY"),
        "`lodestar check --json` sobre un directorio sin `.md` debe incluir el aviso \
         WORKSPACE-EMPTY: códigos vistos = {codigos:?}, json={json}"
    );
}

/// **Criterio `workspace_con_todo_excluido_tambien_avisa`** (mitad CLI): un directorio **con** `.md`
/// pero cuya `discovery.include` los excluye todos también avisa — el caso engañoso no es solo «no
/// hay ficheros», es «no hay inventario».
#[test]
fn cli_workspace_con_todo_excluido_tambien_avisa() {
    let dir = temp_dir("todo-excluido");
    write(dir.path(), "notas/alfa.md", "# Alfa\n\ncontenido real.\n");
    write(
        dir.path(),
        ".lodestar/config.yaml",
        "discovery:\n  include: [\"solo-esto/**/*.md\"]\n",
    );

    let (code, json) = check_json(dir.path());
    let codigos = codigos_del_json(&json);
    assert_eq!(
        code, 0,
        "el aviso no debe cambiar el exit code por sí solo: json={json}"
    );
    assert!(
        codigos.iter().any(|c| c == "WORKSPACE-EMPTY"),
        "un `discovery.include` que excluye TODO también debe avisar con WORKSPACE-EMPTY: \
         códigos vistos = {codigos:?}, json={json}"
    );
}

/// **Criterio `workspace_con_documentos_no_avisa`** (control anti-vacuo, mitad CLI): un workspace
/// con al menos un documento no lleva `WORKSPACE-EMPTY` en la salida de `lodestar check --json`.
#[test]
fn cli_workspace_con_documentos_no_avisa() {
    let dir = temp_dir("con-documentos");
    write(dir.path(), "beta.md", CONCEPT_B);

    let (code, json) = check_json(dir.path());
    let codigos = codigos_del_json(&json);
    assert_eq!(code, 0, "el workspace es conforme: json={json}");
    assert!(
        !codigos.iter().any(|c| c == "WORKSPACE-EMPTY"),
        "un workspace con documentos NO debe llevar WORKSPACE-EMPTY: códigos vistos = {codigos:?}, \
         json={json}"
    );
}

/// **Criterio `el_aviso_de_vacio_lo_ven_las_dos_fachadas`**: la salida de `lodestar check --json`
/// sobre un workspace vacío y la de `knowledge_check(scope: workspace)` (llamado en proceso, vía
/// `lodestar-app`, sobre el MISMO directorio) contienen las DOS el diagnóstico `WORKSPACE-EMPTY`
/// (invariante #3: una sola verdad computada).
#[test]
fn el_aviso_de_vacio_lo_ven_las_dos_fachadas() {
    let dir = temp_dir("dos-fachadas");

    let (code, json) = check_json(dir.path());
    assert_eq!(code, 0, "workspace vacío: sigue siendo exit 0: json={json}");
    let codigos_cli = codigos_del_json(&json);
    assert!(
        codigos_cli.iter().any(|c| c == "WORKSPACE-EMPTY"),
        "`lodestar check --json` debe llevar WORKSPACE-EMPTY: códigos vistos = {codigos_cli:?}"
    );

    let app = lodestar_app::App::open(dir.path()).expect("el workspace temporal debe abrir");
    let report = app
        .knowledge_check(
            &lodestar_app::CheckScope::Workspace,
            Some(lodestar_core::types::Severity::Info),
            false,
            Some(1000),
            None,
        )
        .expect("knowledge_check(workspace) debe responder");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|c| c.code == lodestar_core::types::CheckCode::WorkspaceEmpty),
        "`knowledge_check(scope: workspace)` sobre el MISMO directorio también debe llevar \
         WORKSPACE-EMPTY: diagnósticos = {:?}",
        report
            .diagnostics
            .iter()
            .map(|c| c.code.as_str())
            .collect::<Vec<_>>()
    );
}

/// **Criterio `el_aviso_de_vacio_respeta_block_warnings`** (mitad CLI): con
/// `gate.blockWarnings: true`, `lodestar check` sobre un workspace vacío sale `1` — por la
/// POLÍTICA del usuario, no porque el aviso en sí bloquee.
#[test]
fn el_aviso_de_vacio_respeta_block_warnings() {
    let dir = temp_dir("vacio-block-warnings");
    write(
        dir.path(),
        ".lodestar/config.yaml",
        "gate:\n  blockWarnings: true\n",
    );

    assert_eq!(
        run(dir.path(), &["check"]),
        1,
        "con `gate.blockWarnings: true`, un workspace vacío (que SOLO tiene el aviso \
         WORKSPACE-EMPTY) debe bloquear la puerta de CI: si WORKSPACE-EMPTY no cuenta como `warn` \
         en el `Analysis`, `gate_blocked` no lo ve y este exit sigue siendo 0"
    );
}
