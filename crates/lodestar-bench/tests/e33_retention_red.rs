//! Fase roja de retención E33 (adenda 2026-08-23).
//!
//! Estos guards prueban únicamente el contrato de almacenamiento de evidencia. Los cinco
//! resultados generados no se leen: su forma se comprueba contra un manifiesto versionado y los
//! informes pequeños de formato/gate viven en `tests/fixtures/`.

use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MANIFEST_RELATIVE: &str = "docs/qa/corridas/v0.6.2/manifest.json";
const KNOWN_RAW_SCHEMAS: [&str; 4] = [
    "e33-h04-v2-full",
    "e33-h04-v2",
    "e33-h07-conformidad-v1",
    "e33-h09-v1",
];

const EXPECTED_RESULTS: [(&str, &str, &str); 5] = [
    (
        "e33-h04-rendimiento",
        "docs/qa/e33-h04-banco-rendimiento-2026-08-22.md",
        "e33-h04-corrida-2026-08-22.json",
    ),
    (
        "e33-h06-rendimiento",
        "docs/qa/e33-h06-repo-real-2026-08-22.md",
        "e33-h06-repo-real-2026-08-22.json",
    ),
    (
        "e33-h07-conformidad",
        "docs/qa/corridas/v0.6.2/conformidad.md",
        "conformidad.json",
    ),
    (
        "e33-h07-rendimiento",
        "docs/qa/corridas/v0.6.2/rendimiento.md",
        "rendimiento.json",
    ),
    (
        "e33-h09-rendimiento",
        "docs/qa/e33-h09-realista-100k-2026-08-23.md",
        "e33-h09-realista-100k-2026-08-23.json",
    ),
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("raíz del repositorio")
}

fn valid_retention_fixture() -> Value {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/e33_retention_manifest_invalid.json"))
            .expect("fixture de retención válida como JSON");
    let results = fixture
        .get("results")
        .and_then(Value::as_array)
        .expect("fixture de retención con results");
    assert_eq!(
        results.len(),
        EXPECTED_RESULTS.len(),
        "fixture base debe conservar exactamente cinco resultados"
    );
    validate_manifest(&repo_root(), &fixture)
        .expect("fixture base válida antes de mutar un único campo");
    fixture
}

fn mutate_one_result<F>(mut fixture: Value, mutate: F) -> Value
where
    F: FnOnce(&mut Value),
{
    let before = fixture["results"]
        .as_array()
        .expect("fixture con cinco resultados antes de mutar")
        .len();
    mutate(&mut fixture);
    assert_eq!(
        fixture["results"].as_array().map(Vec::len),
        Some(before),
        "la mutación negativa no puede ocultar un error de cardinalidad"
    );
    fixture
}

fn assert_rejects_field(fixture: Value, expected_error: &str) {
    let error =
        validate_manifest(&repo_root(), &fixture).expect_err("manifiesto mutado inválido aceptado");
    assert!(
        error.contains(expected_error),
        "el negativo debe rechazar {expected_error}, no: {error}"
    );
    assert!(
        !error.contains("exactamente 5"),
        "el negativo debe fallar por el campo mutado, no por cardinalidad: {error}"
    );
}

