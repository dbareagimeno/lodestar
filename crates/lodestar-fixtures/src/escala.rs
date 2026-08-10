//! Generador de corpus de **escala** compartido (E33-H01, `ARCHITECTURE.md §22.2`).
//!
//! El banco de evidencia (`§22`) necesita medir siempre sobre el **mismo** corpus: sin
//! reproducibilidad, dos corridas del gate de rendimiento no son comparables ni entre releases ni
//! entre máquinas. Este módulo es esa garantía — un generador **puro y determinista**: el árbol que
//! escribe depende **solo** de `(perfil, tamaño, semilla)`. Nada de `SystemTime::now()`, nada de
//! orden de iteración de `HashMap`, nada de rutas absolutas dentro del contenido.
//!
//! Dos perfiles, con propósitos distintos y deliberadamente opuestos:
//!
//! - [`Perfil::Plano`] — el corpus homogéneo de **E14-H05**
//!   (`crates/lodestar-app/tests/escala.rs`: `genera_workspace_grande`/`cuerpo_grande`), extraído
//!   aquí tal cual. Documentos idénticos en forma, **sin enlaces entre sí** y sin fauna de
//!   diagnósticos: es el suelo limpio contra el que se miden latencias, y se mantiene byte a byte
//!   para que las cifras históricas de E14-H05 sigan siendo comparables. Por eso —y solo aquí— la
//!   **semilla no influye**: variar el corpus plano rompería justamente esa comparabilidad.
//! - [`Perfil::Realista`] — corpus con **grafo** (enlaces resueltos entre documentos, documentos
//!   aislados), **frontmatter heterogéneo y consultable** (tipos mezclados, fechas, listas, campos
//!   de referencia) y tamaños de cuerpo desiguales. Medir solo sobre el plano mentiría: el coste
//!   real del motor está en resolver enlaces, computar backlinks y consultar frontmatter. Aquí la
//!   semilla **sí** manda: es un parámetro real del corpus.
//!
//! Regla heredada del repo: estos corpus se generan **en runtime** (un `tempdir` del test o del
//! bench) y **nunca** se commitean.

use std::path::Path;

/// Perfil del corpus de escala (`ARCHITECTURE.md §22.2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Perfil {
    /// Corpus homogéneo de E14-H05: documentos iguales en forma, sin enlaces ni diagnósticos.
    /// Insensible a la semilla, por comparabilidad con las cifras históricas.
    Plano,
    /// Corpus con grafo, huérfanos, frontmatter heterogéneo y fauna de diagnósticos de enlace.
    /// Sensible a la semilla.
    Realista,
}

/// Marca única enterrada al **final** del cuerpo de cada documento del perfil plano, lejos de
/// cualquier término buscable y más allá de la ventana del snippet (160 chars).
///
/// Es el centinela de «el payload no arrastra el cuerpo» de `bench_search_payload_acotado`
/// (E14-H05): si aparece en una respuesta de `knowledge_search`, un cuerpo completo viajó. Se
/// expone porque el test que lo juzga vive en otro crate y necesita la **misma** cadena.
pub const CENTINELA_CUERPO: &str = "CENTINELA-CUERPO-QUE-NO-DEBE-VIAJAR";

/// Genera bajo `root` un corpus de `tamano` documentos `.md` del `perfil` pedido, reproducible a
/// partir de `semilla`.
///
/// Escribe **exactamente** `tamano` ficheros `.md` (más el `index.md` del perfil plano, que forma
/// parte de sus `tamano`… ver abajo) creando los directorios intermedios. Dos llamadas con los
/// mismos `(perfil, tamano, semilla)` producen árboles **byte-idénticos**.
///
/// El reparto de los `tamano` documentos depende del perfil:
/// - [`Perfil::Plano`]: `index.md` mínimo + `tamano - 1` documentos `c/documento-NNNNN.md`.
/// - [`Perfil::Realista`]: `tamano` documentos repartidos en secciones temáticas, sin `index.md`
///   (un workspace Markdown universal no lo necesita, `ARCHITECTURE.md §20`).
///
/// # Errores
/// Propaga cualquier error de I/O (crear directorio o escribir fichero).
///
/// # Pánico
/// Si `tamano` es 0: un corpus vacío no es medible y sería un error de llamada, no un caso a
/// modelar.
pub fn genera(root: &Path, perfil: Perfil, tamano: usize, semilla: u64) -> std::io::Result<()> {
    assert!(
        tamano > 0,
        "el corpus de escala necesita al menos un documento"
    );
    match perfil {
        Perfil::Plano => genera_plano(root, tamano),
        Perfil::Realista => genera_realista(root, tamano, semilla),
    }
}

