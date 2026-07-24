//! Tests de integración de la **configuración opcional del workspace** (E15-H08).
//!
//! Fijan el contrato de `.lodestar/config.yaml` como **único** fichero de configuración del motor
//! (`ARCHITECTURE.md §20.5` para la política de descubrimiento, `§20.9` para la de validación) y,
//! sobre todo, la regla que le da sentido:
//!
//! > **la config LIMITA, nunca habilita** (invariante #18 de `REFACTOR_PHASE_2`).
//!
//! De ahí las cuatro propiedades que se verifican aquí:
//!
//! 1. **Su ausencia no impide nada** — un directorio sin `.lodestar/` se abre con la política por
//!    defecto de `§20.5` y Lodestar no le escribe ninguna config encima.
//! 2. **Lo que declara se obedece** — un `discovery.exclude` del usuario llega al inventario real
//!    del producto (`Workspace::document_set`), no solo a una struct.
//! 3. **Rota es un error, no un default** — un YAML malformado nunca degrada en silencio a la
//!    política por defecto: un typo relajaría las restricciones del usuario sin avisar.
//! 4. **Hay un suelo duro que no puede levantar** — `.lodestar/**` queda fuera del inventario
//!    aunque la config lo intente reabrir.
//!
//! Y una quinta, de **retirada**: `lodestar.toml` deja de ser configuración. Dos ficheros de config
//! para lo mismo es deuda (y `identity`, su otro habitante, murió en E15-H01), así que a partir de
//! esta historia un `lodestar.toml` en la raíz es un fichero más del proyecto: ni se lee, ni su
//! sintaxis importa.
//!
//! Los workspaces salen de `lodestar-fixtures` (E15-H05) cuando basta con `arbitrary()`; los
//! ficheros de control (`.gitignore`, `.lodestar/…`) se escriben a mano para que cada escenario sea
//! legible sin saltar al fixture.

use std::path::Path;

use lodestar_core::types::FileMap;
use lodestar_workspace::discovery::DiscoveryPolicy;
use lodestar_workspace::{Workspace, WorkspaceConfig};

/// Los 4 documentos de `lodestar_fixtures::arbitrary()`, en el orden determinista del `FileMap`.
const DOCUMENTOS_ARBITRARY: [&str; 4] = [
    "README.md",
    "one/first.md",
    "three/levels/deep/third.md",
    "two/levels/second.md",
];

/// Escribe `<root>/.lodestar/config.yaml` con el contenido dado (crea `.lodestar/` si falta).
fn escribe_config_yaml(root: &Path, contenido: &str) {
    let dir = root.join(".lodestar");
    std::fs::create_dir_all(&dir).expect("crear .lodestar/");
    std::fs::write(dir.join("config.yaml"), contenido).expect("escribir config.yaml");
}

/// Escribe un fichero bajo `root`, creando los directorios intermedios.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let destino = root.join(rel);
    if let Some(padre) = destino.parent() {
        std::fs::create_dir_all(padre).expect("crear directorio intermedio");
    }
    std::fs::write(&destino, contenido).expect("escribir fichero");
}

/// ¿Está `path` en el inventario?
fn contiene(files: &FileMap, path: &str) -> bool {
    files.keys().any(|p| p.as_str() == path)
}

/// Los paths del inventario, ordenados (el `FileMap` es un `BTreeMap`: determinista).
fn rutas(files: &FileMap) -> Vec<&str> {
    files.keys().map(|p| p.as_str()).collect()
}

// ---------------------------------------------------------------------------
// Criterio 1: sin config no hay ceremonia
// ---------------------------------------------------------------------------

