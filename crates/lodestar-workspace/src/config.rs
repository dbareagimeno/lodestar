//! Configuración **por-workspace**: `<root>/.lodestar/config.yaml` (`ARCHITECTURE.md §20.5`, `§20.9`;
//! `decisiones §0` D4/D5).
//!
//! Desde E15-H08 es el **único** fichero de configuración del motor: el `lodestar.toml` legado
//! (`Config`/`GateConfig`) se borró —dos ficheros de config para lo mismo era deuda, y su otro
//! habitante (`identity`) murió en E15-H01—, de modo que un `lodestar.toml` en la raíz es hoy un
//! fichero más del proyecto: ni se lee, ni su sintaxis importa (cierra `decisiones §8`).
//!
//! La regla que gobierna todo lo que hay aquí es **la config LIMITA, nunca habilita**
//! (`ARCHITECTURE.md §20.1`): su ausencia no impide usar Lodestar (defaults seguros = los de
//! `§20.5`), lo que declara solo puede restringir, y un YAML malformado es un **error explícito**
//! —nunca una caída silenciosa a defaults, que relajaría las restricciones del usuario sin avisar.

use std::collections::BTreeMap;
use std::path::Path;

use lodestar_core::types::{Analysis, Check, CheckCode, RelPath, Severity};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use crate::discovery::{DiscoveryPolicy, CONTROL_PLANE_EXCLUDE};

/// Ruta del fichero de configuración, relativa al root del workspace.
pub const WORKSPACE_CONFIG_FILE: &str = ".lodestar/config.yaml";

/// Configuración efectiva de un workspace (`.lodestar/config.yaml`, YAML).
///
/// El mapeo YAML usa claves `camelCase` (`writableRoots`, `respectGitignore`, `blockWarnings`, …)
/// que se deserializan a los campos `snake_case` de estas structs. Todas las secciones son
/// opcionales y traen defaults seguros.
///
/// # Claves desconocidas (E29-H01)
///
/// Desde E29-H01 tanto esta struct como **todas** sus secciones llevan
/// `#[serde(deny_unknown_fields)]`: una clave que el motor no reconoce es un **error de config**, no
/// un descarte silencioso (`decisiones §16(e)`). El motivo es que el silencio afloja: un
/// `writeableRoots` (typo de `writableRoots`) dejaba la política de escritura en su default —`Vec`
/// vacío = *«todo el workspace es escribible»*—, o sea **más permisiva** que la que el usuario
/// escribió, sin decir una palabra.
///
/// La **única excepción declarada** es [`WorkspaceSection::root`], que se sigue deserializando y
/// descartando (ver su doc-comment): hay `config.yaml` reales que la llevan y `§20.5` ya declaró que
/// se ignora a propósito.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct WorkspaceConfig {
    /// Raíces de escritura/lectura del workspace (la *write policy* de `§20.1`).
    pub workspace: WorkspaceSection,
    /// Política de descubrimiento (`§20.5`): qué documentos forman el inventario.
    pub discovery: DiscoverySection,
    /// Política de validación (`§20.9`): severidad por familia de diagnóstico. Aplicada desde
    /// E20-H04 vía [`ValidationSection::effective_severity`].
    pub validation: ValidationSection,
    /// Puerta de conformidad (strictness de `lodestar check`).
    pub gate: GateSection,
    /// Política transaccional y retención del histórico de recibos (E13; la política de cambios de
    /// `§20.9` **solo se carga** aquí, su mecánica es E20).
    pub transactions: TransactionsSection,
    /// Presupuesto público de memoria retenida (`ARCHITECTURE.md §23`).
    pub performance: PerformanceSection,
}

/// Configuración de performance pública. E35-H01 mantiene una sola perilla: `maxMemory`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct PerformanceSection {
    /// Texto original del presupuesto, por ejemplo `256MiB`.
    pub max_memory: String,
}

impl Default for PerformanceSection {
    fn default() -> Self {
        Self {
            max_memory: "256MiB".to_string(),
        }
    }
}

