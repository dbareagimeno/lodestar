//! E14-H04 — Benchmark funcional (`REFACTOR §17`) como suite e2e.
//!
//! Los 15 escenarios de producto de la tabla `REFACTOR §17` (líneas 1436-1452), compuestos como
//! **viaje e2e** que cruza la superficie real: el servidor MCP por stdio (10 tools, E10–E13). NO se
//! llaman funciones internas del `App`/`Workspace`: cada escenario arranca el binario
//! `lodestar-mcp`, le habla JSON-RPC y asevera sobre las respuestas y el disco. La mayoría de los
//! mecanismos ya existen (las tools se cerraron en E10-H09…E13-H09, dependencia E13-H09), así que
//! esta historia es de **composición/regresión e2e**: verifica que el conjunto de las 10 tools cubre
//! los escenarios de producto de punta a punta sobre un bundle de benchmark realista.
//!
//! ## Códigos de error REALES (los que emite el motor HOY, no los idealizados de §17)
//! El catálogo `ErrorCode` (`lodestar-core::types`, invariante #4) está congelado en 16 variantes;
//! cada escenario asevera el código estable que el motor emite de verdad (verificado en
//! `crates/lodestar-app/src/lib.rs` `error_code`/`workspace_error_code` y `types.rs`):
//!   - Escenario 3 (crear sin campo obligatorio): §17 dice «Plan rechazado». El motor lo materializa
//!     en DOS superficies: `change_plan` devuelve `canApply:false` con `diagnosticsAfter.errors>=1`,
//!     y `change_apply` lo rechaza en el staging con **`NONCONFORMANT_RESULT`** (E13-H01). Se
//!     aseveran ambas.
//!   - Escenario 5 (borrar referenciado): §17 dice «Rechazo con blockers». El motor emite
//!     **`INBOUND_LINKS_EXIST`** al normalizar un `delete` con política `Reject` (los enlaces
//!     entrantes SON los blockers).
//!   - Escenario 6 (modificar cambiado externamente): §17 dice `REVISION_CONFLICT` y el motor emite
//!     exactamente **`REVISION_CONFLICT`** (control optimista por op en `change_plan`). Sin
//!     divergencia.
//!   - Escenario 8 (relación inválida): **`RELATION_CONSTRAINT_VIOLATION`**, antes de escribir
//!     (`change_plan` no toca disco).
//!   - Escenario 13 (fuera de writableRoots): **`PERMISSION_DENIED`** en `change_apply`.
//!   - Escenario 14 (ref de código inexistente): el «diagnóstico» aflora en `knowledge_get` como una
//!     `externalReference` con **`exists:false`** (el check `EXTREF-MISSING` es de la workspace, no
//!     lo fusiona `knowledge_check`; su superficie e2e es `knowledge_get(include:[externalReferences])`).
//!   - Escenario 15 (Markdown inválido a mano): el check **`OKF-TYPE`** (hard-fail) vía
//!     `knowledge_check` scope workspace, `conformant:false`.
//!
//! ## Escenario 12 (crash durante publicación)
//! La PRUEBA AUTORITATIVA de recuperación determinista es el property test
//! **`recovery_sin_parciales`** (E13-H06, `crates/lodestar-workspace/tests/transactions.rs`, gateado
//! tras la feature `test-failpoints`): recorre TODOS los `FailPoint` × dos formas de change set y
//! asevera que el canónico converge a UNO de los dos bordes (todo original o todo resultado), nunca
//! parcial. Ese test vive en OTRO crate y tras una feature que el binario `lodestar-mcp` no compila,
//! así que NO se puede invocar desde aquí. Este benchmark lo COMPLEMENTA con una comprobación e2e de
//! **durabilidad determinista tras reabrir** (`escenario_12_*`): un `change_apply` que se publica
//! (sella `done`) sobrevive a cerrar y reabrir el servidor — un proceso fresco reporta EXACTAMENTE
//! la revisión resultante, el `.md` persiste y el workspace sigue conforme. Modela «cerrar Lodestar
//! (tras) la publicación» con estado determinista; el borde de crash A MITAD lo cubre E13-H06.
//!
//! ## Estructura
//! Cada escenario es una función `escenario_NN_*()` autocontenida (su propio bundle temporal + sus
//! aserciones e2e). Hay UN `#[test]` por fila (`bench_NN_*`, diagnóstico granular: una fila que
//! falla se nombra a sí misma) y un `#[test] benchmark_15_escenarios` que ejerce las 15 en secuencia
//! (el test que nombra la spec, el viaje completo). Ambas formas son reales y no vacuas.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Arnés e2e (local a este binario de test; los helpers de `mcp.rs` viven en otro binario).
// ---------------------------------------------------------------------------

/// Escribe un fichero (creando directorios) bajo `dir`.
fn write(dir: &std::path::Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// Arranca el servidor MCP (perfil `standard`) sobre `dir`, envía `lines` y devuelve las primeras
/// `expect` respuestas JSON-RPC. stdout debe ser JSON-RPC puro.
fn roundtrip(dir: &std::path::Path, lines: &[String], expect: usize) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg("--root")
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    for l in lines {
        writeln!(stdin, "{l}").unwrap();
    }
    stdin.flush().unwrap();
    drop(stdin);
    let mut out = Vec::new();
    for line in (&mut stdout).lines().map_while(Result::ok) {
        out.push(serde_json::from_str(&line).expect("stdout = JSON-RPC puro"));
        if out.len() == expect {
            break;
        }
    }
    child.wait().ok();
    out
}