/// Escribe un fichero bajo `root`, creando los directorios intermedios.
fn escribe(root: &Path, rel: &str, contenido: &str) -> std::io::Result<()> {
    let ruta = root.join(rel);
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(ruta, contenido)
}

// ---------------------------------------------------------------------------
// Perfil plano — el corpus de E14-H05, extraído sin cambiar un byte
// ---------------------------------------------------------------------------

/// Cuerpo grande (~2 KB) del perfil plano: arranque con el término buscable «documento», mucho
/// relleno, y el [`CENTINELA_CUERPO`] al final (bien pasado el snippet window de 160 chars).
///
/// Se expone porque `bench_search_payload_acotado` (E14-H05) reconstruye con él el tamaño del
/// cuerpo completo que cada resultado representa, para aseverar la cota de payload.
pub fn cuerpo_plano(i: usize) -> String {
    let relleno = "Contenido de relleno sintetico para dar cuerpo al documento. ".repeat(40);
    format!(
        "# Documento {i}\n\nEste documento sintetico numero {i} describe un tema de prueba.\n\n{relleno}\n\n{CENTINELA_CUERPO}-{i}\n"
    )
}

/// Corpus homogéneo: un `index.md` mínimo (no lista los documentos: listar N enlaces solo
/// ralentizaría sin cambiar el conjunto que casa) y `tamano - 1` documentos idénticos en forma,
/// **sin enlaces** entre sí. Cada documento casa el término «documento» por título, descripción y
/// cuerpo.
fn genera_plano(root: &Path, tamano: usize) -> std::io::Result<()> {
    escribe(
        root,
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle grande\n",
    )?;
    // El `index.md` es uno de los `tamano` documentos del corpus: el contrato del generador es
    // «`tamano` ficheros `.md` bajo `root`», no «`tamano` documentos además del índice».
    for i in 0..(tamano - 1) {
        escribe(
            root,
            &format!("c/documento-{i:05}.md"),
            &format!(
                "---\ntype: Concept\ntitle: Documento {i}\ndescription: documento sintetico numero {i}\n---\n\n{}",
                cuerpo_plano(i)
            ),
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Perfil realista — grafo, huérfanos, frontmatter heterogéneo y fauna
// ---------------------------------------------------------------------------

/// PRNG determinista y portable (SplitMix64): misma semilla ⇒ misma secuencia en cualquier
/// máquina y cualquier versión de Rust.
///
/// Se implementa aquí, con nueve líneas, en vez de tirar de `rand`: el corpus del banco tiene que
/// ser reproducible entre releases, y un cambio de algoritmo interno de la dependencia cambiaría
/// el corpus en silencio.
struct Aleatorio(u64);

impl Aleatorio {
    /// Siguiente valor de la secuencia.
    fn siguiente(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Entero en `[0, n)`. `n` debe ser > 0.
    fn hasta(&mut self, n: usize) -> usize {
        (self.siguiente() % n as u64) as usize
    }
}

/// Secciones temáticas del corpus realista: dan profundidad de árbol y variedad de `type:`.
const SECCIONES: &[(&str, &str)] = &[
    ("guias", "guia"),
    ("equipos", "equipo"),
    ("decisiones", "decision"),
    ("notas", "nota"),
];

/// Estados posibles del campo `status:` (frontmatter consultable, valores repetidos a propósito
/// para que `metadata_inspect` vea cardinalidad baja).
const ESTADOS: &[&str] = &["activo", "pendiente", "archivado"];

/// Etiquetas del campo `tags:` (lista, para ejercitar el camino de listas del evaluador).
const ETIQUETAS: &[&str] = &["infra", "producto", "seguridad", "datos", "onboarding"];

/// Ruta relativa del documento `i` del corpus realista. El nombre es función pura de `i` (no de la
/// semilla) para que los enlaces se puedan calcular sin materializar el corpus entero.
fn ruta_realista(i: usize) -> String {
    let (dir, _) = SECCIONES[i % SECCIONES.len()];
    format!("{dir}/doc-{i:05}.md")
}

/// Corpus con grafo, huérfanos y fauna de diagnósticos.
///
/// Estructura (todo determinista a partir de `semilla`):
/// - Los documentos se reparten en cuatro secciones temáticas y llevan frontmatter heterogéneo:
///   `title`/`type`/`status`/`priority` (número **o** cadena, tipos mezclados a propósito),
///   `updated` (fecha), `tags` (lista) y, en algunos, `relacionadas` (campo de referencia).
/// - **Aristas**: cada documento «conectado» enlaza a entre uno y tres documentos existentes,
///   elegidos con el PRNG. Es lo que da al banco un grafo con backlinks reales que resolver.
/// - **Huérfanos**: un tramo de documentos (`aislados`) no enlaza a nadie y nadie los enlaza —los
///   candidatos a destino se eligen siempre fuera de ese tramo—, así `graph_query isolated` tiene
///   qué devolver.
/// - **Fauna de enlaces**: unos pocos documentos llevan además un enlace roto a un fichero del
///   proyecto (`LINK-TARGET-MISSING`, `Warn`) y otro a un documento inexistente
///   (`LINK-TARGET-MISSING`, `Err`). Los destinos fantasma viven bajo `zz-inexistente/` **a
///   propósito**: al ordenar los nodos del grafo por `id`, quedan detrás de todos los documentos
///   reales, de modo que una página de `components` del tamaño del corpus conserva íntegras las
///   aristas entre documentos.
/// - **Tamaños heterogéneos**: la longitud del cuerpo varía con el PRNG (de un párrafo a varios
///   miles de caracteres).
///
/// # Precisión sobre `§22.2` (E33-H01, honestidad de lo entregado)
///
/// `ARCHITECTURE.md §22.2` describe el perfil realista como de «distribución de enlaces y
/// frontmatter **modelada sobre corpus reales**». Lo que esta función entrega es más modesto y
/// conviene no confundirlo: una distribución **sintética y uniforme** (el PRNG sortea salientes,
/// estados y etiquetas con igual probabilidad), elegida para ser reproducible y barata, no
/// calibrada contra ninguna medición de un corpus real. Basta para lo que el banco necesita hoy
/// —que medir no se haga solo sobre el corpus plano, que haya enlaces que resolver y backlinks que
/// computar—, pero **no** reproduce la ley de grados de un corpus humano (que suele ser de cola
/// larga, no uniforme). Si una medición futura dependiera de esa forma, hay que calibrarla antes;
/// no está hecho.
fn genera_realista(root: &Path, tamano: usize, semilla: u64) -> std::io::Result<()> {
    // Un tramo final de documentos queda deliberadamente aislado (~14 %, es decir `tamano/7`), con
    // dos topes: al menos 1 —el corpus tiene que tener huérfanos— y **como mucho `tamano - 1`**,
    // para que siempre quede al menos un documento conectado.
    //
    // Con `tamano == 1` las dos exigencias son incompatibles (un único documento no puede estar a
    // la vez aislado y conectado): gana «conectado», porque un corpus realista sin un solo
    // documento en el grafo no es realista. Ese caso degenerado solo aparece en llamadas de
    // juguete; las escalas de `§22.2` son ~100/~1k/~10k.
    let aislados = (tamano / 7)
        .clamp(0, tamano - 1)
        .max(usize::from(tamano > 1));
    let conectados = tamano - aislados;

    let mut rng = Aleatorio(semilla ^ 0x5EED_C0DE_0033_0001);

    for i in 0..tamano {
        let (_, tipo) = SECCIONES[i % SECCIONES.len()];
        let ruta = ruta_realista(i);
        let profundidad = ruta.matches('/').count();
        let aislado = i >= conectados;

        // --- Frontmatter heterogéneo y consultable ---------------------------------------
        let estado = ESTADOS[rng.hasta(ESTADOS.len())];
        let etiqueta_a = ETIQUETAS[rng.hasta(ETIQUETAS.len())];
        let etiqueta_b = ETIQUETAS[rng.hasta(ETIQUETAS.len())];
        // Tipos MEZCLADOS a propósito (el caso `priority: "2"` del testbench): unos documentos
        // llevan la prioridad como número y otros como cadena, para que `metadata_inspect` vea un
        // campo con dos tipos y el evaluador tipado tenga algo que decir.
        let prioridad = if rng.hasta(4) == 0 {
            format!("\"{}\"", 1 + rng.hasta(5))
        } else {
            format!("{}", 1 + rng.hasta(5))
        };
        // Fecha determinista derivada del índice (nunca del reloj).
        let mes = 1 + (i % 12);
        let dia = 1 + (i % 28);
        let mut frontmatter = format!(
            "---\ntitle: Documento {i}\ntype: {tipo}\nstatus: {estado}\npriority: {prioridad}\n\
             updated: \"2026-{mes:02}-{dia:02}\"\ntags: [{etiqueta_a}, {etiqueta_b}]\n"
        );
        // Campo de REFERENCIA en el frontmatter (estilo `relacionadas:`), en uno de cada tres
        // documentos conectados: es lo que consultan las queries del banco.
        if !aislado && i % 3 == 0 && conectados > 1 {
            let otro = (i + 1 + rng.hasta(conectados - 1)) % conectados;
            frontmatter.push_str(&format!("relacionadas:\n  - {}\n", ruta_realista(otro)));
        }
        frontmatter.push_str("---\n\n");

        // --- Cuerpo: título, párrafos de tamaño heterogéneo y enlaces ---------------------
        let mut cuerpo =
            format!("# Documento {i}\n\nDocumento sintetico numero {i} de la seccion.\n\n");
        let parrafos = 1 + rng.hasta(6);
        for p in 0..parrafos {
            let repeticiones = 1 + rng.hasta(12);
            cuerpo.push_str(&format!(
                "Parrafo {p} del documento {i}. {}\n\n",
                "Texto de relleno con contenido buscable. ".repeat(repeticiones)
            ));
        }

        if !aislado {
            // Aristas reales: de uno a tres destinos existentes DENTRO del tramo conectado (así el
            // tramo aislado no recibe entrantes y sigue siendo huérfano).
            cuerpo.push_str("## Relacionados\n\n");
            let salientes = 1 + rng.hasta(3);
            for _ in 0..salientes {
                // Un auto-enlace no es una arista del grafo, así que en vez de SALTAR el sorteo
                // —que dejaría la sección «## Relacionados» vacía si todos cayeran en `i`, y
                // convertiría al documento en aislado sin quererlo— se desplaza al siguiente del
                // tramo conectado. Así cada documento conectado emite siempre al menos un enlace.
                let sorteo = rng.hasta(conectados);
                let destino = if sorteo == i && conectados > 1 {
                    (sorteo + 1) % conectados
                } else {
                    sorteo
                };
                // Solo queda un caso sin destino posible: `conectados == 1` (el único conectado no
                // puede enlazarse más que a sí mismo). Ahí la sección se queda sin enlaces, y es
                // correcto: no hay ningún otro documento al que apuntar.
                if destino == i {
                    continue;
                }
                cuerpo.push_str(&format!(
                    "- [Documento {destino}]({})\n",
                    relativo(&ruta, &ruta_realista(destino))
                ));
            }
            cuerpo.push('\n');

            // Fauna de diagnósticos de enlace, en una fracción reproducible de los documentos:
            //  · destino `.md` ausente  → LINK-TARGET-MISSING (Err)
            //  · destino no-`.md` ausente → LINK-TARGET-MISSING (Warn)
            // Los fantasmas cuelgan de `zz-inexistente/`, que ordena detrás de las secciones
            // reales (ver la nota de la doc de esta función).
            if i % 11 == 0 {
                let subida = "../".repeat(profundidad);
                cuerpo.push_str(&format!(
                    "Referencia pendiente: [borrador]({subida}zz-inexistente/borrador-{i:05}.md).\n"
                ));
            }
            if i % 13 == 0 {
                let subida = "../".repeat(profundidad);
                cuerpo.push_str(&format!(
                    "Script de apoyo: [tarea]({subida}zz-inexistente/tarea-{i:05}.sh).\n"
                ));
            }
        }

        escribe(root, &ruta, &format!("{frontmatter}{cuerpo}"))?;
    }

    Ok(())
}

/// Href **relativo** de `destino` visto desde el documento `origen` (ambos paths relativos a la
/// raíz del workspace, con `/` como separador).
///
/// Los enlaces del corpus se escriben relativos —y no raíz-absolutos— porque es la forma que
/// domina en los workspaces reales y la que ejercita el camino de normalización de `links::resolve`.
fn relativo(origen: &str, destino: &str) -> String {
    let subida = "../".repeat(origen.matches('/').count());
    format!("{subida}{destino}")
}