impl PerformanceSection {
    /// Devuelve el presupuesto efectivo en bytes. `WorkspaceConfig::load` valida antes de
    /// exponer una configuración; el `expect` evita introducir una segunda ruta de error en el
    /// acceso posterior a una config ya validada.
    pub fn max_memory_bytes(&self) -> u64 {
        parse_max_memory(&self.max_memory).expect("performance.maxMemory debe estar validado")
    }

    fn validate(&self) -> Result<(), String> {
        parse_max_memory(&self.max_memory).map(|_| ())
    }
}

const MEMORY_GRAMMAR: &str = "[1-9][0-9]*(MiB|GiB)";
const MIN_MEMORY_BYTES: u64 = 64 * 1024 * 1024;

fn parse_max_memory(received: &str) -> Result<u64, String> {
    let (magnitude, factor, unit) = if let Some(value) = received.strip_suffix("MiB") {
        (
            value,
            1024_u64.checked_mul(1024).expect("factor constante"),
            "MiB",
        )
    } else if let Some(value) = received.strip_suffix("GiB") {
        (
            value,
            1024_u64
                .checked_mul(1024)
                .and_then(|v| v.checked_mul(1024))
                .expect("factor constante"),
            "GiB",
        )
    } else {
        return Err(format!(
            "performance.maxMemory recibió «{received}»: incumple la gramática {MEMORY_GRAMMAR}"
        ));
    };

    if magnitude.is_empty()
        || !magnitude.as_bytes().iter().all(u8::is_ascii_digit)
        || magnitude.starts_with('0')
    {
        return Err(format!(
            "performance.maxMemory recibió «{received}»: incumple la gramática {MEMORY_GRAMMAR}"
        ));
    }

    let amount = magnitude.parse::<u64>().map_err(|_| {
        format!(
            "performance.maxMemory recibió «{received}»: desbordamiento de u64 al convertir la magnitud"
        )
    })?;
    let bytes = amount.checked_mul(factor).ok_or_else(|| {
        format!(
            "performance.maxMemory recibió «{received}»: desbordamiento de u64 al convertir {unit}"
        )
    })?;
    if bytes < MIN_MEMORY_BYTES {
        return Err(format!(
            "performance.maxMemory recibió «{received}»: el mínimo es 64MiB ({} bytes)",
            MIN_MEMORY_BYTES
        ));
    }
    Ok(bytes)
}

/// Raíces de escritura/lectura del workspace (`ARCHITECTURE.md §20.1`).
///
/// > **`workspace.root` NO se implementa** (E15-H08, `§20.5`). `REFACTOR_PHASE_2 §Fase 2` lo
/// > sugería como configuración opcional, pero es **circular**: este fichero vive en
/// > `<root>/.lodestar/config.yaml`, luego hay que conocer ya la raíz para poder leerlo. La raíz
/// > sale **exclusivamente** de `--root` (o `--path`) o del cwd, y es fija durante toda la sesión.
/// > La clave se ignora si aparece en el YAML: no redirige nada.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct WorkspaceSection {
    /// **Excepción declarada al rechazo de claves desconocidas** (E29-H01): `workspace.root` se
    /// deserializa y se **descarta**.
    ///
    /// No es una capacidad a medias sino la forma de mantener el comportamiento que `§20.5`/E15-H08
    /// ya habían decidido —la clave se ignora porque es circular— ahora que
    /// `deny_unknown_fields` convertiría en error de arranque cualquier clave que la struct no
    /// declare. Hay `config.yaml` reales que la llevan: rechazarlos rompería workspaces vivos sin
    /// que nadie lo hubiera decidido.
    ///
    /// El dato se guarda para que `PartialEq`/`Debug` no mientan sobre lo que traía el YAML, pero
    /// **ningún** consumidor lo lee: la raíz sale exclusivamente de `--root`/`--path` o del cwd.
    pub root: Option<String>,
    /// Raíces donde Lodestar puede escribir (validado en E11-H04; aquí solo se carga el dato).
    ///
    /// **`Vec` vacío significa "todo el workspace es escribible"** (sin restricción) — no es una
    /// lista de cero raíces permitidas. No existe un valor centinela para "la raíz del workspace"
    /// porque `RelPath::new(".")` es inválido (`.` se normaliza a "sin componentes" y `RelPath`
    /// rechaza la cadena vacía resultante); representar "todo" como ausencia de restricción evita
    /// ese valor imposible.
    pub writable_roots: Vec<RelPath>,
    /// Raíces visibles para validación pero **nunca** escribibles por Lodestar (p. ej. `src`,
    /// `tests` de un repo de código adoptado). Vacío por defecto. Se retira en E20 con las refs
    /// externas por frontmatter.
    pub reference_roots: Vec<RelPath>,
    /// Rutas (relativas al root, no necesariamente `RelPath` válidos si describen directorios
    /// arbitrarios de un repo adoptado) que el walker ignora. `#[serde(default)]` **reemplaza**
    /// la lista entera cuando el YAML trae `ignored` propio (no hace merge), así que el
    /// deserializado en crudo puede no traer los obligatorios. [`WorkspaceConfig::load`] los
    /// inyecta siempre tras deserializar (merge + dedupe) — el campo `ignored` que ve cualquier
    /// consumidor de `WorkspaceConfig` (tras `load`) SIEMPRE incluye `.lodestar/runtime` y
    /// `.git`, se hayan especificado o no en el YAML.
    pub ignored: Vec<String>,
}