/// Línea `tools/call` para `name` con `arguments`.
fn call(id: u64, name: &str, args: Value) -> String {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": { "name": name, "arguments": args }
    })
    .to_string()
}

/// `structuredContent` de una respuesta de tool, tras verificar que es un objeto (documenta el hueco
/// si la tool/servicio faltara).
fn sc(resp: &Value) -> &Value {
    let sc = &resp["result"]["structuredContent"];
    assert!(
        sc.is_object(),
        "la tool debe devolver structuredContent (objeto): {resp:?}"
    );
    sc
}

/// `true` si la respuesta es un error de EJECUCIÓN de tool que expone el código estable `code`.
fn es_error_con(resp: &Value, code: &str) -> bool {
    resp["result"]["isError"] == Value::Bool(true)
        && resp["error"].is_null()
        && resp.to_string().contains(code)
}

/// Política permisiva: no exige resultado conforme, admite warnings.
fn policy_permisiva() -> Value {
    json!({ "requireConformantResult": false, "allowWarnings": true })
}

/// Política estricta: exige resultado conforme (para probar «plan rechazado»).
fn policy_estricta() -> Value {
    json!({ "requireConformantResult": true, "allowWarnings": true })
}

/// Línea `change_plan` con `operations`/`policy`.
fn change_plan_line(id: u64, operations: Value, policy: Value) -> String {
    call(
        id,
        "change_plan",
        json!({ "operations": operations, "policy": policy }),
    )
}

/// Línea `change_apply` para un `changeSetId`.
fn change_apply_line(id: u64, change_set_id: &str) -> String {
    call(id, "change_apply", json!({ "changeSetId": change_set_id }))
}

/// Línea `change_revert` para un `receiptId`.
fn change_revert_line(id: u64, receipt_id: &str) -> String {
    call(id, "change_revert", json!({ "receiptId": receipt_id }))
}

/// El `changeSetId` de una respuesta `change_plan`.
fn plan_id(resp: &Value) -> String {
    sc(resp)["changeSetId"]
        .as_str()
        .unwrap_or_else(|| panic!("change_plan debe devolver changeSetId: {resp:?}"))
        .to_string()
}

/// Snapshot del conocimiento en disco (`RelPath` → contenido), excluyendo `.lodestar/`. Para
/// aseverar «no escribió».
fn snapshot_md(root: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    fn walk(
        base: &std::path::Path,
        dir: &std::path::Path,
        map: &mut std::collections::BTreeMap<String, String>,
    ) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().and_then(|n| n.to_str()) == Some(".lodestar") {
                    continue;
                }
                walk(base, &path, map);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                let rel = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                map.insert(rel, std::fs::read_to_string(&path).unwrap());
            }
        }
    }
    let mut map = std::collections::BTreeMap::new();
    walk(root, root, &mut map);
    map
}

// ---------------------------------------------------------------------------
// Bundles de benchmark.
// ---------------------------------------------------------------------------

const INDEX: &str = "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n";

/// Bundle mínimo (solo `index.md`).
fn bundle_min() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "index.md", INDEX);
    dir
}

/// Bundle con `.lodestar/schema.yaml` que declara `decision` con `requiredFields:[title,status,
/// rationale]` (para el escenario 3) y `note` (segundo tipo).
fn bundle_schema_decision() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "index.md", INDEX);
    write(
        dir.path(),
        ".lodestar/schema.yaml",
        "\
version: \"1\"
types:
  decision:
    name: decision
    description: Una decision registrada
    requiredFields: [title, status, rationale]
    allowedStatuses: [proposed, accepted, rejected]
  note:
    name: note
    description: Una nota libre
    requiredFields: [title]
",
    );
    dir
}

/// Bundle con schema `task.depends_on -> [component]` y conceptos `component`/`note`/`task`
/// (escenarios 8 y 9).
fn bundle_relaciones(task_depends_on: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "index.md", INDEX);
    write(
        dir.path(),
        ".lodestar/schema.yaml",
        "\
version: \"1\"
types:
  component:
    name: component
    description: Un componente
  note:
    name: note
    description: Una nota
  task:
    name: task
    description: Una tarea que depende de un componente
    relations:
      depends_on:
        targetTypes: [component]
        cardinality: many
",
    );
    write(
        dir.path(),
        "component.md",
        "---\ntype: component\ntitle: Componente\ndescription: el nucleo\n---\n\n# Componente\n\ncuerpo\n",
    );
    write(
        dir.path(),
        "nota.md",
        "---\ntype: note\ntitle: Nota\ndescription: irrelevante\n---\n\n# Nota\n\ncuerpo\n",
    );
    write(
        dir.path(),
        "tarea.md",
        &format!(
            "---\ntype: task\ntitle: Tarea\ndescription: depende de algo\n{task_depends_on}---\n\n# Tarea\n\ncuerpo\n"
        ),
    );
    dir
}

/// Bundle con 4 conceptos relacionados en anillo (`a`/`b`/`c`/`d`), conformes (escenario 7).
fn bundle_cinco_relacionados() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [A](a.md)\n* [B](b.md)\n* [C](c.md)\n* [D](d.md)\n",
    );
    for (slug, next) in [("a", "b"), ("b", "c"), ("c", "d"), ("d", "a")] {
        let up = slug.to_uppercase();
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!(
                "---\ntype: Concept\ntitle: {up}\ndescription: nodo {slug} del cluster\n---\n\n# {up}\n\n[Siguiente]({next}.md)\n"
            ),
        );
    }
    dir
}

