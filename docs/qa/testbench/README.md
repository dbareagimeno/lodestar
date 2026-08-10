# Banco de pruebas (`ARCHITECTURE.md §22`)

> **Esqueleto (E33-H01).** Aquí está documentado lo que E33-H01 entrega: los generadores de corpus
> y cómo verificar su determinismo. El **runner asertable** (formato `expect`, veredicto mecánico
> PASS/FAIL, portabilidad del binario) es **E33-H02** y completará este documento; los umbrales de
> rendimiento y el enganche a release son E33-H05/H07. Lo que hoy hay de ejecución —
> `lodestar_harness.py` y los lotes de `batches/`— es la campaña exploratoria heredada de
> `decisiones §23`, con rutas todavía hardcodeadas.

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
| `lodestar_harness.py` | El ejecutor JSON-RPC/stdio: llamada suelta, lote (`--batch`) o `tools/list`. | — |
| `build_artifact.py` | Empaqueta los resultados de una campaña. | — |

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

Los lotes de `batches/` y las matrices `matriz_r*.json` son la campaña de agosto de 2026 contra el
homelab. Se conservan tal cual como campaña repetible **fuera del gate** (`§22.3`). Hoy siguen
apuntando a rutas absolutas de una máquina concreta (`BINARY`/`HOMELAB` en `lodestar_harness.py`);
E33-H02 las parametriza.

```bash
python3 lodestar_harness.py --root <DIR> --profile readonly --list-tools
python3 lodestar_harness.py --batch batches/L1_typ.json --out /tmp/l1.json
```

**Regla dura heredada**: contra un root **real** (no desechable) solo se corre `--profile readonly`.