impl Default for WorkspaceSection {
    fn default() -> Self {
        WorkspaceSection {
            root: None,
            writable_roots: Vec::new(),
            reference_roots: Vec::new(),
            ignored: default_ignored(),
        }
    }
}

fn default_ignored() -> Vec<String> {
    vec![".lodestar/runtime".to_string(), ".git".to_string()]
}

/// Sección `discovery` (`ARCHITECTURE.md §20.5`): la política de descubrimiento declarada por el
/// usuario, antes de aplicarle el **suelo duro**.
///
/// Sus defaults son, campo a campo, los de [`DiscoveryPolicy::default`] —se derivan de ella, no se
/// reescriben— para que escribir la política por defecto documentada en `§20.5` dentro del
/// `config.yaml` dé exactamente el mismo comportamiento que no escribir nada. Si divergieran,
/// declarar los valores «de fábrica» cambiaría el descubrimiento: una config que *habilita* en vez
/// de limitar.
///
/// La política **efectiva** se obtiene con [`DiscoverySection::policy`], que es donde se inyecta el
/// suelo duro `.lodestar/**`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct DiscoverySection {
    /// Globs de lo que **entra** en el inventario (por defecto `**/*.md`).
    pub include: Vec<String>,
    /// Globs de lo que queda **fuera**, con prioridad sobre `include`.
    ///
    /// Lo que el usuario escriba aquí **reemplaza** la lista por defecto (no hace merge), con una
    /// única excepción innegociable: `.lodestar/**` (ver [`DiscoverySection::policy`]).
    pub exclude: Vec<String>,
    /// Aplicar los `.gitignore` del árbol (por defecto `true`).
    pub respect_gitignore: bool,
    /// Aplicar los `.lodestarignore` del árbol (por defecto `true`).
    pub respect_lodestar_ignore: bool,
    /// Seguir symlinks (por defecto `false`: se reportan con `SYMLINK-UNSUPPORTED`).
    pub follow_symlinks: bool,
    /// Tamaño máximo por documento en bytes; por encima se reporta `DOC-TOO-LARGE`.
    pub max_document_bytes: usize,
}

impl Default for DiscoverySection {
    fn default() -> Self {
        // Derivada de la política del motor: una sola fuente de verdad para los defaults de `§20.5`.
        let p = DiscoveryPolicy::default();
        DiscoverySection {
            include: p.include,
            exclude: p.exclude,
            respect_gitignore: p.respect_gitignore,
            respect_lodestar_ignore: p.respect_lodestar_ignore,
            follow_symlinks: p.follow_symlinks,
            max_document_bytes: p.max_document_bytes,
        }
    }
}

