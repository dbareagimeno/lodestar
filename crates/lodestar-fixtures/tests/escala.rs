//! E33-H01 · BDD-1 — Determinismo del generador de escala compartido (`ARCHITECTURE.md §22.2`).
//!
//! El generador que hoy vive inline en `crates/lodestar-app/tests/escala.rs` (E14-H05,
//! `genera_workspace_grande`/`cuerpo_grande`) se extrae aquí, parametrizado por **perfil**
//! (plano/realista), **tamaño** y **semilla**. La propiedad que el banco de evidencia necesita —y
//! la que este fichero juzga— es que **la misma semilla produce el mismo árbol byte a byte**: sin
//! ella, ninguna medición de `E33-H04` sería comparable entre corridas ni entre máquinas.
//!
//! Nada de timestamps, nada de orden de iteración de `HashMap`, nada de rutas absolutas dentro del
//! contenido: el árbol depende **solo** de `(perfil, tamaño, semilla)`.
//!
//! La comparación se hace sobre una **huella por fichero** (path → digest hex del contenido), no
//! sobre los bytes crudos: es igual de estricta —un solo byte distinto cambia el digest— y mantiene
//! legible el mensaje de fallo, que si no volcaría megabytes de corpus.

use std::collections::BTreeMap;
use std::path::Path;

use lodestar_fixtures::escala::{self, Perfil};

/// Huella del árbol bajo `root`: `path relativo → digest hex del contenido`, en orden determinista
/// (`BTreeMap`) y **sin** interpretar el contenido. Se leen **todos** los ficheros, no solo los
/// `.md`: si el generador emitiese un `.gitignore` o un fichero de proyecto, también entra.
fn huella(root: &Path) -> BTreeMap<String, String> {
    fn recorrer(dir: &Path, root: &Path, out: &mut BTreeMap<String, String>) {
        let mut entradas: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .expect("directorio legible")
            .map(|e| e.expect("entrada legible").path())
            .collect();
        entradas.sort();
        for ruta in entradas {
            if ruta.is_dir() {
                recorrer(&ruta, root, out);
            } else {
                let rel = ruta
                    .strip_prefix(root)
                    .expect("bajo el root")
                    .to_string_lossy()
                    .replace('\\', "/");
                let bytes = std::fs::read(&ruta).expect("fichero legible");
                out.insert(rel, blake3::hash(&bytes).to_hex().to_string());
            }
        }
    }
    let mut out = BTreeMap::new();
    recorrer(root, root, &mut out);
    out
}

/// Genera `(perfil, tamaño, semilla)` en un tempdir nuevo y devuelve su huella.
fn genera_y_huella(perfil: Perfil, tamano: usize, semilla: u64) -> BTreeMap<String, String> {
    let dir = tempfile::tempdir().expect("tempdir");
    escala::genera(dir.path(), perfil, tamano, semilla)
        .expect("el generador debe escribir el árbol");
    huella(dir.path())
}

/// Nº de documentos `.md` de una huella.
fn documentos(h: &BTreeMap<String, String>) -> usize {
    h.keys().filter(|p| p.ends_with(".md")).count()
}

/// Semilla arbitraria pero fija: lo que se juzga es la reproducibilidad, no su valor.
const SEMILLA: u64 = 0xE33_0001;

/// Escala mínima de `§22.2` (~100): basta para las propiedades y mantiene el test barato.
const TAMANO: usize = 100;

// ===========================================================================
// E33-H01 · BDD-1 — `generador_de_escala_es_determinista_con_la_misma_semilla`.
//
// Dado la misma semilla, Cuando el generador Rust genera dos veces el mismo perfil y tamaño,
// Entonces los dos árboles son byte-idénticos.
// ===========================================================================
#[test]
fn generador_de_escala_es_determinista_con_la_misma_semilla() {
    for perfil in [Perfil::Plano, Perfil::Realista] {
        let primera = genera_y_huella(perfil, TAMANO, SEMILLA);
        let segunda = genera_y_huella(perfil, TAMANO, SEMILLA);

        // --- Propiedad 1 (no vacuo): el generador produjo un árbol del tamaño pedido. ---
        // Sin esto, un generador que no escribiera NADA sería «determinista» trivialmente.
        assert_eq!(
            documentos(&primera),
            TAMANO,
            "el perfil {perfil:?} debe emitir exactamente {TAMANO} documentos `.md`"
        );

        // --- Propiedad 2 (LO ESENCIAL): los dos árboles son byte-idénticos. ---
        assert_eq!(
            primera.keys().collect::<Vec<_>>(),
            segunda.keys().collect::<Vec<_>>(),
            "misma semilla ⇒ mismo conjunto de paths (perfil {perfil:?})"
        );
        let difieren: Vec<&String> = primera
            .iter()
            .filter(|(p, d)| segunda.get(*p) != Some(*d))
            .map(|(p, _)| p)
            .collect();
        assert!(
            difieren.is_empty(),
            "misma semilla ⇒ árboles byte-idénticos (perfil {perfil:?}); \
             {} fichero(s) con contenido distinto entre las dos corridas, p. ej. {:?}",
            difieren.len(),
            &difieren[..difieren.len().min(5)],
        );
    }
}

// ===========================================================================
// E33-H01 · BDD-1 (complemento) — en el perfil REALISTA la semilla es un parámetro REAL.
//
// Sin esto, un generador que ignorase la semilla pasaría el test de arriba: sería determinista por
// la razón equivocada (constante), y las tres corridas del banco con semillas distintas medirían
// tres veces el mismo corpus.
//
// El criterio se aplica SOLO al perfil realista: el plano es homogéneo **por definición**
// (`§22.2` lo quiere «comparable con las cifras históricas de E14-H05»), así que ahí la semilla no
// tiene por qué cambiar nada y exigirlo contradiría BDD-3.
// ===========================================================================
#[test]
fn en_el_perfil_realista_la_semilla_cambia_el_corpus() {
    let a = genera_y_huella(Perfil::Realista, TAMANO, 1);
    let b = genera_y_huella(Perfil::Realista, TAMANO, 2);

    assert_eq!(documentos(&a), TAMANO, "corpus no vacuo");
    assert_ne!(
        a, b,
        "la semilla debe influir en el corpus realista: dos semillas distintas produjeron el \
         mismo árbol, luego la semilla es decorativa"
    );
}

// ===========================================================================
// E33-H01 · BDD-1 (complemento) — las tres escalas de `§22.2` existen y el tamaño manda.
//
// `§22.2` pide ~100 / ~1k / ~10k. Aquí se ejercitan las dos baratas en los dos perfiles; la de
// ~10k la paga `escala.rs` de `lodestar-app`, que es donde ese coste ya estaba presupuestado.
// ===========================================================================
#[test]
fn el_generador_soporta_las_escalas_de_la_seccion_22_2() {
    for perfil in [Perfil::Plano, Perfil::Realista] {
        for tamano in [100usize, 1_000] {
            let h = genera_y_huella(perfil, tamano, SEMILLA);
            assert_eq!(
                documentos(&h),
                tamano,
                "el perfil {perfil:?} debe emitir exactamente {tamano} documentos"
            );
        }
    }
}