/// **Dado** un directorio **sin** `.lodestar/`, **Cuando** se abre, **Entonces** se usa la política
/// por defecto de `§20.5` y no se crea ningún fichero de config.
///
/// Es el invariante #18 (*«la config limita, nunca habilita; su ausencia no impide usar
/// Lodestar»*) puesto a prueba en el punto exacto donde esta historia puede romperlo: E15-H08 añade
/// carga de config a la apertura, y la forma trivial de equivocarse es exigir el fichero (o
/// escribirlo al vuelo «para que exista»). Cualquiera de las dos cosas convertiría el caso que
/// persigue la épica —`cd` a un proyecto arbitrario y arrancar— en una ceremonia de configuración.
///
/// Se comprueban las tres caras de «se usa la política por defecto»:
///
/// - **la política efectiva** que sirve el punto de inyección único (`Workspace::discovery_policy`)
///   es literalmente `DiscoveryPolicy::default()`;
/// - **el inventario real** que produce (`document_set()`), porque una política correcta que no se cabléa
///   no vale de nada (misma lección que `workspace_usa_la_politica_de_descubrimiento` en E15-H07);
/// - **los defaults de la sección `discovery` del YAML**, que tienen que ser los mismos: si
///   divergieran, escribir la sección entera con los valores «de fábrica» daría un descubrimiento
///   distinto que no escribirla — una config que *habilita* en vez de limitar.
///
/// Sobre el `.lodestar/` que sí aparece tras abrir: `Workspace::open` garantiza el scaffold de
/// `.lodestar/runtime/` y ajusta el `.gitignore` (D5, ya vigente). Eso no es configuración —son
/// directorios de trabajo desechables—, así que lo que se asevera es que **no hay fichero de
/// config** en disco, ni el nuevo ni el legado.
#[test]
fn sin_config_funciona() {
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();
    // Deliberadamente NO se escribe `.lodestar/config.yaml` ni `lodestar.toml`.

    let ws = Workspace::open(dir.path())
        .expect("un directorio sin `.lodestar/` es un workspace válido: abrirlo no puede fallar");

    // --- (1) La política efectiva es la de `§20.5` ----------------------------------
    assert_eq!(
        ws.discovery_policy(),
        DiscoveryPolicy::default(),
        "sin config, la política efectiva debe ser EXACTAMENTE la de `§20.5`"
    );

    // --- (2) …y llega al inventario real -------------------------------------------
    let doc_set = ws
        .document_set()
        .expect("el inventario debe cargarse sin config");
    assert_eq!(
        rutas(doc_set.files()),
        DOCUMENTOS_ARBITRARY.to_vec(),
        "sin config se descubre todo el árbol `.md` a cualquier profundidad"
    );

    // --- (3) Abrir NO materializa configuración -------------------------------------
    assert!(
        !dir.path().join(".lodestar/config.yaml").exists(),
        "abrir un workspace no puede escribirle una config encima: la ausencia de \
         `.lodestar/config.yaml` es un estado válido y permanente, no un hueco que rellenar"
    );
    assert!(
        !dir.path().join("lodestar.toml").exists(),
        "y mucho menos la config legada, que esta historia retira"
    );

    // --- (4) Los defaults de la sección `discovery` son los de `§20.5` ---------------
    // Sin esta igualdad, un usuario que copiara la política por defecto documentada en `§20.5`
    // dentro de su `config.yaml` obtendría un comportamiento distinto del de no tener config.
    // (No se asevera aquí el default de `exclude`: dónde vive el suelo duro `.lodestar/**` —en el
    // default de la sección o inyectado al construir la política— lo decide el implementador; lo
    // que no es negociable es su efecto, y eso lo fija `exclude_vacio_no_reabre_lodestar`.)
    let cfg = WorkspaceConfig::load(dir.path())
        .expect("un workspace sin config.yaml debe cargar defaults seguros, no fallar");
    assert_eq!(
        cfg.discovery.include,
        DiscoveryPolicy::default().include,
        "el `include` por defecto de la config debe ser el de `§20.5` (`**/*.md`)"
    );
    assert!(
        !cfg.discovery.follow_symlinks,
        "los symlinks siguen desactivados por defecto (`§20.5`): la config puede limitar, no \
         abrir puertas que el default cierra"
    );
}

// ---------------------------------------------------------------------------
// Criterio 2: lo que la config declara se obedece
// ---------------------------------------------------------------------------