impl DiscoverySection {
    /// La [`DiscoveryPolicy`] **efectiva**: lo declarado por el usuario con el **suelo duro**
    /// [`CONTROL_PLANE_EXCLUDE`] (`.lodestar/**`) inyectado siempre.
    ///
    /// El suelo duro vive aquí —en la construcción de la política, no en el default de la
    /// sección— porque un default es sobreescribible por definición: un usuario que escriba
    /// `exclude: []`, o que liste sus propias exclusiones sin repetir las de fábrica (lo natural),
    /// se llevaría por delante la exclusión que sostiene un invariante del motor. Inyectándolo al
    /// construir la política, **toda** vía de obtención (config deserializada, `default()`,
    /// construida a mano) la lleva.
    ///
    /// El invariante que protege (`§20.5`, corrección E15-H07): *todo documento del inventario
    /// tiene que contar para la [`lodestar_core::types::workspace_revision`]*. Un `.md` bajo
    /// `.lodestar/` sería nodo del grafo, analizable y escribible, pero **ciego al control
    /// optimista** —la revisión excluye `.lodestar/` por decisión **D5** y no puede dejar de
    /// hacerlo: `StagingDir` materializa ahí copias `.md` de los documentos cuya escritura está
    /// guardando, así que si contaran, `reverify_base_revision` fallaría *a causa del apply en
    /// curso*. `.lodestar/` es el plano de control de Lodestar (config, cache, runtime), nunca
    /// conocimiento del usuario.
    ///
    /// La config puede, por tanto, **añadir** exclusiones; nunca quitar esa.
    pub fn policy(&self) -> DiscoveryPolicy {
        let mut exclude = self.exclude.clone();
        if !exclude.iter().any(|g| g == CONTROL_PLANE_EXCLUDE) {
            exclude.push(CONTROL_PLANE_EXCLUDE.to_string());
        }
        DiscoveryPolicy {
            include: self.include.clone(),
            exclude,
            respect_gitignore: self.respect_gitignore,
            respect_lodestar_ignore: self.respect_lodestar_ignore,
            follow_symlinks: self.follow_symlinks,
            max_document_bytes: self.max_document_bytes,
        }
    }
}

/// Sección `validation` (`ARCHITECTURE.md §20.9`): severidad por **familia de diagnóstico**
/// (`malformedFrontmatter: error`, `isolatedDocuments: ignore`, …).
///
/// # Lista cerrada de familias (E29-H01)
///
/// Hasta E29-H01 era un mapa **abierto a propósito**, y esa apertura era una mentira barata: una
/// clave que no fuera ninguna de las cinco familias de `§20.9` se cargaba sin rechistar y luego no
/// casaba con nada en `family_of`, de modo que el silenciado quedaba **silenciosamente inerte**.
/// El caso real (G1-04 del testbench, `decisiones §23/A-08`) es escribir un **código** de
/// diagnóstico donde el motor espera una **familia**: `"LINK-TARGET-MISSING": ignore` — el usuario
/// cree haber apagado el diagnóstico y lo sigue viendo en cada `check`.
///
/// Desde E29-H01 la clave se valida contra [`VALIDATION_FAMILIES`] y una familia desconocida es un
/// **error de config** cuyo mensaje **enumera las cinco válidas**: rechazar sin decir qué se admite
/// cambiaría un silencio por un muro.
///
/// El dato se sigue conservando en un `BTreeMap` con la clave literal del YAML (no se normaliza ni
/// se convierte a un enum): [`effective_severity`](ValidationSection::effective_severity) lo
/// consulta por el nombre que devuelve `family_of`, y guardar la cadena mantiene una sola verdad de
/// los nombres —las constantes `FAMILY_*`— sin un segundo catálogo paralelo.
///
/// Desde **E20-H04** la política se **aplica** vía [`ValidationSection::effective_severity`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValidationSection {
    /// Familia de diagnóstico (tal cual aparece en el YAML) → severidad configurada.
    pub families: BTreeMap<String, ValidationSeverity>,
}