/// Las 5 operaciones del escenario 7: 1 `create` + 4 `patch_frontmatter`.
fn cinco_operaciones() -> Value {
    json!([
        { "op": "create", "path": "nuevo.md", "type": "Concept", "title": "Nuevo",
          "body": "# Nuevo\n\ncuerpo del quinto concepto\n" },
        { "op": "patch_frontmatter", "ref": { "path": "a.md" }, "patch": { "description": "a v2" } },
        { "op": "patch_frontmatter", "ref": { "path": "b.md" }, "patch": { "description": "b v2" } },
        { "op": "patch_frontmatter", "ref": { "path": "c.md" }, "patch": { "description": "c v2" } },
        { "op": "patch_frontmatter", "ref": { "path": "d.md" }, "patch": { "description": "d v2" } },
    ])
}

/// Bundle con `target.md` referenciado por EXACTAMENTE 30 emisores de cuerpo (escenario 4).
fn bundle_treinta_backlinks() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "index.md", INDEX);
    write(
        dir.path(),
        "target.md",
        "---\ntype: Concept\ntitle: Target\ndescription: el concepto a mover\n---\n\n# Target\n\ncuerpo\n",
    );
    for i in 0..30 {
        write(
            dir.path(),
            &format!("emisor{i:02}.md"),
            &format!(
                "---\ntype: Concept\ntitle: Emisor {i:02}\ndescription: enlaza al target\n---\n\n# H\n\nreferencia a [target](/target.md).\n"
            ),
        );
    }
    dir
}

// ===========================================================================
// Escenario 1 — Encontrar una decisión por significado → knowledge_search + knowledge_get.
// ===========================================================================
fn escenario_01_buscar_por_significado() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Auth](auth.md)\n",
    );
    write(
        dir.path(),
        "auth.md",
        "---\ntype: decision\ntitle: Autenticacion con tokens\ndescription: Como autenticar usuarios\nstatus: accepted\ntags: [seguridad]\n---\n\n# Resumen\n\nDecidimos usar autenticacion basada en tokens rotatorios.\n",
    );
    write(
        dir.path(),
        "bici.md",
        "---\ntype: concept\ntitle: Bicicletas\ndescription: sobre ruedas\n---\n\n# H\n\nnada que ver con el tema.\n",
    );

    // (1) knowledge_search por significado: encuentra la decisión, no el decoy.
    let resp = roundtrip(
        dir.path(),
        &[call(
            1,
            "knowledge_search",
            json!({ "text": "autenticacion" }),
        )],
        1,
    );
    let results = sc(&resp[0])["results"]
        .as_array()
        .unwrap_or_else(|| panic!("knowledge_search debe devolver results: {resp:?}"));
    assert!(
        results.iter().any(|r| r["path"] == "auth.md"),
        "la decisión que casa «autenticacion» debe aparecer: {resp:?}"
    );
    assert!(
        !results.iter().any(|r| r["path"] == "bici.md"),
        "el decoy que no casa NO debe aparecer: {resp:?}"
    );

    // (2) knowledge_get del resultado: recupera revisión + frontmatter + cuerpo.
    let get = roundtrip(
        dir.path(),
        &[call(
            2,
            "knowledge_get",
            json!({ "ref": { "path": "auth.md" }, "include": ["frontmatter", "body", "revision"] }),
        )],
        1,
    );
    let concept = &sc(&get[0])["concept"];
    assert!(
        concept["revision"]
            .as_str()
            .unwrap_or("")
            .starts_with("blake3:"),
        "knowledge_get debe traer revision «blake3:…»: {get:?}"
    );
    assert!(
        concept["frontmatter"].is_object(),
        "knowledge_get debe traer el frontmatter: {get:?}"
    );
    assert!(
        concept["body"]
            .as_str()
            .unwrap_or("")
            .contains("tokens rotatorios"),
        "knowledge_get debe traer el cuerpo de la decisión: {get:?}"
    );
}

// ===========================================================================
// Escenario 2 — Crear un concepto válido → plan aceptado y aplicado.
// ===========================================================================
fn escenario_02_crear_valido() {
    let dir = bundle_min();
    let ops = json!([
        { "op": "create", "path": "nuevo.md", "type": "Nota", "title": "Nuevo",
          "body": "# Resumen\n\ncuerpo del concepto nuevo\n" },
    ]);
    // (1) Plan aceptado: canApply true bajo política estricta (conforme).
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(1, ops, policy_estricta())],
        1,
    );
    assert_eq!(
        sc(&plan[0])["canApply"],
        Value::Bool(true),
        "un create conforme debe dar un plan aplicable (canApply:true): {plan:?}"
    );
    let id = plan_id(&plan[0]);

    // (2) Plan aplicado: applied true y el .md canónico existe.
    let applied = roundtrip(dir.path(), &[change_apply_line(2, &id)], 1);
    assert_eq!(
        sc(&applied[0])["applied"],
        Value::Bool(true),
        "el plan válido debe aplicarse (applied:true): {applied:?}"
    );
    assert!(
        dir.path().join("nuevo.md").is_file(),
        "el apply debe materializar el .md: {applied:?}"
    );
}

