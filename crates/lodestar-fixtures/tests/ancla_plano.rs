//! E33-H01 (añadido del implementador tras el panel de jueces) — **ancla dorada** del corpus plano.
//!
//! Por qué existe: el arnés de escala de E14-H05 (`crates/lodestar-app/tests/escala.rs`) reconstruye
//! el tamaño de los cuerpos llamando a la MISMA función que los escribió (`escala::cuerpo_plano`),
//! así que sus aserciones son **autoconsistentes**: si alguien cambiara el relleno del cuerpo, el
//! test seguiría en verde y las cifras históricas de E14-H05 dejarían de ser comparables **en
//! silencio** — que es justo lo que `§22.2` pide preservar («comparable con las cifras históricas»).
//!
//! Este fichero rompe esa circularidad con un **hash dorado** literal, calculado sobre un corpus
//! plano pequeño. No comprueba que el corpus sea «bonito»: comprueba que sus bytes son **los mismos
//! de siempre**. Si falla, la pregunta no es «¿arreglo el hash?» sino «¿por qué han cambiado los
//! bytes del corpus con el que comparamos las mediciones?» — y si el cambio es deliberado, se
//! actualiza el dorado **y** se anota que las cifras anteriores dejan de ser comparables.

use std::collections::BTreeMap;
use std::path::Path;

use lodestar_fixtures::escala::{self, Perfil};

/// Huella del árbol: `path relativo → digest hex`, en orden determinista.
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

/// Tamaño del corpus anclado: pequeño a propósito (el ancla es sobre los BYTES, no sobre el
/// volumen), pero suficiente para cubrir el `index.md` y varios documentos.
const TAMANO: usize = 10;

/// Digest dorado del corpus plano de [`TAMANO`] documentos: `blake3` de la huella serializada como
/// `path\ndigest\n…`. Calculado el 2026-08-10.
///
/// Procedencia del valor (no es un número que salió del propio código y se copió sin más): se
/// reconstruyó el generador ORIGINAL de E14-H05 desde `git show HEAD:crates/lodestar-app/tests/
/// escala.rs`, se materializó su corpus y se comparó con `diff -r` contra el que produce
/// `escala::genera(_, Perfil::Plano, 10, _)` — **idénticos byte a byte**. Este dorado ancla, por
/// tanto, los bytes de E14-H05, no los de una extracción que pudiera haber derivado.
///
/// **No lo actualices para «arreglar» el test**: ver el porqué en la cabecera del fichero.
const DORADO: &str = "0d377190e1cd28e2c9a61ed04d4281d8998ed47b8328d8edcfa02bfe022e1400";

#[test]
fn el_corpus_plano_conserva_los_bytes_de_e14_h05() {
    let dir = tempfile::tempdir().unwrap();
    // La semilla es irrelevante en el perfil plano (es su contrato: homogéneo y comparable con las
    // cifras históricas). Se pasa una arbitraria justamente para dejarlo demostrado.
    escala::genera(dir.path(), Perfil::Plano, TAMANO, 12_345)
        .expect("el corpus plano debe escribirse");

    let h = huella(dir.path());
    assert_eq!(
        h.len(),
        TAMANO,
        "el corpus plano de {TAMANO} debe emitir exactamente {TAMANO} ficheros"
    );

    let mut serial = String::new();
    for (path, digest) in &h {
        serial.push_str(path);
        serial.push('\n');
        serial.push_str(digest);
        serial.push('\n');
    }
    let observado = blake3::hash(serial.as_bytes()).to_hex().to_string();

    assert_eq!(
        observado, DORADO,
        "los bytes del corpus PLANO han cambiado respecto al de E14-H05.\n\
         Si el cambio NO es deliberado, es una regresión: el corpus plano existe para que las \
         mediciones de escala sigan siendo comparables con las cifras históricas.\n\
         Si SÍ es deliberado, actualiza el dorado a «{observado}» y deja anotado en la historia \
         que las cifras anteriores dejan de ser comparables."
    );
}