impl<'de> Deserialize<'de> for ValidationSection {
    /// Deserializa el mapa `familia → severidad` **validando cada clave** contra
    /// [`VALIDATION_FAMILIES`] (E29-H01).
    ///
    /// Se escribe a mano en vez de con `#[serde(transparent)]` + un enum de claves porque el enum
    /// obligaría a duplicar los nombres de las familias en un tercer sitio (las constantes
    /// `FAMILY_*`, el enum y el catálogo del mensaje de error). Deserializando al `BTreeMap` y
    /// validando después, la lista vive **solo** en `VALIDATION_FAMILIES`, que a su vez se construye
    /// desde las constantes.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let families = BTreeMap::<String, ValidationSeverity>::deserialize(d)?;
        if let Some(desconocida) = families.keys().find(|k| !es_familia_valida(k)) {
            return Err(D::Error::custom(format!(
                "«{desconocida}» no es una familia de validación de `§20.9`. Las claves de \
                 `validation` son FAMILIAS, no códigos de diagnóstico. Familias válidas: {}",
                VALIDATION_FAMILIES.join(", ")
            )));
        }
        Ok(ValidationSection { families })
    }
}

/// ¿Es `clave` una de las familias de validación que `§20.9` declara?
fn es_familia_valida(clave: &str) -> bool {
    VALIDATION_FAMILIES.contains(&clave)
}

/// Familia `malformedFrontmatter` (`§20.9`): frontmatter no interpretable. Cubre `FM-UNCLOSED` y
/// `FM-YAML-INVALID`. Default: `error`.
pub const FAMILY_MALFORMED_FRONTMATTER: &str = "malformedFrontmatter";
/// Familia `danglingDocumentLinks` (`§20.9`): enlace a un **documento** Markdown inexistente
/// (`LINK-TARGET-MISSING` cuyo destino ausente sería un `.md`). Default: `error`.
pub const FAMILY_DANGLING_DOCUMENT_LINKS: &str = "danglingDocumentLinks";
/// Familia `missingWorkspaceFiles` (`§20.9`): enlace a un **fichero del proyecto** (no `.md`)
/// inexistente (`LINK-TARGET-MISSING` cuyo destino ausente no sería un documento). Default:
/// `warning`.
pub const FAMILY_MISSING_WORKSPACE_FILES: &str = "missingWorkspaceFiles";
/// Familia `caseMismatch` (`§20.9`): capitalización no portable (`LINK-CASE-MISMATCH`, venga del
/// descubrimiento o de un enlace). Default: `warning`.
pub const FAMILY_CASE_MISMATCH: &str = "caseMismatch";
/// Familia `isolatedDocuments` (`§20.9`): documentos sin enlaces internos entrantes ni salientes.
/// Default: `ignore`.
///
/// **No tiene productor** desde E16-H02: el documento aislado dejó de ser un diagnóstico (el código
/// `ORPHAN` murió y pasó a ser una propiedad consultable), así que su default `ignore` es un no-op y
/// `family_of` nunca la devuelve. Aun así se **acepta** en la config: `§20.9` la declara, y
/// rechazar una familia que el contrato publica rompería `config.yaml` reales por una asimetría
/// interna del motor. Es exactamente el motivo por el que esta constante existe aparte de
/// `family_of`.
pub const FAMILY_ISOLATED_DOCUMENTS: &str = "isolatedDocuments";

/// Las **cinco** familias de validación que `§20.9` declara — la lista cerrada contra la que se
/// valida cada clave de la sección `validation` (E29-H01), y el catálogo que se enumera al usuario
/// cuando escribe una que no está.
///
/// Se construye **desde** las constantes `FAMILY_*` para que los nombres vivan en un solo sitio: si
/// una familia se renombra, el mensaje de error y la validación se mueven con ella sin tocar nada
/// más. El orden es el de `§20.9` (las cuatro con productor y, al final, `isolatedDocuments`), no
/// el alfabético: es el orden en que se lee en la documentación de usuario.
pub const VALIDATION_FAMILIES: [&str; 5] = [
    FAMILY_MALFORMED_FRONTMATTER,
    FAMILY_DANGLING_DOCUMENT_LINKS,
    FAMILY_MISSING_WORKSPACE_FILES,
    FAMILY_CASE_MISMATCH,
    FAMILY_ISOLATED_DOCUMENTS,
];