// ===========================================================================
// Escenario 3 — Crear un concepto sin campo obligatorio → plan rechazado.
//
// Dos superficies deben rechazarlo para que «sin campo obligatorio» NUNCA acabe publicado:
//   (1) change_plan: canApply:false + diagnosticsAfter.errors>=1  → VERDE (change_plan usa
//       `plan::validate_result`, que SÍ incluye la validación schema-driven).
//   (2) change_apply: NONCONFORMANT_RESULT y no escribe            → ROJO (HUECO REAL).
//
// HUECO (fase roja para el implementador de E14-H04): `change_apply` PUBLICA el concepto no
// conforme y reporta `conformance.conformant:true` pese a los `SCHEMA-REQFIELD` (level err). Causa:
// `Workspace::validate_staging` (E13-H01, `crates/lodestar-workspace/src/staging.rs`) mide solo
// `bundle.analyze().hard_fail` (los 15 checks OKF) y NO ejecuta `validate_schema`/`validate_relations`
// — así un `SCHEMA-REQFIELD` no cuenta como fallo duro y la publicación pasa el gate. Es una
// divergencia del invariante #3 (una sola verdad computada): `knowledge_check`/`lodestar check` sobre
// el mismo resultado dirían `conformant:false`, pero el gate del único-escritor dice `true`. Cerrarlo:
// extender el gate de staging a la conformidad schema-driven (o que `change_apply` rechace un plan
// persistido con `canApply:false`).
// ===========================================================================
fn escenario_03_crear_sin_campo_obligatorio() {
    let dir = bundle_schema_decision();
    // `decision` requiere [title, status, rationale]; el create solo aporta type+title ⇒ faltan
    // status y rationale ⇒ SCHEMA-REQFIELD ⇒ resultado no conforme.
    let ops = json!([
        { "op": "create", "path": "dec.md", "type": "decision", "title": "Sin campos",
          "body": "# Sin campos\n\ncuerpo\n" },
    ]);

    // (1) change_plan bajo política ESTRICTA: el plan se rechaza a sí mismo (canApply:false) y
    //     reporta los errores del resultado hipotético.
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(1, ops, policy_estricta())],
        1,
    );
    assert_eq!(
        sc(&plan[0])["canApply"],
        Value::Bool(false),
        "un create sin campos obligatorios NO debe ser aplicable (canApply:false): {plan:?}"
    );
    assert!(
        sc(&plan[0])["diagnosticsAfter"]["errors"]
            .as_u64()
            .unwrap_or(0)
            >= 1,
        "el plan debe reportar >=1 error de conformidad en diagnosticsAfter: {plan:?}"
    );
    let id = plan_id(&plan[0]);

    // (2) change_apply DEBE rechazar el plan no conforme: «sin campo obligatorio» no puede acabar
    //     escrito en el canónico. HOY es ROJO — el gate de staging no valida schema-driven (ver la
    //     nota de HUECO de la cabecera), así que publica el concepto reportando conformant:true.
    let applied = roundtrip(dir.path(), &[change_apply_line(2, &id)], 1);
    assert!(
        es_error_con(&applied[0], "NONCONFORMANT_RESULT"),
        "HUECO: change_apply publica un create con SCHEMA-REQFIELD (err) en vez de rechazarlo con \
         NONCONFORMANT_RESULT — el gate de validate_staging solo cuenta hard_fail OKF, no valida \
         schema-driven (invariante #3): {applied:?}"
    );
    assert!(
        !dir.path().join("dec.md").exists(),
        "HUECO: un create no conforme por schema NO debe materializar el .md, pero change_apply lo \
         escribe: {applied:?}"
    );
}

// ===========================================================================
// Escenario 4 — Mover un concepto con 30 backlinks → enlaces actualizados dentro del mismo plan.
// ===========================================================================
fn escenario_04_mover_30_backlinks() {
    let dir = bundle_treinta_backlinks();
    let antes = snapshot_md(dir.path());

    let ops = json!([
        { "op": "move", "from": "target.md", "to": "movido/target.md", "rewriteInboundLinks": true },
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(1, ops, policy_permisiva())],
        1,
    );
    let normalized = sc(&plan[0])["normalizedOperations"]
        .as_array()
        .unwrap_or_else(|| panic!("change_plan debe devolver normalizedOperations: {plan:?}"));

    // El plan lleva el Move MÁS las 30 reescrituras de enlaces entrantes, todo en UN change set: 31.
    assert_eq!(
        normalized.len(),
        31,
        "mover con 30 backlinks debe producir 1 Move + 30 reescrituras = 31 ops en un solo plan: {plan:?}"
    );
    assert!(
        !plan_id(&plan[0]).is_empty(),
        "el plan del move debe tener un único changeSetId: {plan:?}"
    );

    // No escribe (la actualización de enlaces vive DENTRO del plan, aún sin aplicar).
    assert_eq!(
        antes,
        snapshot_md(dir.path()),
        "change_plan del move NO debe tocar el disco"
    );
}