/// **Dado** un `.lodestar/config.yaml` con `discovery.exclude: ["notas/**"]`, **Cuando** se
/// descubre, **Entonces** `notas/x.md` queda fuera del inventario.
///
/// El test no se conforma con que la struct traiga el glob: exige que el glob llegue **al producto**
/// por los dos caminos que leen conocimiento del disco, y que ambos vean lo mismo:
///
/// - `document_set()` — el inventario que alimenta análisis, grafo, búsqueda y las dos fachadas;
/// - `workspace_revision()` — el control optimista del motor transaccional.
///
/// Si el `exclude` llegara solo al primero, un documento excluido seguiría moviendo la revisión del
/// workspace: `change_apply` empezaría a dar `WRITE_CONFLICT` fantasma por cambios en ficheros que
/// el plan ni siquiera ve. Es la misma coherencia que E15-H07 fijó para la política por defecto,
/// ahora con una política que viene de fuera.
#[test]
fn exclude_configurado() {
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();
    escribe(dir.path(), "notas/x.md", "# Nota excluida por la config\n");
    escribe_config_yaml(dir.path(), "discovery:\n  exclude: [\"notas/**\"]\n");

    let ws = Workspace::open(dir.path()).unwrap();

    // --- La política efectiva lleva el glob del usuario ------------------------------
    assert!(
        ws.discovery_policy()
            .exclude
            .iter()
            .any(|g| g == "notas/**"),
        "el `discovery.exclude` del YAML debe llegar a la política efectiva; era: {:?}",
        ws.discovery_policy().exclude
    );

    // --- …y el inventario del producto lo obedece ------------------------------------
    let doc_set = ws.document_set().unwrap();
    assert!(
        !contiene(doc_set.files(), "notas/x.md"),
        "`notas/**` está excluido por la config: `notas/x.md` no puede estar en el inventario. \
         Inventario: {:?}",
        rutas(doc_set.files())
    );
    assert_eq!(
        rutas(doc_set.files()),
        DOCUMENTOS_ARBITRARY.to_vec(),
        "excluir `notas/**` no puede llevarse por delante ningún otro documento"
    );

    // --- …y la revisión del workspace ve exactamente lo mismo ------------------------
    let rev_inicial = ws.workspace_revision().unwrap();
    std::fs::write(
        dir.path().join("notas/x.md"),
        "# Nota excluida (modificada por el test)\n",
    )
    .unwrap();
    assert_eq!(
        ws.workspace_revision().unwrap(),
        rev_inicial,
        "un documento excluido por la config NO forma parte de la revisión del workspace: si la \
         mueve, `workspace_revision` descubre un conjunto distinto del de `document_set()` y el control \
         optimista pasa a dar conflictos por ficheros que el plan no ve"
    );

    std::fs::write(
        dir.path().join("one/first.md"),
        "# Primero (modificado por el test)\n",
    )
    .unwrap();
    assert_ne!(
        ws.workspace_revision().unwrap(),
        rev_inicial,
        "y lo que SÍ está en el inventario debe seguir moviéndola: un `exclude` demasiado ancho \
         dejaría documentos vivos sin protección del control optimista"
    );
}

// ---------------------------------------------------------------------------
// Criterio 3: una config rota es un error, no un default
// ---------------------------------------------------------------------------