fn validate_manifest(root: &Path, manifest: &Value) -> Result<(), String> {
    let object = manifest
        .as_object()
        .ok_or_else(|| "manifiesto debe ser un objeto JSON".to_owned())?;
    let schema = object
        .get("schema_version")
        .and_then(Value::as_str)
        .ok_or_else(|| "manifiesto sin schema_version".to_owned())?;
    if schema != "e33-evidence-manifest-v1" {
        return Err(format!("schema_version inesperado: {schema}"));
    }
    let results = object
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| "manifiesto sin results array".to_owned())?;
    if results.len() != EXPECTED_RESULTS.len() {
        return Err(format!(
            "results debe contener exactamente 5 entradas, no {}",
            results.len()
        ));
    }

    let mut ids = BTreeSet::new();
    for result in results {
        let result = result
            .as_object()
            .ok_or_else(|| "cada resultado debe ser un objeto".to_owned())?;
        let id = result
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "resultado sin id".to_owned())?;
        ids.insert(id.to_owned());

        let summary = result
            .get("summary")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("{id}: summary vacío"))?;
        if !root.join(summary).is_file() {
            return Err(format!("{id}: summary inexistente: {summary}"));
        }

        let artifact = result
            .get("artifact")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{id}: falta artifact"))?;
        let url = artifact
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{id}: falta artifact.url"))?;
        if !url.starts_with("https://") || !url.contains("/releases/") {
            return Err(format!(
                "{id}: artifact.url no es una URL HTTPS estable de release: {url}"
            ));
        }
        if url.contains("localhost") || url.contains("/private/") || url.contains("/tmp/") {
            return Err(format!("{id}: artifact.url parece una ruta privada: {url}"));
        }
        let artifact_sha = artifact
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{id}: falta artifact.sha256"))?;
        if artifact_sha.len() != 64 || !artifact_sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("{id}: SHA-256 de artifact inválido"));
        }
        if artifact
            .get("size_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            == 0
        {
            return Err(format!("{id}: artifact.size_bytes debe ser positivo"));
        }
        if artifact.get("media_type").and_then(Value::as_str) != Some("application/gzip") {
            return Err(format!(
                "{id}: artifact.media_type debe ser application/gzip"
            ));
        }
        if artifact.get("compression").and_then(Value::as_str) != Some("gzip") {
            return Err(format!("{id}: artifact.compression debe ser gzip"));
        }

        let raw = result
            .get("raw")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{id}: falta raw"))?;
        if raw.contains_key("url") {
            return Err(format!(
                "{id}: raw no debe contener url; pertenece a artifact"
            ));
        }
        let raw_sha = raw
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{id}: falta raw.sha256"))?;
        if raw_sha.len() != 64 || !raw_sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("{id}: SHA-256 de raw inválido"));
        }
        if raw.get("size_bytes").and_then(Value::as_u64).unwrap_or(0) == 0 {
            return Err(format!("{id}: raw.size_bytes debe ser positivo"));
        }
        let raw_schema = raw
            .get("schema_version")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("{id}: falta raw.schema_version"))?;
        if !KNOWN_RAW_SCHEMAS.contains(&raw_schema) {
            return Err(format!("{id}: schema de raw desconocido: {raw_schema}"));
        }
    }

    let expected_ids: BTreeSet<_> = EXPECTED_RESULTS
        .iter()
        .map(|(id, _, _)| (*id).to_owned())
        .collect();
    if ids != expected_ids {
        return Err(format!("ids de resultados no coinciden: {ids:?}"));
    }
    Ok(())
}

#[test]
fn manifiesto_de_release_declara_los_cinco_resultados_y_sus_resumenes() {
    let root = repo_root();
    let path = root.join(MANIFEST_RELATIVE);
    let tracked = Command::new("git")
        .args(["ls-files", "--error-unmatch", MANIFEST_RELATIVE])
        .current_dir(&root)
        .output()
        .expect("consultar índice git para el manifiesto");
    assert!(
        tracked.status.success(),
        "el manifiesto de release debe estar rastreado por Git: {MANIFEST_RELATIVE}"
    );
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("leer manifiesto versionado {}: {error}", path.display()));
    let manifest: Value = serde_json::from_str(&text).expect("manifiesto JSON válido");
    validate_manifest(&root, &manifest)
        .unwrap_or_else(|error| panic!("manifiesto inválido: {error}"));

    let results = manifest["results"].as_array().expect("results no vacío");
    for (id, summary, _) in EXPECTED_RESULTS {
        let result = results
            .iter()
            .find(|result| result["id"] == id)
            .unwrap_or_else(|| panic!("falta resultado {id}"));
        assert_eq!(result["summary"], summary, "{id}: resumen trazable");
        let summary_tracked = Command::new("git")
            .args(["ls-files", "--error-unmatch", summary])
            .current_dir(&root)
            .output()
            .expect("consultar índice git para el resumen");
        assert!(
            summary_tracked.status.success(),
            "el resumen debe estar rastreado por Git: {summary}"
        );
    }
}