// ===========================================================================
// Escenario 5 — Borrar un concepto referenciado → rechazo con blockers (INBOUND_LINKS_EXIST).
// ===========================================================================
fn escenario_05_borrar_referenciado() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [A](a.md)\n* [B](b.md)\n* [C](c.md)\n",
    );
    write(
        dir.path(),
        "objetivo.md",
        "---\ntype: concept\ntitle: Objetivo\ndescription: referenciado por 3\n---\n\n# Objetivo\n\ncuerpo\n",
    );
    for slug in ["a", "b", "c"] {
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!(
                "---\ntype: concept\ntitle: {slug}\ndescription: enlaza al objetivo\n---\n\n# {slug}\n\n[Objetivo](objetivo.md)\n"
            ),
        );
    }

    // delete con la política por defecto (Reject): los 3 entrantes son blockers ⇒ INBOUND_LINKS_EXIST.
    let ops = json!([ { "op": "delete", "ref": { "path": "objetivo.md" } } ]);
    let resp = roundtrip(
        dir.path(),
        &[change_plan_line(1, ops, policy_permisiva())],
        1,
    );
    assert!(
        es_error_con(&resp[0], "INBOUND_LINKS_EXIST"),
        "borrar un concepto referenciado debe rechazarse con INBOUND_LINKS_EXIST: {resp:?}"
    );
    assert!(
        dir.path().join("objetivo.md").is_file(),
        "un delete rechazado NO debe borrar el .md: {resp:?}"
    );
}

// ===========================================================================
// Escenario 6 — Modificar un concepto cambiado externamente → REVISION_CONFLICT.
// ===========================================================================
fn escenario_06_conflicto_revision() {
    let dir = bundle_cinco_relacionados();

    // (1) Revisión actual de a.md.
    let get = roundtrip(
        dir.path(),
        &[call(
            1,
            "knowledge_get",
            json!({ "ref": { "path": "a.md" }, "include": ["revision"] }),
        )],
        1,
    );
    let old_rev = sc(&get[0])["concept"]["revision"]
        .as_str()
        .unwrap_or_else(|| panic!("knowledge_get debe devolver revision de a.md: {get:?}"))
        .to_string();

    // (2) a.md cambia EN DISCO (cambio externo entre lectura y plan).
    write(
        dir.path(),
        "a.md",
        "---\ntype: Concept\ntitle: A\ndescription: CAMBIADA EXTERNAMENTE\n---\n\n# A\n\notro cuerpo\n",
    );

    // (3) change_plan con la revisión VIEJA ⇒ REVISION_CONFLICT.
    let ops = json!([
        { "op": "patch_frontmatter", "ref": { "path": "a.md" },
          "patch": { "description": "desde el plan" }, "expectedRevision": old_rev },
    ]);
    let resp = roundtrip(
        dir.path(),
        &[change_plan_line(2, ops, policy_permisiva())],
        1,
    );
    assert!(
        es_error_con(&resp[0], "REVISION_CONFLICT"),
        "modificar sobre una revisión obsoleta debe dar REVISION_CONFLICT: {resp:?}"
    );
}

// ===========================================================================
// Escenario 7 — Cambiar cinco conceptos relacionados → un único change set.
// ===========================================================================
fn escenario_07_cinco_conceptos() {
    let dir = bundle_cinco_relacionados();
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(1, cinco_operaciones(), policy_permisiva())],
        1,
    );
    let s = sc(&plan[0]);
    assert!(
        s["changeSetId"].as_str().is_some_and(|id| !id.is_empty()),
        "las 5 ops deben producir un único changeSetId: {plan:?}"
    );
    assert_eq!(
        s["normalizedOperations"].as_array().map(Vec::len),
        Some(5),
        "las 5 ops relacionadas deben caber en un solo change set con 5 normalizedOperations: {plan:?}"
    );
}

// ===========================================================================
// Escenario 8 — Introducir una relación inválida → error antes de escribir (RELATION_CONSTRAINT_VIOLATION).
// ===========================================================================
fn escenario_08_relacion_invalida() {
    let dir = bundle_relaciones(""); // tarea.md sin depends_on todavía.
    let antes = snapshot_md(dir.path());

    // add_relation depends_on de la tarea hacia `nota.md` (tipo note), pero depends_on solo admite
    // `component` ⇒ RELATION_CONSTRAINT_VIOLATION, antes de tocar disco.
    let ops = json!([
        { "op": "add_relation", "source": "tarea.md", "relation": "depends_on", "target": "nota.md" },
    ]);
    let resp = roundtrip(
        dir.path(),
        &[change_plan_line(1, ops, policy_permisiva())],
        1,
    );
    assert!(
        es_error_con(&resp[0], "RELATION_CONSTRAINT_VIOLATION"),
        "una relación con target de tipo no admitido debe dar RELATION_CONSTRAINT_VIOLATION: {resp:?}"
    );
    assert_eq!(
        antes,
        snapshot_md(dir.path()),
        "el error de relación inválida debe ocurrir ANTES de escribir"
    );
}

