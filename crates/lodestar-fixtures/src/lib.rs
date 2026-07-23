//! Workspaces de ejemplo reusables por los tests (E0-H03; ampliado en E15-H05).
//!
//! Dos familias conviven aquí:
//!
//! - **Workspaces Markdown universales** ([`arbitrary`], [`with_edge_cases`], [`materialize`],
//!   [`materialize_disk_only`]) — los que exige `ARCHITECTURE.md §20.5` y
//!   `REFACTOR_PHASE_2 §Tests imprescindibles`: estructuras de carpetas arbitrarias, sin `index.md`
//!   ni frontmatter obligatorio, con los casos límite del descubrimiento.
//! - **Bundles OKF heredados** ([`conformant`], [`with_issues`], [`synthetic`]) — de v0.2.x, vivos
//!   solo mientras existan sus consumidores (los retiran E16/E17 al cambiar el modelo documental).
//!
//! Todos son deterministas: misma llamada ⇒ mismos bytes.

use std::path::Path;

use lodestar_core::types::{FileMap, RelPath};

/// Construye un `FileMap` a partir de pares `(path, contenido)`.
pub fn file_map(pairs: &[(&str, &str)]) -> FileMap {
    pairs
        .iter()
        .map(|(p, c)| {
            (
                RelPath::new(p).expect("fixture con path válido"),
                (*c).to_string(),
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Workspaces Markdown universales (E15-H05, `ARCHITECTURE.md §20.5`)
// ---------------------------------------------------------------------------

/// El workspace del `§Resultado esperado` de `REFACTOR_PHASE_2`: estructura arbitraria, **sin**
/// `index.md`, **sin** frontmatter, con enlaces cruzados entre la raíz y tres niveles de
/// profundidad en ambos sentidos.
///
/// ```text
/// README.md                       → one/first.md, three/levels/deep/third.md
/// one/first.md                    → ../two/levels/second.md   (hermano en otro árbol)
/// two/levels/second.md            → (sin salientes: solo lo enlazan)
/// three/levels/deep/third.md      → ../../../README.md        (vuelta a la raíz)
/// ```
pub fn arbitrary() -> FileMap {
    file_map(&[
        (
            "README.md",
            "# Proyecto\n\nEmpieza por [lo primero](one/first.md) y mira lo \
             [profundo](three/levels/deep/third.md).\n",
        ),
        (
            "one/first.md",
            "# Primero\n\nHermano en otro árbol: [segundo](../two/levels/second.md).\n",
        ),
        (
            "two/levels/second.md",
            "# Segundo\n\nNo enlaza a nadie; solo lo enlazan.\n",
        ),
        (
            "three/levels/deep/third.md",
            "# Tercero\n\nVolver a la [visión general](../../../README.md).\n",
        ),
    ])
}

/// Casos límite de resolución de enlaces y de paths, en un solo workspace.
///
/// Cubre: paths con espacios, href con `%20`, directorio oculto, dos documentos con el **mismo
/// basename** en árboles distintos, un enlace con **capitalización errónea** (portabilidad), un
/// enlace a un fichero de código del proyecto, un enlace externo, un anchor propio, un destino
/// inexistente y un intento de escape del workspace.
pub fn with_edge_cases() -> FileMap {
    file_map(&[
        (
            "notas/con espacios.md",
            "# Con espacios\n\nEl path de este documento lleva espacios.\n",
        ),
        (
            "raiz.md",
            "# Raíz\n\n\
             Espacios por porcentaje: [nota](notas/con%20espacios.md).\n\
             Capitalización errónea: [auth](Docs/Auth.md).\n\
             A código del proyecto: [servicio](src/auth/token_service.rs).\n\
             Externo: [web](https://example.com).\n\
             Anchor propio: [aquí](#raiz).\n\
             Inexistente: [falta](no-existe.md).\n\
             Escape: [fuera](../../../etc/passwd).\n",
        ),
        (
            "docs/auth.md",
            "# Auth\n\nEl enlace real es en minúsculas: `docs/auth.md`.\n",
        ),
        (
            ".oculto/secreto.md",
            "# Oculto\n\nVive en un directorio que empieza por punto.\n",
        ),
        // Mismo basename (`auth.md`) en dos árboles distintos: deben quedar inequívocos.
        (
            "packages/api/docs/auth.md",
            "# Auth de la API\n\nMismo basename que [el otro](../../../docs/auth.md).\n",
        ),
    ])
}

/// Escribe un [`FileMap`] en disco bajo `root`, creando los directorios intermedios.
///
/// Los tests de descubrimiento necesitan ficheros reales (el walker recorre disco, no un mapa);
/// el resto de tests siguen trabajando con el `FileMap` en memoria.
///
/// # Errores
/// Propaga cualquier error de I/O (crear directorio o escribir fichero).
pub fn materialize(files: &FileMap, root: &Path) -> std::io::Result<()> {
    for (rel, content) in files {
        let target = root.join(rel.as_str());
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, content)?;
    }
    Ok(())
}

/// Materializa los casos que **no** son representables en un [`FileMap`] porque no son texto UTF-8
/// válido, no son ficheros regulares, o son ficheros de control del descubrimiento.
///
/// Crea bajo `root`:
/// - `binario.md` — bytes no UTF-8 (`DOC-NOT-UTF8`).
/// - `enorme.md` — `size_limit + 1` bytes de texto (`DOC-TOO-LARGE`).
/// - `enlace.md` — symlink a `README.md`, solo en Unix (`SYMLINK-UNSUPPORTED`).
/// - `vendor/dep.md` + `.gitignore` con `vendor/`.
/// - `borradores/wip.md` + `.lodestarignore` con `borradores/`.
/// - `src/auth/token_service.rs` — fichero del proyecto que **no** es Markdown (destino de un
///   enlace `WorkspaceFile`).
///
/// # Errores
/// Propaga cualquier error de I/O.
pub fn materialize_disk_only(root: &Path, size_limit: usize) -> std::io::Result<()> {
    std::fs::write(root.join("binario.md"), [0xF0, 0x28, 0x8C, 0xBC])?;
    // `size_limit + 1` bytes exactos: un byte por encima del límite, sin ambigüedad de frontera.
    std::fs::write(root.join("enorme.md"), "a".repeat(size_limit + 1))?;

    std::fs::create_dir_all(root.join("vendor"))?;
    std::fs::write(root.join("vendor/dep.md"), "# Dependencia\n")?;
    std::fs::write(root.join(".gitignore"), "vendor/\n")?;

    std::fs::create_dir_all(root.join("borradores"))?;
    std::fs::write(root.join("borradores/wip.md"), "# Borrador\n")?;
    std::fs::write(root.join(".lodestarignore"), "borradores/\n")?;

    std::fs::create_dir_all(root.join("src/auth"))?;
    std::fs::write(
        root.join("src/auth/token_service.rs"),
        "// Destino de un enlace WorkspaceFile.\n",
    )?;

    // El symlink necesita un destino existente para que el walker lo vea como `.md`.
    let objetivo = root.join("README.md");
    if !objetivo.exists() {
        std::fs::write(&objetivo, "# Objetivo del symlink\n")?;
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&objetivo, root.join("enlace.md"))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Bundles OKF heredados (v0.2.x) — se retiran con sus consumidores en E16/E17
// ---------------------------------------------------------------------------

/// Bundle conforme mínimo: un index raíz + un concept válido que se enlazan mutuamente.
pub fn conformant() -> FileMap {
    file_map(&[
        ("index.md", "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n# Concept\n\n* [Alfa](alfa.md)\n"),
        (
            "alfa.md",
            "---\ntype: Concept\ntitle: Alfa\ndescription: Primer concept\n---\n\n# Resumen\n\nEnlaza a [Beta](/beta.md).\n",
        ),
        (
            "beta.md",
            "---\ntype: Concept\ntitle: Beta\ndescription: Segundo concept\n---\n\n# Resumen\n\nVuelve a [Alfa](/alfa.md).\n",
        ),
    ])
}

/// Workspace con un documento por cada diagnóstico que produce `conform` en el **catálogo mínimo**
/// de `ARCHITECTURE.md §20.9` (E16-H05): `FM-UNCLOSED`, `FM-YAML-INVALID`, `DOC-CONFLICT-MARKER` y
/// los dos de enlaces que siguen vivos hasta E17 (`LINK-STUB`, `LINK-REL`).
///
/// Ya **no** dispara el catálogo OKF (`OKF-FM01`, `OKF-TYPE`, `REC-*`, `FMT-*`, `BODY-STRUCT`,
/// `ORPHAN`): esos códigos se retiraron. Los dos primeros documentos son justamente el contraste —
/// un `.md` pelado y un frontmatter con metadata «rara» son válidos y **silenciosos**.
pub fn with_issues() -> FileMap {
    file_map(&[
        // Sin frontmatter, sin encabezados y sin enlaces: VÁLIDO y silencioso.
        ("sin-fm.md", "Solo cuerpo, sin encabezados.\n"),
        // Metadata arbitraria del usuario (antes `FMT-TAGS`/`FMT-TS`/`REC-*`): también silencioso.
        (
            "metadata-libre.md",
            "---\ntitle: \ntags: uno\ntimestamp: ayer\n---\n\ncuerpo sin encabezado\n",
        ),
        // Frontmatter sin cerrar → FM-UNCLOSED (hard-fail).
        ("sin-cierre.md", "---\ntype: Concept\n"),
        // Bloque bien delimitado con YAML inválido → FM-YAML-INVALID (hard-fail, con rango).
        ("yaml-roto.md", "---\ntype: : :\n  - x\n---\n\n# H\n\ncuerpo\n"),
        // Enlace a inexistente → LINK-STUB; enlace relativo → LINK-REL.
        (
            "malo.md",
            "---\ntype: Nota\ntitle: Malo\ndescription: x\n---\n\n# H\n\n[falta](/no-existe.md) y [rel](./otro.md)\n",
        ),
        // Marcadores de conflicto → DOC-CONFLICT-MARKER (hard-fail).
        (
            "conflicto.md",
            "---\ntype: Nota\ntitle: C\ndescription: d\n---\n\n# H\n\n<<<<<<< HEAD\nuno\n=======\ndos\n>>>>>>> rama\n",
        ),
    ])
}

/// Genera un bundle sintético de `n` concepts deterministas (semilla fija → bytes idénticos).
pub fn synthetic(n: usize) -> FileMap {
    let mut pairs: Vec<(RelPath, String)> = Vec::with_capacity(n + 1);
    pairs.push((
        RelPath::new("index.md").unwrap(),
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n".to_string(),
    ));
    for i in 0..n {
        let next = (i + 1) % n;
        let raw = format!(
            "---\ntype: Concept\ntitle: Concept {i}\ndescription: sintetico {i}\n---\n\n# Resumen\n\nEnlaza a [siguiente](/c{next:06}.md).\n"
        );
        pairs.push((RelPath::new(&format!("c{i:06}.md")).unwrap(), raw));
    }
    pairs.into_iter().collect()
}