/// La **familia de diagnóstico** (`§20.9`) a la que pertenece un [`Check`], o `None` si su código
/// no está gobernado por ninguna familia configurable (su severidad es intrínseca y no se puede
/// reclasificar desde `validation`).
///
/// `LINK-TARGET-MISSING` se reparte en **dos** familias según la naturaleza del destino ausente
/// (`related[0]`): un documento Markdown → `danglingDocumentLinks`; otro fichero del proyecto →
/// `missingWorkspaceFiles`. Es el **mismo discriminador** ([`RelPath::is_markdown`]) con el que
/// `links::diagnose` asigna la severidad hardcodeada, de modo que aplicar el default no cambia nada.
///
/// La familia [`FAMILY_ISOLATED_DOCUMENTS`] de `§20.9` **no** aparece aquí: el documento aislado
/// dejó de ser un diagnóstico (el código `ORPHAN` murió en E16-H02, es una propiedad consultable).
/// Su default `ignore` es, por tanto, un no-op — no hay nada que suprimir. Esto **no** la saca de
/// [`VALIDATION_FAMILIES`]: la lista cerrada de la config es la de `§20.9`, no la de los
/// productores vivos (E29-H01).
fn family_of(check: &Check) -> Option<&'static str> {
    match check.code {
        CheckCode::FmUnclosed | CheckCode::FmYamlInvalid => Some(FAMILY_MALFORMED_FRONTMATTER),
        CheckCode::LinkCaseMismatch => Some(FAMILY_CASE_MISMATCH),
        CheckCode::LinkTargetMissing => {
            let markdown = check.related.first().is_some_and(RelPath::is_markdown);
            Some(if markdown {
                FAMILY_DANGLING_DOCUMENT_LINKS
            } else {
                FAMILY_MISSING_WORKSPACE_FILES
            })
        }
        // `DOC-CONFLICT-MARKER`, `DOC-NOT-UTF8`, `DOC-TOO-LARGE`, `PATH-NOT-UTF8`,
        // `SYMLINK-UNSUPPORTED`, `LINK-ESCAPES-WORKSPACE`: fuera de las 5 familias de `§20.9`, su
        // severidad no es configurable.
        _ => None,
    }
}

impl ValidationSection {
    /// La **severidad efectiva** de `check` bajo esta política (`§20.9`), o `None` si la familia
    /// configurada lo **suprime** (`ignore`).
    ///
    /// - Familia configurada a `error`/`warning`/`ignore` → `Err`/`Warn`/`None`, sea cual sea el
    ///   productor del diagnóstico (reclasifica **cada** diagnóstico de esa familia).
    /// - Familia **no** mencionada en la config, o código **sin familia** (`family_of` devuelve
    ///   `None`) → se conserva la severidad intrínseca que trae el [`Check`]. Como los defaults de
    ///   `§20.9` coinciden con las severidades hardcodeadas, no declarar `validation` no cambia nada.
    pub fn effective_severity(&self, check: &Check) -> Option<Severity> {
        match family_of(check).and_then(|f| self.families.get(f)) {
            Some(ValidationSeverity::Error) => Some(Severity::Err),
            Some(ValidationSeverity::Warning) => Some(Severity::Warn),
            Some(ValidationSeverity::Ignore) => None,
            None => Some(check.level),
        }
    }
}

/// Severidad configurable de una familia de diagnóstico (`§20.9`). **Solo dato**: quien la aplique
/// es E20.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ValidationSeverity {
    /// El diagnóstico es un error.
    Error,
    /// El diagnóstico es un aviso.
    Warning,
    /// El diagnóstico no se reporta.
    Ignore,
}

/// Puerta de conformidad: strictness de `lodestar check` (`ARCHITECTURE.md §7.3`).
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct GateSection {
    /// Si `true`, los avisos (`Warn`) también hacen fallar la puerta (además de los errores).
    pub block_warnings: bool,
}

