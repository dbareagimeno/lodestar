# Banco de pruebas (`ARCHITECTURE.md §22`)

> **Estado (E33-H02).** El banco de **conformidad** está completo y es asertable: corpus canónico
> determinista (H01), runner con veredicto mecánico PASS/FAIL y exit code (H02), y un gate de **99
> casos** que corre contra el corpus con **0 FAIL**. Quedan fuera, por historia: los centinelas de
> `decisiones §22` (integridad referencial de frontmatter) y `§24` (caja/Unicode) (**H03**), el banco de **rendimiento** y sus umbrales (**H04**/**H05**), el
> dogfooding (**H06**), el enganche a release y CI (**H07**) y el paquete de evidencia para
> `decisiones §14` (**H08**). Ni una ruta de máquina en este directorio: el binario y el root son
> parámetros.

El banco separa dos piezas de vida distinta (`§22.1`):

1. **Banco de conformidad** (python, este directorio): casos esperado-vs-real contra el contrato
   MCP, ejecutados por el arnés JSON-RPC/stdio.
2. **Banco de rendimiento** (Rust): mide los servicios de `App` a tres escalas. Su corpus lo produce
   el **generador de escala** de `lodestar-fixtures` (ver abajo).

---

## Qué genera cada script

| Script | Qué produce | Determinista |
|---|---|---|
| `make_corpus.py DEST [--seed N]` | El **corpus canónico** de conformidad (92 `.md` en APFS): red temática con grafo, huérfanos y fauna de diagnósticos; frontmatter heterogéneo; los sets patológicos de `make_fixtures.py` que no escriben config; y las semillas de los centinelas de E33-H03. | Sí — misma semilla ⇒ mismo árbol |
| `make_fixtures.py SET DEST` | Inyecta **un** set patológico suelto (`mdi`, `chk_a`, `chk_b`, `gaps`, `cfg_*`, `ignore`, `dirlinks`, `kinds`, `warngate_*`, `refroots`) en un worktree existente. Es el que consumen los lotes históricos de `batches/`. | Sí (contenido literal) |
| `lodestar_harness.py` | El ejecutor JSON-RPC/stdio y el **runner** del gate: llamada suelta, lote (`--batch`), banco completo (`--run-all`) o `tools/list`. Emite veredicto y exit code. | — |
| `runner_expect.py` | El **evaluador del formato `expect`** (E33-H02): selector de paths, familias de aserción, invariantes entre pasos y veredicto por caso. No habla JSON-RPC. | — |
| `build_artifact.py` | Empaqueta los resultados de una campaña. | — |
| `selftest_runner.py` | El **autotest del runner** (E33-H02): corre `batches/meta_runner.json` y verifica el veredicto, el exit code y el detalle del FAIL. | — |

Para generar el Artifact desde cualquier directorio, declara el directorio que contiene las tres
matrices y la salida (los defaults son relativos al propio script):

```bash
python3 build_artifact.py --input-dir . --out /tmp/artifact_lodestar.html
```

El generador solo lee `matriz_r1.json`, `matriz_r2.json` y `matriz_r3.json` del `--input-dir` y
escribe el fichero indicado por `--out`.

### El contrato del runner asertable

La semántica exacta del formato `expect` —qué asevera cada clave, el selector de paths, el
veredicto, los exit codes y qué debe imprimir el resumen— está fijada en
[`FORMATO_EXPECT.md`](FORMATO_EXPECT.md), y **demostrada** por el lote de autotest
[`batches/meta_runner.json`](batches/meta_runner.json) (`META-01` casa, `META-02` discrepa a
propósito, `META-03` es exploratorio). El test ejecutable de la historia es:

```bash
./selftest_runner.py                       # genera un corpus efímero y lo borra
./selftest_runner.py --corpus DIR --keep   # contra un corpus ya generado
```

El runner ya implementa `expect`: `selftest_runner.py` está **en verde** con sus **12/12
comprobaciones**, y `runner_expect.py` es el evaluador del formato.

---

## Cómo se corre el banco

Requisito único: el binario MCP compilado. Sin `--binary` ni `LODESTAR_MCP_BIN`, se busca en
`target/release/lodestar-mcp` **relativo a la raíz del repo**, derivada de la ubicación de los
scripts — nunca una ruta absoluta.

```bash
cargo build --release -p lodestar-mcp   # y `-p lodestar-cli` si vas a correr el lote H
```

### 1. El gate completo (lo que se corre por release)

```bash
./make_corpus.py /tmp/corpus                                  # el campo de pruebas
python3 lodestar_harness.py --run-all --root-corpus /tmp/corpus --out /tmp/corrida.json
```

Sin `--root-corpus`, el runner **genera** un corpus efímero con `make_corpus.py` y lo borra al
terminar; pasarlo explícitamente solo ahorra el minuto de generarlo. `--out` escribe la evidencia
cruda (las respuestas del wire, caso a caso); stdout lleva el veredicto.

Exit codes: **0** sin FAIL · **1** con FAIL · **2** uso incorrecto · **3** el banco no pudo correr.
Son los cuatro que la CLI del producto congela, por coherencia.

### 2. Un lote suelto (lo normal mientras se escribe un caso)

```bash
python3 lodestar_harness.py --batch batches/gate_L9_check_a.json --root-corpus /tmp/corpus
python3 lodestar_harness.py --batch batches/meta_runner.json --root-corpus /tmp/corpus --incluir-demos
```

`--incluir-demos` añade los casos `gate: false`, que sin la bandera se omiten (`SKIP`). Es cómo se
ve morder al runner: `META-02` falla a propósito y el resumen nombra el subcampo que discrepó.

### 3. La campaña exploratoria (dogfooding contra conocimiento real)

Los lotes históricos del homelab (`batches/L*.json`, `G*.json`, `H*.json`, `verify_*.json` y las
matrices `matriz_r*.json`) se conservan como campaña repetible **fuera del gate**. Sus rutas
absolutas se sustituyeron por tokens, así que hoy corren contra cualquier root:

```bash
python3 lodestar_harness.py --root ~/mi-workspace --profile readonly --list-tools
python3 lodestar_harness.py --root ~/mi-workspace --profile readonly \
        --call knowledge_search '{"where": "has(tags)", "limit": 5}'
python3 lodestar_harness.py --batch batches/L1_typ.json --root ~/mi-workspace --out /tmp/l1.json
```

> **REGLA DURA, generalizada en E33-H02**: contra un root **declarado real** —`--root`, o un lote
> con `"root": "real"`— solo se admite `--profile readonly`; además, el preflight rechaza cualquier
> paso `shell`/`spawn` con exit 2 antes de abrir una sola sesión o ejecutar pasos. Para mutar hace
> falta un root **desechable**: un lote con
> `"root": "corpus"` (copia efímera del canónico) o `"root": "worktree"` (worktree git del root
> real, el mecanismo del homelab). El invariante que la sostiene está aseverado en el propio gate
> (`gate_L8` · `L8-READONLY-NO-MUTA`): una tanda de lecturas deja la `workspaceRevision` intacta.

### Tokens de los pasos `shell`/`spawn`

Ningún lote nombra una máquina. En el `cmd` de un paso se sustituyen por texto:

| Token | Qué es |
|---|---|
| `@root` | la raíz del workspace del caso (la copia efímera, el worktree o el root real) |
| `@repo` | la raíz de este repositorio |
| `@bin.mcp` | el binario `lodestar-mcp` en uso |
| `@bin.cli` | el binario `lodestar` (CLI); `--binary-cli` o `LODESTAR_CLI_BIN` lo cambian |
| `@testbench` | este directorio |

Los placeholders `@stepN.ruta.de.campos` de los `arguments` (encadenar un `changeSetId` de un paso
al siguiente) siguen funcionando igual que siempre.

---

## Qué cubre el gate, y qué no

**16 lotes · 99 casos**. La corrida histórica de 97 casos está datada en
[`../corrida-banco-2026-08-10.md`](../corrida-banco-2026-08-10.md); los dos casos añadidos después
fijan la semántica secuencial de `change_plan` y su publicación por `change_apply`.

| Lote | Qué asevera |
|---|---|
| `gate_L1_consulta` | lenguaje de consulta: namespaces reservados, type errors del orden, la asimetría `=` vs `>=` |
| `gate_L2_proyeccion` | `include` de `knowledge_search` y `knowledge_get`; lo no pedido no viaja; `sections` |
| `gate_L3_metadata` | `metadata_inspect` en sus dos modos y **las tres clases límite** del dialecto de dot-paths |
| `gate_L5_grafo` | las ocho operaciones de `graph_query`, `outgoing ≡ neighborhood(1,out)`, `impact_analyze` |
| `gate_L6_plan` | forma del plan, **hash determinista y versionado**, composición secuencial/no-op por terminal, guards de `delete`/`create`/`move`, `REVISION_CONFLICT` |
| `gate_L7_apply` | round trip **byte a byte**, replay secuencial en disco y la familia de conflictos completa |
| `gate_L8_readonly` | el perfil oculta **y** rechaza (-32602), y las lecturas no mutan |
| `gate_L9_check_a` | el catálogo de diagnósticos de contenido con sus severidades |
| `gate_L10_check_b` | patologías binarias/estructurales y la frontera daño-estructural vs daño-semántico |
| `gate_L11_scopes` | los cuatro scopes de `knowledge_check` y el umbral `minimumSeverity` |
| `gate_L12_robustez` | cursores firmados, cotas de `limit`, claves no declaradas, errores de protocolo |
| `gate_G_descubrimiento` | `.lodestarignore`, `writableRoots`/`referenceRoots`, config rota, `instructions` |
| `gate_H_cli_recuperacion` | **exit codes congelados** de la CLI y de `lodestar-mcp`, gate de avisos, recuperación |
| `gate_invariantes` | los transversales del informe §5: `where ≡ filter`, orden determinista, revisión del contenido |
| `gate_verify_g1` / `gate_verify_g2` | las repros `verify_*` de `decisiones §23` que no viven ya en un lote temático |
| `meta_runner` | el **autotest del runner** (BDD-1/BDD-2), fuera de `--run-all` |

**Cómo se eligió qué entra** (`§22.3`): entra lo que asevera **contrato estable** —códigos,
invariantes, formas de respuesta—, no lo que asevera el contenido de un corpus concreto. La base
son las repros `verify_*` de `decisiones §23`, los invariantes transversales del informe
[`../informe-homelab-2026-08-06.md`](../informe-homelab-2026-08-06.md) §5, y al menos un caso por
lote temático de la matriz original (L1–L12, G, H).

**Los hallazgos ya saldados se aseveran CORREGIDOS.** Un caso portado de `§23` no reproduce el bug
histórico: fija el statu quo del motor de hoy y cita su hallazgo de origen. Por ejemplo `M-01`
(revert de un `-revert`, que era un no-op destructivo) hoy se asevera como redo que funciona y
encadena (`gate_L7` · `L7-APL-REVERT-DEL-REVERT`), citando `E28-H01`.

**Qué NO cubre**, deliberadamente:

- **Rendimiento**: ni una latencia. Es `E33-H04`/`H05` y vive en Rust, no aquí.
- **Los centinelas de las decisiones abiertas** (`decisiones §22` integridad referencial de
  frontmatter, `§24` caja/Unicode): el corpus ya planta sus semillas en `centinelas/`, pero sus esperados los escribe
  `E33-H03`.
- **Conteos del inventario**: prohibidos por la regla de oro del formato (ver abajo).
- **El homelab**: dejó de ser el campo de pruebas del gate (`§22.2`). Sigue siendo dogfooding
  opcional por el modo `--root`.
- **La cache SQLite**: el producto no la lee (`decisiones §14`) y el banco no la mide.

---

## Cómo se añade un caso con `expect`

La semántica exacta de cada clave está en [`FORMATO_EXPECT.md`](FORMATO_EXPECT.md), que es el
contrato; esto es el camino corto.

1. **Elige el lote** por tema (o crea uno `gate_*.json` y añádelo a `LOTES_DEL_GATE` en
   `lodestar_harness.py` — la lista es literal a propósito: qué entra al gate es una decisión, no un
   glob del directorio).
2. **Escribe el caso**, con `descripcion` que cite su origen (hallazgo de `§23`, sección del
   informe, o el lote temático del que viene):

```jsonc
{
  "id": "L9-CHK-01-FM-UNCLOSED",
  "descripcion": "Origen: L9_chk_a.json CHK-01. Fuente: mcp.yml catálogo CheckCode.",
  "steps": [
    { "kind": "call", "tool": "knowledge_check",
      "arguments": { "scope": { "kind": "document", "ref": { "path": "fixtures/roto-unclosed.md" } } },
      "expect": {
        "describe": "Por qué se asevera esto — se IMPRIME cuando el caso falla.",
        "is_error": false,
        "equals": { "structured.diagnostics.0.code": "FM-UNCLOSED" },
        "length": { "structured.diagnostics": 1 },
        "matches": { "structured.diagnostics.0.id": "^diag:blake3:[0-9a-f]{64}$" }
      } }
    ,{ "kind": "call", "tool": "workspace_status", "arguments": {},
       "expect": { "is_error": false, "present": ["structured.workspaceRevision"] } }
    ,{ "kind": "call", "tool": "workspace_status", "arguments": {},
       "expect": { "is_error": false, "present": ["structured.workspaceRevision"] } }
  ],
  "expect": [
    { "invariant": "same", "steps": [1, 2], "path": "structured.workspaceRevision",
      "describe": "Invariante ENTRE dos lecturas workspace_status." }
  ]
}
```

3. **Córrelo y léelo fallar antes de creerte que pasa.** Un caso que nunca has visto en rojo no
   prueba nada — mútalo (cambia el esperado a algo falso), confirma el FAIL, deshaz la mutación.
4. **Los casos sin `expect` son exploratorios**: se ejecutan y se registran, no computan al
   veredicto. Es lo que permite que los lotes históricos sigan corriendo tal cual bajo el runner
   nuevo.

### La regla de oro de los esperados

> **Prohibido `equals` sobre conteos del inventario.** El corpus contiene dos pares de nombres que
> **colisionan en APFS y no en ext4** (caja y NFC/NFD), así que `counts.documents`,
> `counts.isolated`, `counts.dangling`, `totalApproximate` y el `summary` de scope workspace
> **dependen del sistema de ficheros**. Se aseveran con `min_length`/`type`/`present`, o con
> `equals` solo cuando el conteo es de un scope **documento** concreto. Igual con
> `revision`/`workspaceRevision`/`planHash`: nunca contra un literal, sino con `matches` de forma
> (`^blake3:[0-9a-f]{64}$`) e invariantes `same`/`differs` entre pasos.

Dos guardas del runner ayudan a que un esperado no se pierda en silencio:

- una **clave de `expect` desconocida** (un typo como `equals2`) es un **FAIL**, no una aserción que
  se cumple sola;
- una **clave repetida** en el mismo objeto JSON aborta el lote con exit 2, porque el parser se
  quedaría con la última y descartaría la primera —y con ella su aserción— sin decir nada.

### `gate: false` y el exit code

Contrato y runner coinciden: **un FAIL es un FAIL, venga de donde venga**. Si un caso se **ejecutó**
y falló, el runner sale con exit `1`, tenga `gate: true` o `gate: false` (`FORMATO_EXPECT.md` §5).

Lo que `gate: false` protege es la corrida por release, y lo protege **no ejecutando** el caso, no
perdonando su FAIL: sin `--incluir-demos` —el modo del gate y del CI— los casos de demostración ni
se corren (salen `SKIP`), así que jamás pueden ensuciar su exit code. Con `--incluir-demos` —el modo
pedagógico— el exit `1` es justamente lo que se quiere enseñar: si un FAIL de demostración se
tragara el exit code, `META-02` no demostraría mecánicamente nada.

### El corpus canónico (`make_corpus.py`)

```bash
./make_corpus.py /tmp/corpus                # semilla por defecto (0xE330001)
./make_corpus.py /tmp/corpus --seed 7       # otra semilla ⇒ otro corpus, igual de reproducible
./make_corpus.py /tmp/corpus --no-patologicos   # solo la red temática + centinelas
```

Contiene, a propósito y con esperados estables:

- **Grafo real**: enlaces resueltos entre secciones (`guias/`, `equipos/`, `decisiones/`, `notas/`),
  un tramo final de documentos **aislados** y `relacionadas:` en el frontmatter de algunos.
- **Fauna de diagnósticos**: `LINK-TARGET-MISSING` en sus dos niveles (`Err` a `.md`, `Warn` a
  fichero de proyecto), `LINK-CASE-MISMATCH`, `LINK-ESCAPES-WORKSPACE`, `FM-UNCLOSED`,
  `FM-YAML-INVALID`, `DOC-CONFLICT-MARKER`, `DOC-BOM`, `DOC-NOT-UTF8`, `DOC-TOO-LARGE`,
  `SYMLINK-UNSUPPORTED`.
- **Frontmatter consultable**: tipos mezclados (`priority` número en unos, cadena en otros), fechas,
  listas, nulos explícitos, claves con punto literal y una clave llamada `frontmatter`.
- **Centinelas de E33-H03** (`centinelas/`): una referencia de frontmatter rota (`relacionadas:`
  apunta a un documento inexistente — el grafo solo mira enlaces del cuerpo, así que hoy no es
  diagnóstico ni arista), un par de paths que difieren **solo** en caja, y un par NFC/NFD del mismo
  nombre lógico (`canción.md`) cuya **única** diferencia es la forma Unicode. Ver la advertencia de
  portabilidad de abajo: los dos pares colisionan en APFS y no en ext4.

**Advertencia de portabilidad, deliberada**: los dos centinelas de colisión de nombres se comportan
distinto según el filesystem. Medido en APFS (macOS) el 2026-08-10:

| Centinela | APFS (macOS) | ext4 (Linux) |
|---|---|---|
| Caja: `centinelas/caja/informe.md` + `Informe.md` | **1** fichero (`informe.md`, contenido del segundo escritor) | 2 ficheros |
| Unicode: `centinelas/unicode/canción.md` en NFC + NFD | **1** fichero `canción.md` (contenido del que escribió en NFD); `exists()` responde `True` por las dos formas | 2 ficheros |

El nº total de documentos depende, por tanto, del sistema de ficheros — en esta máquina el corpus
completo son **92** `.md`. Los esperados del banco (H02/H03) se escriben sobre el comportamiento
observado, nunca sobre un conteo absoluto, y por eso el hash recursivo de abajo solo se compara
**entre corridas de la misma máquina**.

Lo mismo aplica al fichero de >10 MiB (`fixtures/gigante.md`) de `chk_b`: el corpus con patológicos
pesa ~11 MB. Se genera en runtime y **nunca** se commitea.

### Verificar el determinismo

El criterio estructural de E33-H01: dos corridas con la misma semilla producen el mismo árbol.

```bash
./make_corpus.py /tmp/corpus-a && ./make_corpus.py /tmp/corpus-b
diff -r /tmp/corpus-a /tmp/corpus-b && echo "idénticos"
```

Con **hash recursivo** (un solo número que comparar entre máquinas o entre releases; incluye los
symlinks por su destino, no por el contenido apuntado):

```bash
huella() {
  ( cd "$1" && find . \( -type f -o -type l \) | LC_ALL=C sort | while read -r f; do
      if [ -L "$f" ]; then printf '%s SYMLINK %s\n' "$f" "$(readlink "$f")"
      else printf '%s %s\n' "$f" "$(shasum -a 256 "$f" | cut -d' ' -f1)"; fi
    done ) | shasum -a 256 | cut -d' ' -f1
}
huella /tmp/corpus-a; huella /tmp/corpus-b   # deben coincidir
```

Comprobado en la máquina de desarrollo (2026-08-10, semilla por defecto):
`8a188cb6d51d26028fc4e30ee7d77ad25262335b9c87037ec0f01827ec45982f` en dos corridas consecutivas, y
un hash distinto (`97bfa2c3…`) con `--seed 7`. El valor absoluto **depende del sistema de ficheros**
(ver la advertencia de portabilidad de arriba): lo que se verifica es que las dos corridas de la
misma máquina coincidan.

---

## El generador de escala (Rust)

Vive en `crates/lodestar-fixtures/src/escala.rs` — no en un test ni en el core (invariante #2) —, y
lo consumen tanto el arnés de escala de `lodestar-app` como el banco de rendimiento:

```rust
use lodestar_fixtures::escala::{self, Perfil};

escala::genera(root, Perfil::Plano,    10_001, 0)?;  // el corpus homogéneo de E14-H05
escala::genera(root, Perfil::Realista,  1_000, 0xE330001)?;
```

- **`Perfil::Plano`** — el corpus de E14-H05 byte a byte: documentos idénticos en forma, **sin
  enlaces** ni diagnósticos. Es el suelo limpio contra el que se miden latencias, y por eso la
  semilla **no** influye en él (variarlo rompería la comparabilidad con las cifras históricas).
  Su `index.md` cuenta entre los documentos que escribe: `tamano = 10_001` produce los 10 000
  `c/documento-*.md` de E14-H05.
- **`Perfil::Realista`** — grafo con backlinks reales, huérfanos, frontmatter heterogéneo, tamaños
  de cuerpo desiguales y fauna de enlaces. Aquí la semilla **sí** es un parámetro real del corpus.

Escalas de `§22.2`: ~100 / ~1k / ~10k, pasadas como `tamano`.

Determinismo verificado por
`crates/lodestar-fixtures/tests/escala.rs::generador_de_escala_es_determinista_con_la_misma_semilla`
(huella `path → blake3(contenido)` de dos corridas), y las propiedades cualitativas del perfil
realista por `crates/lodestar-app/tests/corpus_realista.rs`.

Además, `crates/lodestar-fixtures/tests/ancla_plano.rs` guarda un **hash dorado** del corpus plano
pequeño. Es la única red que detecta una deriva de sus bytes frente a E14-H05: el arnés de escala
reconstruye los cuerpos con la misma función que los escribe, así que por sí solo seguiría en verde
aunque el corpus cambiara —y las cifras históricas dejarían de ser comparables en silencio—. Si ese
test falla, la pregunta es por qué cambiaron los bytes, no cómo actualizar el hash.

---

## Campaña exploratoria heredada (`decisiones §23`)

Los lotes históricos de `batches/` (`L*` —incluido explícitamente `L12_rob.json`—, `G*`, `H*`, `verify_*`) y las matrices `matriz_r*.json`
son la campaña de agosto de 2026 contra el homelab: 189 casos con esperado en prosa y verificación
adversarial, resumidos en [`../informe-homelab-2026-08-06.md`](../informe-homelab-2026-08-06.md) y
disueltos en `decisiones §23`. Se conservan como campaña repetible **fuera del gate** (`§22.3`).

E33-H02 les quitó las rutas de máquina **sin tocar un solo caso**: los comandos que llamaban a un
binario o a un root por su ruta absoluta ahora usan los tokens `@bin.mcp`/`@bin.cli`/`@root`/
`@repo`/`@testbench`, y las citas de fuentes en las matrices son relativas al repo. Su contenido
—argumentos, pasos y el campo `esperado` en prosa— es el mismo. Como no llevan `expect`, el runner
los trata como **exploratorios**: se ejecutan, se registran y no computan al veredicto.

```bash
python3 lodestar_harness.py --root <DIR> --profile readonly --list-tools
python3 lodestar_harness.py --batch batches/L1_typ.json --root <DIR> --out /tmp/l1.json
```

Ojo al leerlos: sus `esperado` describen el motor de **agosto de 2026**, y varios hallazgos se han
saldado desde entonces (E28–E31). Cuando un lote histórico y un lote `gate_*` parezcan contradecirse,
**manda el `gate_*`**: es el que se ejecuta y el que está aseverado.

**Regla dura**: contra un root **real** (no desechable) solo se corre `--profile readonly` y no se
permiten pasos `shell`/`spawn`; el runner impone ambas condiciones en el preflight, no son una
convención.

Los históricos `G0_real.json` y `H5_cursor_cli.json` contienen pasos `shell`, y `L12_rob.json`
también forma parte de los lotes migrados: los tres declaran `"root": "worktree"`. El runner crea
un worktree git efímero por caso y permite conservar la campaña de mutaciones/procesos sin ejecutar
shell sobre el root real. Los casos y sus esperados no cambian; un lote que necesite apuntar
directamente a un root real queda limitado a llamadas MCP en readonly.