#[test]
fn manifiesto_rechaza_url_de_artifact_fuera_de_release_estable() {
    let fixture = mutate_one_result(valid_retention_fixture(), |manifest| {
        manifest["results"][0]["artifact"]["url"] =
            json!("https://github.com/dbareagimeno/lodestar/archive/v0.6.2.json");
    });
    assert_rejects_field(
        fixture,
        "artifact.url no es una URL HTTPS estable de release",
    );
}

#[test]
fn manifiesto_rechaza_sha256_de_artifact_invalido() {
    let fixture = mutate_one_result(valid_retention_fixture(), |manifest| {
        manifest["results"][0]["artifact"]["sha256"] = json!("not-a-sha");
    });
    assert_rejects_field(fixture, "SHA-256 de artifact inválido");
}

#[test]
fn manifiesto_rechaza_tamanio_de_artifact_invalido() {
    let fixture = mutate_one_result(valid_retention_fixture(), |manifest| {
        manifest["results"][0]["artifact"]["size_bytes"] = json!(0);
    });
    assert_rejects_field(fixture, "artifact.size_bytes debe ser positivo");
}

#[test]
fn manifiesto_rechaza_sha256_de_raw_invalido() {
    let fixture = mutate_one_result(valid_retention_fixture(), |manifest| {
        manifest["results"][0]["raw"]["sha256"] = json!("not-a-sha");
    });
    assert_rejects_field(fixture, "SHA-256 de raw inválido");
}

#[test]
fn manifiesto_rechaza_tamanio_de_raw_invalido() {
    let fixture = mutate_one_result(valid_retention_fixture(), |manifest| {
        manifest["results"][0]["raw"]["size_bytes"] = json!(0);
    });
    assert_rejects_field(fixture, "raw.size_bytes debe ser positivo");
}

#[test]
fn manifiesto_rechaza_schema_de_raw_vacio() {
    let fixture = mutate_one_result(valid_retention_fixture(), |manifest| {
        manifest["results"][0]["raw"]["schema_version"] = json!("   ");
    });
    assert_rejects_field(fixture, "falta raw.schema_version");
}

#[test]
fn manifiesto_rechaza_schema_de_raw_desconocido() {
    let fixture = mutate_one_result(valid_retention_fixture(), |manifest| {
        manifest["results"][0]["raw"]["schema_version"] = json!("e33-h99-future");
    });
    assert_rejects_field(fixture, "schema de raw desconocido");
}

#[test]
fn volcados_generados_no_estan_versionados_ni_presentes_en_el_arbol() {
    let root = repo_root();
    for (_, _, raw_name) in EXPECTED_RESULTS {
        let relative = match raw_name {
            "e33-h04-corrida-2026-08-22.json" => "docs/qa/e33-h04-corrida-2026-08-22.json",
            "e33-h06-repo-real-2026-08-22.json" => "docs/qa/e33-h06-repo-real-2026-08-22.json",
            "conformidad.json" => "docs/qa/corridas/v0.6.2/conformidad.json",
            "rendimiento.json" => "docs/qa/corridas/v0.6.2/rendimiento.json",
            "e33-h09-realista-100k-2026-08-23.json" => {
                "docs/qa/e33-h09-realista-100k-2026-08-23.json"
            }
            other => panic!("raw no catalogado: {other}"),
        };
        assert!(
            !root.join(relative).exists(),
            "raw generado presente: {relative}"
        );
        let tracked = Command::new("git")
            .args(["ls-files", "--error-unmatch", relative])
            .current_dir(&root)
            .output()
            .expect("consultar índice git");
        assert!(
            !tracked.status.success(),
            "raw generado versionado: {relative}"
        );
    }
}