// ===========================================================================
// Escenario 9 — Corregir safe fixes → operaciones apply_fix.
// ===========================================================================
fn escenario_09_safe_fixes() {
    // tarea.md declara depends_on hacia un target INEXISTENTE ⇒ REL-TARGET con un `Fix { safe }`.
    let dir = bundle_relaciones("depends_on:\n  - inexistente.md\n");

    // (1) knowledge_check con fixes sugeridos: localiza el diagnóstico REL-TARGET y su fixId.
    let check = roundtrip(
        dir.path(),
        &[call(
            1,
            "knowledge_check",
            json!({ "scope": { "kind": "workspace" }, "includeSuggestedFixes": true }),
        )],
        1,
    );
    let diags = sc(&check[0])["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("knowledge_check debe devolver diagnostics: {check:?}"));
    let rel_target = diags
        .iter()
        .find(|d| d["code"] == "REL-TARGET")
        .unwrap_or_else(|| {
            panic!("debe haber un diagnóstico REL-TARGET por la relación rota: {check:?}")
        });
    let fix = rel_target["fixes"]
        .as_array()
        .and_then(|f| f.first())
        .unwrap_or_else(|| panic!("el REL-TARGET debe traer un fix sugerido: {rel_target:?}"));
    assert_eq!(
        fix["safe"],
        Value::Bool(true),
        "el fix sugerido para REL-TARGET debe ser safe: {fix:?}"
    );
    let fix_id = fix["fixId"]
        .as_str()
        .unwrap_or_else(|| panic!("el fix debe llevar un fixId: {fix:?}"))
        .to_string();

    // (2) change_plan con una operación `apply_fix` sobre ese fixId: produce un plan real cuyo
    //     resultado RESUELVE el REL-TARGET (semanticDiff.diagnosticsResolved lo recoge).
    let ops = json!([ { "op": "apply_fix", "fixId": fix_id } ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(2, ops, policy_permisiva())],
        1,
    );
    let s = sc(&plan[0]);
    assert!(
        s["normalizedOperations"]
            .as_array()
            .is_some_and(|o| !o.is_empty()),
        "apply_fix debe producir >=1 operación normalizada: {plan:?}"
    );
    let resueltos = s["semanticDiff"]["diagnosticsResolved"]
        .as_array()
        .unwrap_or_else(|| panic!("el plan debe traer semanticDiff.diagnosticsResolved: {plan:?}"));
    assert!(
        resueltos.iter().any(|d| d["code"] == "REL-TARGET"),
        "el safe fix debe RESOLVER el diagnóstico REL-TARGET: {plan:?}"
    );
}

// ===========================================================================
// Escenario 10 — Revisar un refactor → diff semántico en change_plan.
// ===========================================================================
fn escenario_10_diff_refactor() {
    let dir = bundle_cinco_relacionados();
    // Un refactor de a.md: cambia el frontmatter Y el cuerpo.
    let ops = json!([
        { "op": "patch_frontmatter", "ref": { "path": "a.md" }, "patch": { "description": "refactor" } },
        { "op": "replace_body", "ref": { "path": "a.md" }, "body": "# A\n\ncuerpo refactorizado por completo\n" },
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(1, ops, policy_permisiva())],
        1,
    );
    let diff = &sc(&plan[0])["semanticDiff"];

    let modified: Vec<&str> = diff["modified"]
        .as_array()
        .unwrap_or_else(|| panic!("semanticDiff debe traer `modified`: {plan:?}"))
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        modified.contains(&"a.md"),
        "el diff semántico del refactor debe marcar a.md como modificado: {plan:?}"
    );

    let toca_cuerpo = diff["bodyChanges"]
        .as_array()
        .is_some_and(|a| a.iter().any(|v| v == "a.md"));
    let toca_fm = diff["frontmatterChanges"]
        .as_array()
        .is_some_and(|a| a.iter().any(|v| v == "a.md"));
    assert!(
        toca_cuerpo && toca_fm,
        "el diff semántico debe distinguir el cambio de cuerpo y de frontmatter de a.md: {plan:?}"
    );
}

// ===========================================================================
// Escenario 11 — Recuperar un cambio reciente → change_revert.
// ===========================================================================
fn escenario_11_revert() {
    let dir = bundle_min();
    let ops = json!([
        { "op": "create", "path": "nuevo.md", "type": "Nota", "title": "Nuevo",
          "body": "# Resumen\n\ncuerpo del concepto nuevo\n" },
    ]);
    // Plan → apply (captura receiptId + revisión previa).
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(1, ops, policy_permisiva())],
        1,
    );
    let id = plan_id(&plan[0]);
    let applied = roundtrip(dir.path(), &[change_apply_line(2, &id)], 1);
    let receipt = sc(&applied[0])["receiptId"]
        .as_str()
        .unwrap_or_else(|| panic!("change_apply debe devolver receiptId: {applied:?}"))
        .to_string();
    let previa = sc(&applied[0])["previousWorkspaceRevision"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert!(
        dir.path().join("nuevo.md").is_file(),
        "precondición: el apply debe crear nuevo.md: {applied:?}"
    );

    // Revert → el workspace vuelve a la revisión previa y el .md desaparece.
    let reverted = roundtrip(dir.path(), &[change_revert_line(3, &receipt)], 1);
    assert_eq!(
        sc(&reverted[0])["reverted"],
        Value::Bool(true),
        "un receipt reciente debe revertirse (reverted:true): {reverted:?}"
    );
    assert_eq!(
        sc(&reverted[0])["workspaceRevision"].as_str().unwrap_or(""),
        previa,
        "revertir debe devolver el workspace a la previousWorkspaceRevision del apply: {reverted:?}"
    );
    assert!(
        !dir.path().join("nuevo.md").exists(),
        "revertir un create debe borrar el .md: {reverted:?}"
    );
}

