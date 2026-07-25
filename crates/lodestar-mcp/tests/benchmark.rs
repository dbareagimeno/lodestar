//! E14-H04 — Benchmark funcional (`REFACTOR §17`) como suite e2e.
//!
//! Los 15 escenarios de producto de la tabla `REFACTOR §17` (líneas 1436-1452), compuestos como
//! **viaje e2e** que cruza la superficie real: el servidor MCP por stdio (10 tools, E10–E13). NO se
//! llaman funciones internas del `App`/`Workspace`: cada escenario arranca el binario
//! `lodestar-mcp`, le habla JSON-RPC y asevera sobre las respuestas y el disco. La mayoría de los
//! mecanismos ya existen (las tools se cerraron en E10-H09…E13-H09, dependencia E13-H09), así que
//! esta historia es de **composición/regresión e2e**: verifica que el conjunto de las 10 tools cubre
//! los escenarios de producto de punta a punta sobre un workspace de benchmark realista.
//!
//! ## Códigos de error REALES (los que emite el motor HOY, no los idealizados de §17)
//! El catálogo `ErrorCode` (`lodestar-core::types`, invariante #4) está congelado en 16 variantes;
//! cada escenario asevera el código estable que el motor emite de verdad (verificado en
//! `crates/lodestar-app/src/lib.rs` `error_code`/`workspace_error_code` y `types.rs`):
//!   - Escenario 3 (crear un documento NO conforme): §17 dice «Plan rechazado». RECOMPUESTO en
//!     E20-H03 con un código vivo (`LINK-TARGET-MISSING` por un enlace roto, ya que `SCHEMA-REQFIELD`
//!     se retiró). El motor lo materializa en DOS superficies: `change_plan` devuelve `canApply:false`
//!     con `diagnosticsAfter.errors>=1`, y `change_apply` lo rechaza en el staging con
//!     **`NONCONFORMANT_RESULT`** (E14-H04). Se aseveran ambas.
//!   - Escenario 5 (borrar referenciado): §17 dice «Rechazo con blockers». El motor emite
//!     **`INBOUND_LINKS_EXIST`** al normalizar un `delete` con política `Reject` (los enlaces
//!     entrantes SON los blockers).
//!   - Escenario 6 (modificar cambiado externamente): §17 dice `REVISION_CONFLICT` y el motor emite
//!     exactamente **`REVISION_CONFLICT`** (control optimista por op en `change_plan`). Sin
//!     divergencia.
//!   - Escenario 8 (relación inválida): RETIRADO en E20-H03 (relaciones tipadas eliminadas con
//!     `core::schema`; una relación es un enlace, sin restricción de tipo).
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
//! Cada escenario es una función `escenario_NN_*()` autocontenida (su propio workspace temporal + sus
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
// Workspaces de benchmark.
// ---------------------------------------------------------------------------

const INDEX: &str = "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n";

/// Workspace mínimo (solo `index.md`).
fn workspace_min() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "index.md", INDEX);
    dir
}

// (E20-H03: los fixtures `workspace_schema_decision` y `workspace_relaciones`, que escribían un
// `.lodestar/schema.yaml` con tipos/relaciones tipadas, se retiran con la maquinaria de schema.)
/// Workspace con 4 documentos relacionados en anillo (`a`/`b`/`c`/`d`), conformes (escenario 7).
fn workspace_cinco_relacionados() -> tempfile::TempDir {
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
        { "op": "create", "path": "nuevo.md", "body": "# Nuevo\n\ncuerpo del quinto documento\n" },
        { "op": "patch_frontmatter", "ref": { "path": "a.md" }, "patch": { "description": "a v2" } },
        { "op": "patch_frontmatter", "ref": { "path": "b.md" }, "patch": { "description": "b v2" } },
        { "op": "patch_frontmatter", "ref": { "path": "c.md" }, "patch": { "description": "c v2" } },
        { "op": "patch_frontmatter", "ref": { "path": "d.md" }, "patch": { "description": "d v2" } },
    ])
}