/// **Dado** un `.lodestar/config.yaml` con YAML malformado, **Cuando** se abre, **Entonces** error
/// explícito — nunca caída silenciosa a defaults.
///
/// ## Por qué el error tiene que salir por la apertura
///
/// Hoy `Workspace::open` ni mira la config, y los dos consumidores internos que sí la leen la
/// cargan con `unwrap_or_default()` (`lib.rs:131`, `receipts.rs:181`). Mientras la config solo
/// guardaba `writableRoots` y la retención de recibos, eso era una imprudencia acotada; en cuanto
/// gobierna el **descubrimiento**, deja de serlo: un typo en el YAML haría que Lodestar descubriera
/// un conjunto de documentos **distinto del que el usuario declaró** sin decir una palabra —
/// analizando lo que el usuario excluyó, o dejando de proteger lo que incluyó. Un fallo silencioso
/// de una config de seguridad es peor que no tener config.
///
/// Por eso el test exige que **falle la apertura** (que es lo que traduce el criterio *«cuando se
/// abre»*), y cierra además la puerta de atrás: si alguna vía de apertura decidiera no validar,
/// entonces la lectura del inventario por esa vía tampoco puede devolver un resultado calculado con
/// defaults inventados. Ninguna de las dos puede acabar en un `Ok` con la política por defecto.
///
/// El mensaje debe nombrar el fichero: un «error de IO» a secas ante un typo de YAML es
/// indistinguible de un disco lleno, y quien lo lea no sabrá qué arreglar.
#[test]
fn config_malformada_es_error() {
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();
    // Secuencia de flujo YAML sin cerrar: parseo inválido garantizado.
    escribe_config_yaml(dir.path(), "discovery:\n  exclude: [\"notas/**\"\n");

    let err = match Workspace::open(dir.path()) {
        Err(e) => e,
        Ok(ws) => panic!(
            "abrir un workspace con `.lodestar/config.yaml` malformado debe ser un error \
             explícito, no una caída silenciosa a la política por defecto. Se abrió con la \
             política {:?}",
            ws.discovery_policy()
        ),
    };
    let msg = err.to_string();
    assert!(
        msg.contains("config.yaml"),
        "el error debe nombrar el fichero que hay que arreglar (si no, es indistinguible de \
         cualquier otro fallo de IO). Mensaje: {msg:?}"
    );

    // Puerta de atrás: la apertura hermética (CLI one-shot) tampoco puede servir un inventario
    // calculado con una política que el usuario no escribió.
    if let Ok(ws) = Workspace::open_ephemeral(dir.path()) {
        assert!(
            ws.document_set().is_err(),
            "con la config malformada, ninguna vía puede devolver un inventario: se habría \
             calculado con defaults que el usuario nunca declaró. Inventario servido: {:?}",
            ws.document_set().map(|b| rutas(b.files()).len())
        );
    }
}

// ---------------------------------------------------------------------------
// Criterio 4: `lodestar.toml` deja de ser configuración
// ---------------------------------------------------------------------------