// ===========================================================================
// Escenario 12 — Cerrar Lodestar durante publicación → recuperación determinista.
//
// La prueba autoritativa de crash A MITAD es `recovery_sin_parciales` (E13-H06, otro crate + feature
// `test-failpoints`, no invocable desde aquí). Este escenario COMPLEMENTA con durabilidad
// determinista tras reabrir: una publicación sellada sobrevive a cerrar/reabrir el servidor sin
// estado parcial.
// ===========================================================================
fn escenario_12_crash_recuperacion() {
    let dir = bundle_min();
    let ops = json!([
        { "op": "create", "path": "nuevo.md", "type": "Nota", "title": "Nuevo",
          "body": "# Resumen\n\ncuerpo publicado\n" },
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(1, ops, policy_permisiva())],
        1,
    );
    let id = plan_id(&plan[0]);
    let applied = roundtrip(dir.path(), &[change_apply_line(2, &id)], 1);
    let rev_resultante = sc(&applied[0])["workspaceRevision"]
        .as_str()
        .unwrap_or_else(|| panic!("change_apply debe devolver workspaceRevision: {applied:?}"))
        .to_string();

    // "Cerrar Lodestar": el proceso del apply ya terminó (child.wait). Reabrir un servidor FRESCO y
    // comprobar estado DETERMINISTA:
    //   (a) workspace_status reporta EXACTAMENTE la revisión resultante (nada se perdió a medias);
    //   (b) el .md publicado persiste íntegro;
    //   (c) el workspace queda conforme (sin diagnósticos parciales/corruptos).
    let post = roundtrip(
        dir.path(),
        &[
            call(3, "workspace_status", json!({})),
            call(
                4,
                "knowledge_check",
                json!({ "scope": { "kind": "workspace" } }),
            ),
        ],
        2,
    );
    assert_eq!(
        sc(&post[0])["workspaceRevision"].as_str().unwrap_or(""),
        rev_resultante,
        "tras reabrir, workspace_status debe reportar la revisión resultante (durable/determinista): {post:?}"
    );
    let contenido = std::fs::read_to_string(dir.path().join("nuevo.md")).unwrap();
    assert!(
        contenido.contains("cuerpo publicado"),
        "el .md publicado debe persistir íntegro tras reabrir: {contenido:?}"
    );
    assert_eq!(
        sc(&post[1])["conformant"],
        Value::Bool(true),
        "el workspace recuperado debe quedar conforme (sin parciales): {post:?}"
    );
}

// ===========================================================================
// Escenario 13 — Intentar escribir fuera de writableRoots → rechazo (PERMISSION_DENIED).
// ===========================================================================
fn escenario_13_fuera_writable() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "index.md", INDEX);
    write(
        dir.path(),
        "knowledge/concepto.md",
        "---\ntype: Concept\ntitle: Concepto\ndescription: dentro de knowledge\n---\n\n# H\n\ncuerpo\n",
    );
    write(dir.path(), "src/existente.rs", "fn main() {}\n");
    write(
        dir.path(),
        ".lodestar/config.yaml",
        "workspace:\n  writableRoots: [knowledge]\n  referenceRoots: [src]\n",
    );

    // Plan de un create bajo src/ (fuera de writableRoots): change_plan no valida writable, así que
    // produce el plan; el rechazo recae en change_apply (único escritor, assert_writable).
    let ops = json!([
        { "op": "create", "path": "src/malicioso.md", "type": "Nota", "title": "Malo",
          "body": "# Malo\n\nfuera de writableRoots\n" },
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(1, ops, policy_permisiva())],
        1,
    );
    let id = plan_id(&plan[0]);

    let applied = roundtrip(dir.path(), &[change_apply_line(2, &id)], 1);
    assert!(
        es_error_con(&applied[0], "PERMISSION_DENIED"),
        "escribir fuera de writableRoots debe dar PERMISSION_DENIED: {applied:?}"
    );
    assert!(
        !dir.path().join("src/malicioso.md").exists(),
        "el apply rechazado NO debe crear nada bajo src/: {applied:?}"
    );
}

// ===========================================================================
// Escenario 14 — Referenciar un archivo de código inexistente → diagnóstico.
// (La superficie e2e es knowledge_get(externalReferences): exists:false por la ref rota.)
// ===========================================================================
fn escenario_14_ref_codigo_inexistente() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "index.md", INDEX);
    write(dir.path(), "src/existe.rs", "fn main() {}\n");
    write(
        dir.path(),
        ".lodestar/config.yaml",
        "workspace:\n  writableRoots: [knowledge]\n  referenceRoots: [src]\n",
    );
    // Un concepto con dos referencias de código: una que existe y una que NO.
    write(
        dir.path(),
        "knowledge/tarea.md",
        "---\ntype: Concept\ntitle: Tarea\ndescription: con refs de codigo\nimplemented_by:\n  - src/existe.rs\n  - src/inexistente.rs\n---\n\n# Tarea\n\ncuerpo\n",
    );

    let resp = roundtrip(
        dir.path(),
        &[call(
            1,
            "knowledge_get",
            json!({ "ref": { "path": "knowledge/tarea.md" }, "include": ["externalReferences"] }),
        )],
        1,
    );
    let refs = sc(&resp[0])["concept"]["externalReferences"]
        .as_array()
        .unwrap_or_else(|| panic!("knowledge_get debe devolver externalReferences: {resp:?}"));

    let inexistente = refs
        .iter()
        .find(|r| r["path"] == "src/inexistente.rs")
        .unwrap_or_else(|| panic!("debe listar la ref rota src/inexistente.rs: {resp:?}"));
    assert_eq!(
        inexistente["exists"],
        Value::Bool(false),
        "una ref a un archivo de código inexistente debe marcarse exists:false (diagnóstico): {resp:?}"
    );
    // No vacuo: la ref que SÍ existe se marca exists:true.
    let existe = refs
        .iter()
        .find(|r| r["path"] == "src/existe.rs")
        .unwrap_or_else(|| panic!("debe listar la ref existente src/existe.rs: {resp:?}"));
    assert_eq!(
        existe["exists"],
        Value::Bool(true),
        "una ref a un archivo de código existente debe marcarse exists:true: {resp:?}"
    );
}