/// Política transaccional (`§20.9`) y retención del histórico de recibos (mecánica de la retención
/// en E13; la de `rejectNewErrors`/`allowExistingErrors`, en E20 — aquí solo el dato de config).
///
/// Tipos deliberadamente simples (`String`/`usize`): la unidad de `retain_receipts_for` (p. ej.
/// `"24h"`) la interpreta quien implemente la retención, no este loader.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct TransactionsSection {
    /// Durante cuánto tiempo se retiene un recibo antes de purgarlo (p. ej. `"24h"`).
    pub retain_receipts_for: String,
    /// Número máximo de recibos retenidos simultáneamente.
    pub maximum_receipts: usize,
    /// Un cambio no puede introducir errores nuevos ni empeorar los existentes (por defecto
    /// `true`). Aplicada en el gate diferencial de [`crate::Workspace::validate_staging`] (E20-H04).
    pub reject_new_errors: bool,
    /// Lodestar puede trabajar en un repositorio que ya tiene problemas, y una reparación parcial
    /// se puede aplicar (por defecto `true`). Aplicada en el gate diferencial de
    /// [`crate::Workspace::validate_staging`] (E20-H04).
    pub allow_existing_errors: bool,
}

impl Default for TransactionsSection {
    fn default() -> Self {
        TransactionsSection {
            retain_receipts_for: "24h".to_string(),
            maximum_receipts: 20,
            reject_new_errors: true,
            allow_existing_errors: true,
        }
    }
}