/// **Dado** un `lodestar.toml` en la raíz, **Cuando** se abre, **Entonces** se ignora por completo
/// (es un fichero más del proyecto).
///
/// La historia **borra** `Config`/`lodestar.toml` (`config.rs:14-63`) y cierra `DECISIONES.md §8`:
/// dos ficheros de configuración para lo mismo es deuda, y el otro habitante del TOML (`identity`)
/// murió en E15-H01. Lo que este test fija no es la desaparición del símbolo —eso no se puede
/// aseverar desde un test— sino sus **dos consecuencias observables**, que son las que un futuro
/// «pues volvamos a soportar TOML por compatibilidad» rompería:
///
/// 1. **El TOML no manda**: aunque contenga secciones con nombres reconocibles, no altera el
///    descubrimiento. El fichero de configuración es uno y es `.lodestar/config.yaml`.
/// 2. **Su sintaxis no importa**: un `lodestar.toml` que ni siquiera es TOML válido no puede tumbar
///    la apertura. Es el contraste exacto con `config_malformada_es_error` — allí el fichero roto
///    **es** la config del motor y por eso aborta; aquí es un fichero cualquiera de un proyecto
///    ajeno (los hay a montones: `lodestar.toml` podría ser de otra herramienta homónima) y
///    Lodestar no tiene ninguna autoridad para juzgarlo.
#[test]
fn lodestar_toml_ignorado() {
    // --- (1) El TOML no manda; el YAML sí -------------------------------------------
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();
    // TOML válido y deliberadamente «plausible»: si alguien lo leyera, `one/` desaparecería del
    // inventario y los avisos bloquearían la puerta.
    escribe(
        dir.path(),
        "lodestar.toml",
        "[gate]\nblock_warnings = true\n\n[discovery]\nexclude = [\"one/**\"]\n",
    );
    // La config de verdad excluye OTRA cosa, para que ambos efectos sean distinguibles.
    escribe_config_yaml(dir.path(), "discovery:\n  exclude: [\"two/**\"]\n");

    let ws = Workspace::open(dir.path())
        .expect("un `lodestar.toml` en la raíz no puede impedir abrir el workspace");
    let doc_set = ws.document_set().unwrap();

    assert!(
        contiene(doc_set.files(), "one/first.md"),
        "`lodestar.toml` es un fichero más del proyecto: su `[discovery]` no puede excluir nada. \
         Inventario: {:?}",
        rutas(doc_set.files())
    );
    assert!(
        !contiene(doc_set.files(), "two/levels/second.md"),
        "…mientras que el `discovery.exclude` de `.lodestar/config.yaml` sí manda. Inventario: {:?}",
        rutas(doc_set.files())
    );
    assert!(
        !ws.discovery_policy().exclude.iter().any(|g| g == "one/**"),
        "ningún glob puede venir del TOML; la política era: {:?}",
        ws.discovery_policy().exclude
    );

    // --- (2) Su sintaxis no importa --------------------------------------------------
    let otro = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), otro.path()).unwrap();
    // Ni siquiera es TOML: cabecera de sección sin cerrar. Antes de esta historia esto era exit 3
    // en `lodestar check`; ahora es basura irrelevante.
    escribe(
        otro.path(),
        "lodestar.toml",
        "[gate\nblock_warnings = true\n",
    );

    let ws = Workspace::open(otro.path()).expect(
        "un `lodestar.toml` que ni siquiera es TOML válido no puede tumbar la apertura: Lodestar \
         ya no lo lee, así que no tiene nada que decir sobre su sintaxis",
    );
    assert_eq!(
        rutas(ws.document_set().unwrap().files()),
        DOCUMENTOS_ARBITRARY.to_vec(),
        "y el inventario debe ser el completo, como si el fichero no existiera"
    );
}

// ---------------------------------------------------------------------------
// Criterio 5 (el suelo duro): la config puede añadir exclusiones, nunca quitar `.lodestar/**`
// ---------------------------------------------------------------------------