/// Workspace con `target.md` referenciado por EXACTAMENTE 30 emisores de cuerpo (escenario 4).
fn workspace_treinta_backlinks() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "index.md", INDEX);
    write(
        dir.path(),
        "target.md",
        "---\ntype: Concept\ntitle: Target\ndescription: el documento a mover\n---\n\n# Target\n\ncuerpo\n",
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
        "---\ntype: document\ntitle: Bicicletas\ndescription: sobre ruedas\n---\n\n# H\n\nnada que ver con el tema.\n",
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
    let document = &sc(&get[0])["document"];
    assert!(
        document["revision"]
            .as_str()
            .unwrap_or("")
            .starts_with("blake3:"),
        "knowledge_get debe traer revision «blake3:…»: {get:?}"
    );
    assert!(
        document["frontmatter"].is_object(),
        "knowledge_get debe traer el frontmatter: {get:?}"
    );
    assert!(
        document["body"]
            .as_str()
            .unwrap_or("")
            .contains("tokens rotatorios"),
        "knowledge_get debe traer el cuerpo de la decisión: {get:?}"
    );
}

// ===========================================================================
// Escenario 2 — Crear un documento válido → plan aceptado y aplicado.
// ===========================================================================
fn escenario_02_crear_valido() {
    let dir = workspace_min();
    let ops = json!([
        { "op": "create", "path": "nuevo.md", "body": "# Resumen\n\ncuerpo del documento nuevo\n" },
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
// Escenario 3 — Crear un documento NO conforme → plan rechazado (RECOMPUESTO E20-H03).
//
// El escenario §17 sigue siendo el mismo («un create que deja el workspace no conforme NUNCA acaba
// publicado»), pero con un código VIVO de `§20.9` en vez del retirado `SCHEMA-REQFIELD`: el nuevo
// documento lleva un enlace a un `.md` inexistente ⇒ `LINK-TARGET-MISSING` (Err) ⇒ resultado no
// conforme. Dos superficies deben rechazarlo:
//   (1) change_plan: canApply:false + diagnosticsAfter.errors>=1 (usa `plan::validate_result`).
//   (2) change_apply: NONCONFORMANT_RESULT y no escribe (gate de `validate_staging`, E14-H04).
// ===========================================================================
fn escenario_03_crear_no_conforme() {
    let dir = workspace_min();
    // El create añade un documento con un enlace a un `.md` que no existe ⇒ LINK-TARGET-MISSING (Err)
    // ⇒ resultado no conforme.
    let ops = json!([
        { "op": "create", "path": "dec.md", "body": "# No conforme\n\n[roto](no-existe.md)\n" },
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
        "un create que deja el workspace no conforme NO debe ser aplicable (canApply:false): {plan:?}"
    );
    assert!(
        sc(&plan[0])["diagnosticsAfter"]["errors"]
            .as_u64()
            .unwrap_or(0)
            >= 1,
        "el plan debe reportar >=1 error de conformidad en diagnosticsAfter: {plan:?}"
    );
    let id = plan_id(&plan[0]);

    // (2) change_apply DEBE rechazar el plan no conforme: un resultado no conforme no puede acabar
    //     escrito en el canónico (gate de `validate_staging`, invariante #3).
    let applied = roundtrip(dir.path(), &[change_apply_line(2, &id)], 1);
    assert!(
        es_error_con(&applied[0], "NONCONFORMANT_RESULT"),
        "change_apply debe rechazar un create no conforme con NONCONFORMANT_RESULT: {applied:?}"
    );
    assert!(
        !dir.path().join("dec.md").exists(),
        "un create no conforme NO debe materializar el .md: {applied:?}"
    );
}

// ===========================================================================
// Escenario 4 — Mover un documento con 30 backlinks → enlaces actualizados dentro del mismo plan.
// ===========================================================================
fn escenario_04_mover_30_backlinks() {
    let dir = workspace_treinta_backlinks();
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

    // El plan lleva el Move MÁS las 30 reescrituras de enlaces entrantes, todo en UN change set.
    //
    // E23-H03 relaja el CONTEO exacto (era `== 31`) sin aflojar lo que este escenario mide: desde
    // esa historia `normalize_move` reescribe también el CUERPO DEL PROPIO DOCUMENTO MOVIDO, y aquí
    // `target.md` no tiene salientes, así que emitir esa op (32) o ahorrársela (31) son ambas
    // implementaciones defendibles. Lo que sí se exige, y con más precisión que un conteo: 1 `Move`
    // y exactamente 30 reescrituras de emisores, todo bajo un único changeSetId.
    let moves = normalized.iter().filter(|op| op["op"] == "move").count();
    assert_eq!(
        moves, 1,
        "el plan debe llevar exactamente un `move`: {plan:?}"
    );
    let emisores = normalized
        .iter()
        .filter(|op| {
            op["op"] == "replace_body"
                && op["path"].as_str().is_some_and(|p| p.starts_with("emisor"))
        })
        .count();
    assert_eq!(
        emisores, 30,
        "mover con 30 backlinks debe reescribir los 30 emisores en el mismo plan: {plan:?}"
    );
    assert!(
        normalized.len() <= 32,
        "el plan no debe llevar ops de más (a lo sumo: 1 move + 30 emisores + el documento \
         movido): {plan:?}"
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
// Escenario 5 — Borrar un documento referenciado → rechazo con blockers (INBOUND_LINKS_EXIST).
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
        "---\ntype: document\ntitle: Objetivo\ndescription: referenciado por 3\n---\n\n# Objetivo\n\ncuerpo\n",
    );
    for slug in ["a", "b", "c"] {
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!(
                "---\ntype: document\ntitle: {slug}\ndescription: enlaza al objetivo\n---\n\n# {slug}\n\n[Objetivo](objetivo.md)\n"
            ),
        );
    }

    // delete con política `reject` EXPLÍCITA (E21-H03, §Fase 12: un delete sin política y con
    // backlinks es INVALID_SCHEMA, no un `reject` en silencio): los 3 entrantes son blockers ⇒
    // INBOUND_LINKS_EXIST.
    let ops = json!([ { "op": "delete", "ref": { "path": "objetivo.md" }, "inboundLinksPolicy": "reject" } ]);
    let resp = roundtrip(
        dir.path(),
        &[change_plan_line(1, ops, policy_permisiva())],
        1,
    );
    assert!(
        es_error_con(&resp[0], "INBOUND_LINKS_EXIST"),
        "borrar un documento referenciado debe rechazarse con INBOUND_LINKS_EXIST: {resp:?}"
    );
    assert!(
        dir.path().join("objetivo.md").is_file(),
        "un delete rechazado NO debe borrar el .md: {resp:?}"
    );
}

// ===========================================================================
// Escenario 6 — Modificar un documento cambiado externamente → REVISION_CONFLICT.
// ===========================================================================
fn escenario_06_conflicto_revision() {
    let dir = workspace_cinco_relacionados();

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
    let old_rev = sc(&get[0])["document"]["revision"]
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
// Escenario 7 — Cambiar cinco documentos relacionados → un único change set.
// ===========================================================================
fn escenario_07_cinco_documentos() {
    let dir = workspace_cinco_relacionados();
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
// Escenarios 8 y 9 — RETIRADOS en E20-H03.
//   · 8 (relación inválida → RELATION_CONSTRAINT_VIOLATION): las relaciones tipadas y su validación
//     desaparecen con `core::schema` (`§20.10`: una relación es un enlace, sin restricción de tipo).
//   · 9 (safe fixes de REL-TARGET): el diagnóstico `REL-TARGET` y su `Fix{safe}` mueren con
//     `validate_relations`; ya no hay fixes que aplicar. Ambos ejercitaban capacidades que E20
//     elimina, no un hueco por cubrir.
// ===========================================================================

// ===========================================================================
// Escenario 10 — Revisar un refactor → diff semántico en change_plan.
// ===========================================================================
fn escenario_10_diff_refactor() {
    let dir = workspace_cinco_relacionados();
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
    let dir = workspace_min();
    let ops = json!([
        { "op": "create", "path": "nuevo.md", "body": "# Resumen\n\ncuerpo del documento nuevo\n" },
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
    let dir = workspace_min();
    let ops = json!([
        { "op": "create", "path": "nuevo.md", "body": "# Resumen\n\ncuerpo publicado\n" },
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
        "knowledge/documento.md",
        "---\ntype: Concept\ntitle: Documento\ndescription: dentro de knowledge\n---\n\n# H\n\ncuerpo\n",
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
        { "op": "create", "path": "src/malicioso.md", "body": "# Malo\n\nfuera de writableRoots\n" },
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
    // Un documento con dos referencias de código: una que existe y una que NO.
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
    let refs = sc(&resp[0])["document"]["externalReferences"]
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
fn bench_03_crear_no_conforme() {
    escenario_03_crear_no_conforme();
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
fn bench_07_cinco_documentos() {
    escenario_07_cinco_documentos();
}
// bench_08_relacion_invalida / bench_09_safe_fixes: RETIRADOS en E20-H03 (capacidades eliminadas).
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
// E14-H04 · Criterio `benchmark_escenarios`: las filas de §17 en un solo viaje e2e.
// Es el test que nombra la spec; ejerce los escenarios en secuencia sobre la superficie real. En
// E20-H03 quedan 13 (los escenarios 8 y 9 —relación tipada inválida y safe fixes de REL-TARGET— se
// retiraron con `core::schema`).
// ---------------------------------------------------------------------------
// ===========================================================================
// E23-H03 — `move` recalcula sus PROPIOS enlaces salientes, APLICANDO A DISCO
// (`ARCHITECTURE.md §20.11`, `requirements/epica-23-cierre-migracion.md`). Fase ROJA.
//
// El defecto existe precisamente porque `escenario_04_mover_30_backlinks` **solo planifica** y
// asevera que el disco NO cambió: la mitad saliente del move nunca se ejerció de punta a punta.
// Estos tres cierran ese hueco por la superficie real (JSON-RPC → binario `lodestar-mcp`):
// planifican, **aplican**, leen los `.md` publicados y revierten.
//
// Síntoma reproducido con los binarios: con `notas/alfa.md` que contiene `[b]: beta.md` y
// `notas/beta.md` existente, planificar el move a `archivo/alfa.md` da `canApply:false` con
// `diagnosticsAfter.errors:1` y el apply falla con `NONCONFORMANT_RESULT` — el documento movido se
// lleva sus hrefs relativos escritos para la ubicación VIEJA.
//
// ROJO esperado HOY: por ASERCIÓN (`canApply` es `false`, así que el test se para en el plan).
// ===========================================================================

/// `notas/alfa.md`: el documento a mover. Enlaza a su vecina `notas/beta.md` de las DOS formas
/// (inline + definición de referencia) y lleva además los tres destinos que **no** dependen de
/// dónde viva el documento: una URI externa, un anchor propio y un href raíz-absoluto.
const ALFA_CON_SALIENTES: &str = "---\ntitle: Alfa\n---\n\n# Alfa\n\n## seccion\n\n\
     Inline: [Beta](beta.md).\n\n\
     Referencia: [beta][b].\n\n\
     Externo: [Sitio](https://example.com/notas/beta.md).\n\n\
     Ancla: [Sección](#seccion).\n\n\
     Raíz: [Raíz](/raiz.md).\n\n\
     [b]: beta.md\n";

/// Workspace del defecto: el documento con salientes, su vecina y el destino raíz-absoluto.
fn workspace_move_salientes() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "notas/alfa.md", ALFA_CON_SALIENTES);
    write(
        dir.path(),
        "notas/beta.md",
        "---\ntitle: Beta\n---\n\n# Beta\n\ncuerpo\n",
    );
    write(dir.path(), "raiz.md", "---\ntitle: Raíz\n---\n\n# Raíz\n");
    dir
}

/// Las operaciones del move que fija la historia.
fn ops_move_alfa() -> Value {
    json!([
        { "op": "move", "from": "notas/alfa.md", "to": "archivo/alfa.md",
          "rewriteInboundLinks": true },
    ])
}

/// Planifica `ops` bajo `policy` y devuelve `(changeSetId, respuesta)`, exigiendo `canApply:true`.
/// Es donde muere hoy el rojo de E23-H03: el mensaje incluye el plan entero (con
/// `diagnosticsAfter`), que es el diagnóstico útil.
fn planifica_aplicable(dir: &std::path::Path, ops: Value, policy: Value) -> (String, Value) {
    let plan = roundtrip(dir, &[change_plan_line(1, ops, policy)], 1);
    assert_eq!(
        sc(&plan[0])["canApply"],
        Value::Bool(true),
        "el plan debe ser aplicable (canApply:true): {plan:?}"
    );
    (plan_id(&plan[0]), plan[0].clone())
}

/// Aplica `id` exigiendo `applied:true` y devuelve el `receiptId`.
fn aplica(dir: &std::path::Path, id: &str) -> String {
    let applied = roundtrip(dir, &[change_apply_line(2, id)], 1);
    assert_eq!(
        sc(&applied[0])["applied"],
        Value::Bool(true),
        "el plan debe aplicarse (applied:true): {applied:?}"
    );
    sc(&applied[0])["receiptId"]
        .as_str()
        .unwrap_or_else(|| panic!("change_apply debe devolver receiptId: {applied:?}"))
        .to_string()
}

/// `true` si `knowledge_check` (scope workspace) da el workspace por conforme — el equivalente e2e
/// de «`lodestar check` sale 0» dentro de este binario de test.
fn workspace_conforme(dir: &std::path::Path) -> (bool, Value) {
    let resp = roundtrip(
        dir,
        &[call(
            9,
            "knowledge_check",
            json!({ "scope": { "kind": "workspace" } }),
        )],
        1,
    );
    (
        sc(&resp[0])["conformant"] == Value::Bool(true),
        resp[0].clone(),
    )
}

/// Lee un `.md` publicado (falla con un mensaje claro si no existe).
fn lee(dir: &std::path::Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel))
        .unwrap_or_else(|e| panic!("«{rel}» debe existir en disco tras el apply: {e}"))
}

/// Criterio `move_recalcula_salientes` — **Dado** `notas/alfa.md` con un enlace inline y una
/// definición de referencia a `notas/beta.md`, **Cuando** se mueve a `archivo/alfa.md` y se
/// **aplica**, **Entonces** ambos hrefs quedan `../notas/beta.md` y el workspace queda conforme
/// (`lodestar check` sale 0).
///
/// Y el criterio `move_no_toca_externos_ni_anchors` en su mitad e2e: la URI externa, el anchor
/// propio y el href raíz-absoluto sobreviven **byte a byte** al viaje completo (su versión pura
/// vive en `crates/lodestar-core/tests/core.rs`).
#[test]
fn move_recalcula_salientes() {
    let dir = workspace_move_salientes();

    // Política ESTRICTA a propósito: el criterio es que mover una nota que enlaza a sus vecinas
    // deja el workspace consistente, así que el plan tiene que ser conforme por sí mismo.
    let (id, _plan) = planifica_aplicable(dir.path(), ops_move_alfa(), policy_estricta());
    aplica(dir.path(), &id);

    let raw = lee(dir.path(), "archivo/alfa.md");
    assert!(
        !dir.path().join("notas/alfa.md").exists(),
        "el documento movido no puede seguir en su ubicación vieja; disco =\n{raw}"
    );

    // 1) Los dos salientes relativos, recalculados desde `archivo/`.
    assert!(
        raw.contains("[Beta](../notas/beta.md)"),
        "el enlace inline saliente debe quedar `../notas/beta.md` tras el move; \
         `archivo/alfa.md` =\n{raw}"
    );
    assert!(
        raw.contains("[b]: ../notas/beta.md"),
        "la definición de referencia saliente debe quedar `[b]: ../notas/beta.md` tras el move; \
         `archivo/alfa.md` =\n{raw}"
    );
    assert!(
        !raw.contains("](beta.md)") && !raw.contains("[b]: beta.md"),
        "ningún saliente puede conservar el destino viejo relativo a `notas/`; \
         `archivo/alfa.md` =\n{raw}"
    );

    // 2) Y lo que NO depende de la ubicación sobrevive byte a byte.
    for intacto in [
        "Externo: [Sitio](https://example.com/notas/beta.md).",
        "Ancla: [Sección](#seccion).",
        "Raíz: [Raíz](/raiz.md).",
    ] {
        assert!(
            raw.contains(intacto),
            "«{intacto}» debe sobrevivir intacto al move; `archivo/alfa.md` =\n{raw}"
        );
    }

    // 3) El veredicto del producto: el workspace queda conforme (equivale a `check` → exit 0).
    let (conforme, resp) = workspace_conforme(dir.path());
    assert!(
        conforme,
        "tras mover una nota que enlaza a sus vecinas el workspace debe quedar conforme \
         (`lodestar check` sale 0): {resp:?}"
    );
}

/// Criterio `move_completo_treinta_backlinks` — **Dado** un documento con 30 backlinks entrantes y
/// enlaces salientes propios, **Cuando** se mueve y se **aplica**, **Entonces** los 30 emisores
/// apuntan al destino nuevo y el documento movido conserva sus salientes válidos, **en una sola
/// transacción**.
///
/// Es la unión de las dos mitades del `move` (entrante, ya cubierta por `escenario_04`, y saliente,
/// el defecto de E23-H03) sobre el mismo change set: un único `changeSetId` y un único `receiptId`.
#[test]
fn move_completo_treinta_backlinks() {
    let dir = workspace_move_salientes();
    for i in 0..30 {
        write(
            dir.path(),
            &format!("emisor{i:02}.md"),
            &format!(
                "---\ntitle: Emisor {i:02}\n---\n\n# Emisor {i:02}\n\nreferencia a [alfa](notas/alfa.md).\n"
            ),
        );
    }

    let (id, plan) = planifica_aplicable(dir.path(), ops_move_alfa(), policy_estricta());
    assert!(
        !id.is_empty(),
        "las 31+ escrituras deben caber en un ÚNICO change set: {plan:?}"
    );

    // El plan lleva la mitad entrante (30 emisores) Y la saliente (el propio documento movido).
    let normalized = sc(&plan)["normalizedOperations"]
        .as_array()
        .unwrap_or_else(|| panic!("change_plan debe devolver normalizedOperations: {plan:?}"))
        .clone();
    let emisores_reescritos = normalized
        .iter()
        .filter(|op| {
            op["op"] == "replace_body"
                && op["path"].as_str().is_some_and(|p| p.starts_with("emisor"))
        })
        .count();
    assert_eq!(
        emisores_reescritos, 30,
        "el plan debe reescribir los 30 emisores entrantes: {plan:?}"
    );
    assert!(
        normalized.iter().any(|op| {
            op["op"] == "replace_body"
                && matches!(
                    op["path"].as_str(),
                    Some("notas/alfa.md") | Some("archivo/alfa.md")
                )
        }),
        "el plan debe incluir además la reescritura del PROPIO documento movido (sus salientes \
         recalculados desde la ubicación nueva): {plan:?}"
    );

    let receipt = aplica(dir.path(), &id);
    assert!(
        !receipt.is_empty(),
        "la transacción única debe devolver un receiptId"
    );

    // 1) Los 30 emisores apuntan al destino nuevo.
    for i in 0..30 {
        let emisor = lee(dir.path(), &format!("emisor{i:02}.md"));
        assert!(
            emisor.contains("[alfa](archivo/alfa.md)"),
            "el emisor {i:02} debe apuntar al destino nuevo `archivo/alfa.md`; cuerpo =\n{emisor}"
        );
        assert!(
            !emisor.contains("notas/alfa.md"),
            "el emisor {i:02} no debe conservar el destino viejo; cuerpo =\n{emisor}"
        );
    }

    // 2) Y el documento movido conserva sus salientes VÁLIDOS desde la ubicación nueva.
    let movido = lee(dir.path(), "archivo/alfa.md");
    assert!(
        movido.contains("[Beta](../notas/beta.md)") && movido.contains("[b]: ../notas/beta.md"),
        "el documento movido debe conservar sus salientes recalculados; \
         `archivo/alfa.md` =\n{movido}"
    );

    let (conforme, resp) = workspace_conforme(dir.path());
    assert!(
        conforme,
        "tras el move completo el workspace debe quedar conforme: {resp:?}"
    );
}

/// Criterio `move_revert_completo` — **Dado** ese move aplicado, **Cuando** se hace
/// `change_revert`, **Entonces** los 32 ficheros vuelven a su contenido previo.
///
/// Se compara el árbol `.md` ENTERO (snapshot antes/después), que es más fuerte que contar
/// ficheros: cualquier byte distinto en cualquier `.md` —incluido el path del documento movido—
/// hace fallar el test. Con una guarda de no vacuidad: entre medias el árbol tiene que haber
/// cambiado de verdad.
#[test]
fn move_revert_completo() {
    let dir = workspace_move_salientes();
    for i in 0..30 {
        write(
            dir.path(),
            &format!("emisor{i:02}.md"),
            &format!(
                "---\ntitle: Emisor {i:02}\n---\n\n# Emisor {i:02}\n\nreferencia a [alfa](notas/alfa.md).\n"
            ),
        );
    }
    let antes = snapshot_md(dir.path());
    assert_eq!(
        antes.len(),
        33,
        "el fixture son 30 emisores + alfa + beta + raiz"
    );

    let (id, _) = planifica_aplicable(dir.path(), ops_move_alfa(), policy_estricta());
    let receipt = aplica(dir.path(), &id);

    let tras_aplicar = snapshot_md(dir.path());
    assert_ne!(
        antes, tras_aplicar,
        "guarda de no vacuidad: el apply del move TIENE que haber cambiado el árbol"
    );

    let reverted = roundtrip(dir.path(), &[change_revert_line(3, &receipt)], 1);
    assert_eq!(
        sc(&reverted[0])["reverted"],
        Value::Bool(true),
        "el receipt del move debe poder revertirse: {reverted:?}"
    );

    let despues = snapshot_md(dir.path());
    assert_eq!(
        despues, antes,
        "revertir el move debe devolver TODOS los `.md` a su contenido previo, byte a byte \
         (incluido el documento movido en su ubicación original y los 30 emisores)"
    );
}

// ===========================================================================
// E23-H05 — Políticas de borrado honestas, POR LA SUPERFICIE MCP
// (`ARCHITECTURE.md §20.11`, `requirements/epica-23-cierre-migracion.md`). Fase ROJA.
//
// `delete_politica_retirada_nombra_validas` es el rojo: hoy `retarget`/`create_stub` se ACEPTAN y
// el plan sale (con el borrado a secas), así que la respuesta no es un error y no nombra nada.
// `delete_remove_links_aplicado` y `delete_revert_byte_a_byte` son los criterios 2 y 3 de la
// historia: la política que SÍ sobrevive tiene que seguir funcionando de punta a punta (aplicar +
// revertir). Son controles anti-regresión de la retirada — si están verdes hoy, su valor es que
// sigan verdes después.
// ===========================================================================

/// Workspace con `objetivo.md` referenciado desde `a.md` y `b.md` (2 backlinks inline).
fn workspace_borrado_con_backlinks() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "objetivo.md",
        "---\ntitle: Objetivo\n---\n\n# Objetivo\n\ncuerpo\n",
    );
    for slug in ["a", "b"] {
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!(
                "---\ntitle: {slug}\n---\n\n# {slug}\n\nApunta a [Objetivo](objetivo.md) y sigue.\n"
            ),
        );
    }
    dir
}

/// Criterio `delete_retarget_rechazado` (mitad de wire; la del `App` vive en
/// `crates/lodestar-app/tests/eliminacion.rs`) — **Dado** un `delete` con
/// `inboundLinksPolicy: "retarget"`, **Cuando** se planifica, **Entonces** se rechaza con
/// `INVALID_SCHEMA` **y un mensaje que nombra las políticas válidas**.
///
/// El mensaje es parte del criterio: el valor de la política es lo que el agente escribió, así que
/// el rechazo tiene que decirle cuáles quedan. Hoy `ErrorCode` viaja a la superficie como código
/// pelado (`e.as_str()` en `tools.rs`), de modo que este test obliga a que el rechazo de un valor
/// retirado lleve además el texto — sin tocar el catálogo congelado de 16 códigos.
#[test]
fn delete_politica_retirada_nombra_validas() {
    let dir = workspace_borrado_con_backlinks();

    for politica in ["retarget", "create_stub"] {
        let resp = roundtrip(
            dir.path(),
            &[change_plan_line(
                1,
                json!([
                    { "op": "delete", "ref": { "path": "objetivo.md" },
                      "inboundLinksPolicy": politica }
                ]),
                policy_permisiva(),
            )],
            1,
        );
        assert!(
            es_error_con(&resp[0], "INVALID_SCHEMA"),
            "`inboundLinksPolicy: {politica}` está RETIRADA (E23-H05) y debe rechazarse con \
             INVALID_SCHEMA, no aceptarse para producir un plan que no ejecuta la política: {resp:?}"
        );
        let texto = resp[0].to_string();
        for valida in ["reject", "remove_links"] {
            assert!(
                texto.contains(valida),
                "el rechazo de «{politica}» debe nombrar las políticas válidas (falta \
                 «{valida}»): {resp:?}"
            );
        }
        assert!(
            dir.path().join("objetivo.md").is_file(),
            "un delete rechazado no puede borrar nada: {resp:?}"
        );
    }

    // Control anti-vacuo: la política viva no se rechaza (el filtro es del VALOR retirado, no del
    // parámetro).
    let ok = roundtrip(
        dir.path(),
        &[change_plan_line(
            2,
            json!([
                { "op": "delete", "ref": { "path": "objetivo.md" },
                  "inboundLinksPolicy": "remove_links" }
            ]),
            policy_permisiva(),
        )],
        1,
    );
    assert!(
        !es_error_con(&ok[0], "INVALID_SCHEMA"),
        "`remove_links` sigue siendo una política válida y debe planificar: {ok:?}"
    );
}

/// Criterio `delete_remove_links_aplicado` — **Dado** un `delete` con
/// `inboundLinksPolicy: "remove_links"` sobre un documento con 2 backlinks, **Cuando** se
/// **aplica**, **Entonces** el `.md` desaparece, los 2 emisores conservan el texto sin el enlace, y
/// `check` sale 0.
#[test]
fn delete_remove_links_aplicado() {
    let dir = workspace_borrado_con_backlinks();

    let (id, _) = planifica_aplicable(
        dir.path(),
        json!([
            { "op": "delete", "ref": { "path": "objetivo.md" },
              "inboundLinksPolicy": "remove_links" }
        ]),
        policy_estricta(),
    );
    aplica(dir.path(), &id);

    assert!(
        !dir.path().join("objetivo.md").exists(),
        "el documento borrado debe desaparecer del disco"
    );
    for slug in ["a", "b"] {
        let emisor = lee(dir.path(), &format!("{slug}.md"));
        assert!(
            !emisor.contains("objetivo.md"),
            "tras `remove_links` el emisor {slug}.md no puede conservar el enlace; cuerpo =\n{emisor}"
        );
        assert!(
            emisor.contains("Apunta a Objetivo y sigue."),
            "`remove_links` desenlaza dejando el TEXTO del enlace en su sitio, sin comerse la \
             frase; cuerpo =\n{emisor}"
        );
    }

    let (conforme, resp) = workspace_conforme(dir.path());
    assert!(
        conforme,
        "tras borrar con `remove_links` el workspace debe quedar conforme (`check` sale 0): {resp:?}"
    );
}

/// Criterio `delete_revert_byte_a_byte` — **Dado** ese borrado aplicado, **Cuando** se hace
/// `change_revert`, **Entonces** el documento vuelve **byte a byte** y los emisores recuperan sus
/// enlaces.
#[test]
fn delete_revert_byte_a_byte() {
    let dir = workspace_borrado_con_backlinks();
    let antes = snapshot_md(dir.path());

    let (id, _) = planifica_aplicable(
        dir.path(),
        json!([
            { "op": "delete", "ref": { "path": "objetivo.md" },
              "inboundLinksPolicy": "remove_links" }
        ]),
        policy_estricta(),
    );
    let receipt = aplica(dir.path(), &id);
    assert_ne!(
        antes,
        snapshot_md(dir.path()),
        "guarda de no vacuidad: el apply del borrado TIENE que haber cambiado el árbol"
    );

    let reverted = roundtrip(dir.path(), &[change_revert_line(3, &receipt)], 1);
    assert_eq!(
        sc(&reverted[0])["reverted"],
        Value::Bool(true),
        "el receipt del borrado debe poder revertirse: {reverted:?}"
    );

    assert_eq!(
        snapshot_md(dir.path()),
        antes,
        "revertir el borrado debe devolver el documento BYTE A BYTE y restaurar los enlaces de los \
         dos emisores"
    );
}

#[test]
fn benchmark_escenarios() {
    escenario_01_buscar_por_significado();
    escenario_02_crear_valido();
    escenario_03_crear_no_conforme();
    escenario_04_mover_30_backlinks();
    escenario_05_borrar_referenciado();
    escenario_06_conflicto_revision();
    escenario_07_cinco_documentos();
    escenario_10_diff_refactor();
    escenario_11_revert();
    escenario_12_crash_recuperacion();
    escenario_13_fuera_writable();
    escenario_14_ref_codigo_inexistente();
    escenario_15_editar_markdown_invalido();
}