impl WorkspaceConfig {
    /// Carga `<root>/.lodestar/config.yaml` si existe; si no, devuelve los defaults seguros (la
    /// ausencia de fichero **no** es un error: `§20.1`, arranque sin ceremonia). YAML malformado,
    /// una clave desconocida, una familia de `validation` fuera de [`VALIDATION_FAMILIES`], o un
    /// `writableRoots`/`referenceRoots` con un componente inválido (p. ej. `..`, rechazado por
    /// `RelPath`), sí es un error explícito — nunca se silencia a defaults.
    ///
    /// # Ausente frente a ilegible (E29-H01)
    ///
    /// Solo [`std::io::ErrorKind::NotFound`] cae a [`WorkspaceConfig::default`]. **Cualquier otro**
    /// error de lectura —permisos, un directorio en lugar del fichero, un disco que falla— es
    /// `Err`: hasta E29-H01 un `Err(_)` a secas los igualaba todos al caso legítimo, de modo que un
    /// usuario con una política escrita y no aplicada no se enteraba. La distinción es fina y es
    /// toda la diferencia: **ausente** es un estado válido y permanente; **ilegible** significa que
    /// hay una política declarada que Lodestar no está obedeciendo.
    ///
    /// Tras deserializar, inyecta siempre los obligatorios (`.lodestar/runtime`, `.git`) en
    /// `workspace.ignored` (merge + dedupe): `#[serde(default)]` reemplaza la lista entera cuando
    /// el YAML trae la suya, así que sin esta inyección un `ignored` explícito del usuario se
    /// comería los obligatorios. El suelo duro del **descubrimiento** no se inyecta aquí sino en
    /// [`DiscoverySection::policy`], para que lo lleve toda vía de construcción de la política.
    pub fn load(root: &Path) -> Result<WorkspaceConfig, String> {
        let path = root.join(WORKSPACE_CONFIG_FILE);
        let mut cfg = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let cfg = serde_yaml::from_str::<WorkspaceConfig>(&text)
                    .map_err(|e| format!("{WORKSPACE_CONFIG_FILE} inválido: {e}"))?;
                cfg.performance
                    .validate()
                    .map_err(|e| format!("{WORKSPACE_CONFIG_FILE} inválido: {e}"))?;
                cfg
            }
            // Ausente: el único caso legítimo (`§20.1`, arranque sin ceremonia).
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => WorkspaceConfig::default(),
            // Ilegible: existe una config que el usuario escribió y que no se está aplicando.
            Err(e) => {
                return Err(format!(
                    "{WORKSPACE_CONFIG_FILE} no se pudo leer: {e}. El fichero existe pero no es \
                     legible; Lodestar NO cae a los valores por defecto, porque eso aplicaría una \
                     política distinta de la declarada sin avisar"
                ))
            }
        };
        for obligatorio in default_ignored() {
            if !cfg.workspace.ignored.contains(&obligatorio) {
                cfg.workspace.ignored.push(obligatorio);
            }
        }
        Ok(cfg)
    }

    /// `true` si la puerta de conformidad debe fallar para este análisis según la strictness
    /// configurada (`gate.blockWarnings`).
    ///
    /// Es lo que consume `lodestar check` sobre el veredicto del motor: la config solo puede
    /// **endurecer** la puerta (que los avisos también bloqueen), nunca relajarla.
    pub fn gate_blocked(&self, a: &Analysis) -> bool {
        a.hard_fail() > 0 || (self.gate.block_warnings && a.warn_count() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Las secciones que esta historia **solo carga** (`validation`, la política de cambios de
    /// `transactions`) se deserializan sin perder datos, con sus claves camelCase — y
    /// `workspace.root` se ignora sin tumbar el parseo (es circular: `§20.5`).
    #[test]
    fn secciones_solo_de_carga_se_deserializan_sin_perder_datos() {
        let yaml = "\
workspace:
  root: /otro/sitio
  writableRoots: [knowledge]
validation:
  malformedFrontmatter: error
  isolatedDocuments: ignore
  caseMismatch: warning
transactions:
  rejectNewErrors: false
  allowExistingErrors: true
";
        let cfg: WorkspaceConfig = serde_yaml::from_str(yaml).expect("YAML válido");

        // `workspace.root` no redirige nada: se ignora y el resto de la sección se carga igual.
        assert_eq!(cfg.workspace.writable_roots.len(), 1);

        assert_eq!(
            cfg.validation.families.get("malformedFrontmatter"),
            Some(&ValidationSeverity::Error)
        );
        assert_eq!(
            cfg.validation.families.get("isolatedDocuments"),
            Some(&ValidationSeverity::Ignore)
        );
        assert_eq!(
            cfg.validation.families.get("caseMismatch"),
            Some(&ValidationSeverity::Warning)
        );

        assert!(!cfg.transactions.reject_new_errors);
        assert!(cfg.transactions.allow_existing_errors);
        // Lo no declarado conserva su default (la sección no se reemplaza entera).
        assert_eq!(cfg.transactions.maximum_receipts, 20);
        assert_eq!(cfg.transactions.retain_receipts_for, "24h");
    }

    /// El suelo duro no depende de que el usuario lo declare, ni de qué más excluya.
    #[test]
    fn el_suelo_duro_sobrevive_a_cualquier_exclude() {
        for yaml in [
            "discovery:\n  exclude: []\n",
            "discovery:\n  exclude: [\"notas/**\"]\n",
            "discovery: {}\n",
            "{}\n",
        ] {
            let cfg: WorkspaceConfig = serde_yaml::from_str(yaml).expect("YAML válido");
            let policy = cfg.discovery.policy();
            assert!(
                policy.exclude.iter().any(|g| g == CONTROL_PLANE_EXCLUDE),
                "el suelo duro debe estar en la política efectiva de «{yaml}»: {:?}",
                policy.exclude
            );
            // …y sin duplicarlo cuando ya viene de los defaults.
            assert_eq!(
                policy
                    .exclude
                    .iter()
                    .filter(|g| *g == CONTROL_PLANE_EXCLUDE)
                    .count(),
                1
            );
        }
    }

    /// Una severidad fuera del catálogo de `§20.9` es un error de config, no un default silencioso.
    #[test]
    fn severidad_desconocida_es_error() {
        let res: Result<WorkspaceConfig, _> =
            serde_yaml::from_str("validation:\n  malformedFrontmatter: catastrofe\n");
        assert!(res.is_err(), "«catastrofe» no es una severidad válida");
    }
}