/// **Dado** un `.lodestar/config.yaml` con `discovery.exclude: []`, **Cuando** se descubre,
/// **Entonces** `.lodestar/templates/plantilla.md` **sigue** fuera del inventario.
///
/// ## Qué se juega aquí
///
/// E15-H07 cerró un agujero de consistencia: un `.md` bajo `.lodestar/` que entrara en el
/// inventario sería nodo del grafo, resultado de `knowledge_search` y sujeto de `change_apply` —
/// y sería **ciego al control optimista**, porque `lodestar_core::types::workspace_revision`
/// excluye todo `.lodestar/` (decisión **D5**) y sus cambios nunca moverían la revisión. La
/// revisión **no puede** dejar de excluirlo: `StagingDir` materializa bajo
/// `.lodestar/runtime/staging/` copias `.md` de los documentos cuya escritura está guardando, así
/// que si contaran, `reverify_base_revision` fallaría *a causa del apply en curso* — el motor
/// transaccional invalidaría su propia base al preparar la escritura.
///
/// El agujero se cerró por el lado del descubrimiento, con `.lodestar/**` en el `exclude` por
/// defecto. Esta historia hace ese `exclude` **configurable**, y con ello reabriría el agujero de
/// la forma más tonta posible: un usuario que escriba `exclude: []` —o que liste sus propias
/// exclusiones sin repetir las de fábrica, que es lo natural— borraría la que sostiene el
/// invariante. Por eso `.lodestar/**` es un **suelo duro**: la config puede **añadir** exclusiones,
/// nunca quitar esa.
///
/// ## Cómo está construido el escenario
///
/// El YAML apaga **todo lo que sí es configurable**: `exclude: []` vacía la lista y
/// `respectGitignore: false` desactiva el otro filtro. Así el test distingue las dos mitades sin
/// ambigüedad:
///
/// - lo apagable **se apaga de verdad** (`vendor/dep.md`, ignorado por `.gitignore`, entra) — sin
///   esto, un implementador podría «cumplir» el suelo duro simplemente ignorando la sección
///   `discovery` entera, que es justo lo que hace el código de hoy;
/// - y aun con todo apagado, `.lodestar/` **sigue fuera**, en los tres sitios donde puede aparecer
///   un `.md` de control.
///
/// La última mitad comprueba el invariante que hay detrás y que sobrevive a cualquier cambio futuro
/// de la lista de globs: *todo documento del inventario cuenta para la revisión del workspace*. Se
/// verifica de la única forma observable desde fuera —tocando cada documento descubierto y
/// exigiendo que la revisión se mueva—, ahora bajo una política que viene del usuario.
#[test]
fn exclude_vacio_no_reabre_lodestar() {
    let dir = tempfile::tempdir().unwrap();
    lodestar_fixtures::materialize(&lodestar_fixtures::arbitrary(), dir.path()).unwrap();
    escribe(dir.path(), ".gitignore", "vendor/\n");
    escribe(dir.path(), "vendor/dep.md", "# Dependencia\n");

    // Los tres sitios donde puede aparecer un `.md` bajo el plano de control.
    let control = [
        // Entrada de la generación, no documento de la base.
        ".lodestar/templates/plantilla.md",
        // Un `.md` suelto en la raíz del directorio de control.
        ".lodestar/nota.md",
        // Copia de staging: el caso que hace imposible relajar D5.
        ".lodestar/runtime/staging/copia.md",
    ];
    for rel in control {
        escribe(dir.path(), rel, "# Fichero de control\n");
    }

    // El usuario apaga TODO lo que la config le permite apagar.
    escribe_config_yaml(
        dir.path(),
        "discovery:\n  exclude: []\n  respectGitignore: false\n",
    );

    let ws = Workspace::open(dir.path()).unwrap();
    let doc_set = ws.document_set().unwrap();
    let files = doc_set.files();

    // --- Mitad 1: lo apagable se apaga de verdad -------------------------------------
    assert!(
        contiene(files, "vendor/dep.md"),
        "`respectGitignore: false` debe obedecerse: si no, el suelo duro se estaría «cumpliendo» \
         por la vía de ignorar la sección `discovery` entera. Inventario: {:?}",
        rutas(files)
    );

    // --- Mitad 2: el suelo duro aguanta ----------------------------------------------
    for rel in control {
        assert!(
            !contiene(files, rel),
            "`.lodestar/` es el plano de control de Lodestar, no conocimiento del usuario: la \
             config NO puede reabrirlo ni con `exclude: []`. Sobra {rel}. Inventario: {:?}",
            rutas(files)
        );
    }
    assert!(
        ws.discovery_policy()
            .exclude
            .iter()
            .any(|g| g == ".lodestar/**"),
        "el suelo duro debe estar presente en la política EFECTIVA, se declare o no en el YAML; \
         era: {:?}",
        ws.discovery_policy().exclude
    );

    // --- Mitad 3: el invariante que hay detrás ---------------------------------------
    // Todo documento del inventario cuenta para la revisión del workspace. Si alguno no la
    // moviera, sería escribible por el motor transaccional y ciego al control optimista.
    let descubiertos: Vec<String> = rutas(files).into_iter().map(String::from).collect();
    assert!(
        !descubiertos.is_empty(),
        "precondición: el inventario no puede estar vacío"
    );
    let mut anterior = ws.workspace_revision().unwrap();
    for rel in descubiertos {
        let destino = dir.path().join(&rel);
        let mut contenido = std::fs::read_to_string(&destino).unwrap();
        contenido.push_str("\n<!-- tocado por el test -->\n");
        std::fs::write(&destino, contenido).unwrap();

        let actual = ws.workspace_revision().unwrap();
        assert_ne!(
            actual, anterior,
            "`{rel}` está en el inventario pero cambiarlo NO mueve la revisión del workspace: \
             sería un documento escribible al que el control optimista no protege"
        );
        anterior = actual;
    }
}