// ===========================================================================
// Escenario 15 — Editar directamente un Markdown inválido → detectado por knowledge_check.
// (El gate de CI vía `lodestar check` (CLI) lo cubre `check_caza_edicion_directa`, E14-H01.)
// ===========================================================================
fn escenario_15_editar_markdown_invalido() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Editado](editado-a-mano.md)\n",
    );
    // RECOMPUESTO en E16-H05: el escenario se apoyaba en `OKF-TYPE` (frontmatter sin `type`), y
    // ese código se retiró — un `.md` sin `type` es un documento de primera clase. El escenario
    // §17 sigue siendo el mismo («alguien editó el Markdown a mano y lo dejó inválido; el motor
    // lo caza»), pero con el catálogo mínimo de `§20.9`: aquí el frontmatter está delimitado y su
    // YAML es sintácticamente inválido ⇒ `FM-YAML-INVALID` (hard-fail), que es exactamente lo que
    // impide a Lodestar interpretar y modificar el documento con seguridad.
    write(
        dir.path(),
        "editado-a-mano.md",
        "---\ntitle: : :\n  - a pelo\ndescription: a pelo\n---\n\n# Nota\n\ncuerpo.\n",
    );

    let resp = roundtrip(
        dir.path(),
        &[call(
            1,
            "knowledge_check",
            json!({ "scope": { "kind": "workspace" } }),
        )],
        1,
    );
    let diags = sc(&resp[0])["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("knowledge_check debe devolver diagnostics: {resp:?}"));
    let del_fichero: Vec<&Value> = diags
        .iter()
        .filter(|d| {
            d["targets"]
                .as_array()
                .is_some_and(|t| t.iter().any(|v| v == "editado-a-mano.md"))
        })
        .collect();
    assert!(
        del_fichero.iter().any(|d| d["code"] == "FM-YAML-INVALID"),
        "knowledge_check debe cazar el Markdown editado a mano con FM-YAML-INVALID: {resp:?}"
    );
    // Y el diagnóstico acota el bloque: `§20.9` exige rango para `FM-YAML-INVALID`, y aquí son
    // las líneas 2..4 (1-based, delimitadores excluidos).
    let con_rango = del_fichero
        .iter()
        .find(|d| d["code"] == "FM-YAML-INVALID")
        .expect("ya comprobado arriba");
    assert_eq!(
        con_rango["range"],
        json!({ "startLine": 2, "endLine": 4 }),
        "el diagnóstico de frontmatter ilegible debe acotar las líneas del bloque: {resp:?}"
    );
    assert_eq!(
        sc(&resp[0])["conformant"],
        Value::Bool(false),
        "un frontmatter inválido debe dejar el workspace NO conforme: {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// Un #[test] por fila (diagnóstico granular).
// ---------------------------------------------------------------------------

#[test]
fn bench_01_buscar_por_significado() {
    escenario_01_buscar_por_significado();
}
#[test]
fn bench_02_crear_valido() {
    escenario_02_crear_valido();
}
#[test]
fn bench_03_crear_sin_campo_obligatorio() {
    escenario_03_crear_sin_campo_obligatorio();
}
#[test]
fn bench_04_mover_30_backlinks() {
    escenario_04_mover_30_backlinks();
}
#[test]
fn bench_05_borrar_referenciado() {
    escenario_05_borrar_referenciado();
}
#[test]
fn bench_06_conflicto_revision() {
    escenario_06_conflicto_revision();
}
#[test]
fn bench_07_cinco_conceptos() {
    escenario_07_cinco_conceptos();
}
#[test]
fn bench_08_relacion_invalida() {
    escenario_08_relacion_invalida();
}
#[test]
fn bench_09_safe_fixes() {
    escenario_09_safe_fixes();
}
#[test]
fn bench_10_diff_refactor() {
    escenario_10_diff_refactor();
}
#[test]
fn bench_11_revert() {
    escenario_11_revert();
}
#[test]
fn bench_12_crash_recuperacion() {
    escenario_12_crash_recuperacion();
}
#[test]
fn bench_13_fuera_writable() {
    escenario_13_fuera_writable();
}
#[test]
fn bench_14_ref_codigo_inexistente() {
    escenario_14_ref_codigo_inexistente();
}
#[test]
fn bench_15_editar_markdown_invalido() {
    escenario_15_editar_markdown_invalido();
}

// ---------------------------------------------------------------------------
// E14-H04 · Criterio `benchmark_15_escenarios`: las 15 filas de §17 en un solo viaje e2e.
// Es el test que nombra la spec; ejerce los 15 escenarios en secuencia sobre la superficie real.
// ---------------------------------------------------------------------------
#[test]
fn benchmark_15_escenarios() {
    escenario_01_buscar_por_significado();
    escenario_02_crear_valido();
    escenario_03_crear_sin_campo_obligatorio();
    escenario_04_mover_30_backlinks();
    escenario_05_borrar_referenciado();
    escenario_06_conflicto_revision();
    escenario_07_cinco_conceptos();
    escenario_08_relacion_invalida();
    escenario_09_safe_fixes();
    escenario_10_diff_refactor();
    escenario_11_revert();
    escenario_12_crash_recuperacion();
    escenario_13_fuera_writable();
    escenario_14_ref_codigo_inexistente();
    escenario_15_editar_markdown_invalido();
}
