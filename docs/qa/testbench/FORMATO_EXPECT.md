# Formato `expect` del banco de conformidad (contrato del runner, E33-H02)

> Este documento **fija el contrato** que el runner asertable de `lodestar_harness.py` debe cumplir.
> Se escribió en la fase roja de E33-H02 (antes de la implementación) junto con el lote de autotest
> [`batches/meta_runner.json`](batches/meta_runner.json) y el selftest ejecutable
> [`selftest_runner.py`](selftest_runner.py). Autoridad de diseño: `ARCHITECTURE.md §22.1`/`§22.3`.
>
> Regla de lectura: donde este documento y el README general discrepen sobre la **semántica de una
> clave de `expect`**, manda este documento; el README manda sobre *cómo se corre* el banco.

---

## 1. Qué añade `expect` y qué no cambia

El formato de lote heredado de `decisiones §23` (`batch`, `root`, `profile`, `fixtures`, `cases[]`
con forma corta `tool`/`arguments` o forma larga `steps[]`, placeholders `@stepN.ruta`, `fresh_root`,
`no_server`) **no cambia**. `expect` es **aditivo**:

- un caso **sin** `expect` en ninguno de sus pasos ni a nivel de caso es **exploratorio**: se ejecuta
  igual que hoy, se registra su resultado, y **no computa** al veredicto (ni PASS ni FAIL);
- un caso **con** al menos un `expect` es **asertable**: produce veredicto PASS o FAIL.

El campo libre `esperado` (texto en prosa, presente en los lotes históricos) se conserva como
documentación humana y **nunca** se evalúa. `expect` es lo único mecánico.

## 2. Dónde se declara

```jsonc
{
  "batch": "meta_runner",
  "cases": [
    {
      "id": "CASO-01",
      "gate": true,                  // §5: pertenencia al gate
      "steps": [
        { "kind": "call", "tool": "workspace_status", "arguments": {},
          "expect": { /* aserciones sobre ESTE paso (§3) */ } }
      ],
      "expect": [ /* invariantes ENTRE pasos (§4) */ ]
    }
  ]
}
```

- **`step.expect`** — objeto. Aserciones sobre el resultado de ese paso.
- **`case.expect`** — lista de objetos. Invariantes que relacionan **dos o más pasos**.

En la **forma corta** (`{"id":…, "tool":…, "arguments":…}`, sin `steps`) el `expect` del caso
declarado como **objeto** (no lista) se interpreta como el `expect` del paso único. Si es lista, son
invariantes entre pasos — que con un solo paso solo puede comparar el paso 0 consigo mismo.

## 3. `step.expect` — aserciones sobre un paso

Todas las claves son **opcionales** y **conjuntivas**: el paso pasa si **todas** las declaradas se
cumplen. Un `expect: {}` vacío no asevera nada y **no** convierte el caso en asertable.

| Clave | Tipo | Semántica |
|---|---|---|
| `is_error` | bool | El resultado tiene `is_error` con exactamente ese valor. `false` exige además que **no** haya `protocol_error` ni `harness_exception`. |
| `error_code` | string | El resultado es un error de **tool** cuyo `error_code` (el prefijo `CODIGO:` del texto) es exactamente esa cadena. Implica `is_error: true`. |
| `protocol_error_code` | int | El resultado es un error de **protocolo** JSON-RPC con ese `code` (p. ej. `-32602`). Es la distinción que el arnés ya hace y que el banco necesita aseverar (readonly *rechaza* la tool, no la ejecuta). |
| `equals` | objeto `{path: valor}` | Para cada entrada: el valor en `path` (§3.1) **existe** y es **igual** al valor declarado (igualdad JSON estructural: mismo tipo, y en objetos/listas mismos elementos en el mismo orden). |
| `present` | lista de paths | Cada path **resuelve** (la clave existe, aunque su valor sea `null`). |
| `absent` | lista de paths | Cada path **no resuelve** *o* resuelve a `null`. Esa disyunción es deliberada: en el wire, «campo ausente» y «campo `null`» son la misma afirmación para el banco (p. ej. `nextCursor`). |
| `matches` | objeto `{path: regex}` | El valor en `path` existe, es una **cadena**, y la regex (dialecto `re` de Python, semántica `re.search`) casa contra ella. Para anclar, escríbelo con `^…$`. |
| `contains` | objeto `{path: valor}` | El valor en `path` es una **lista** que contiene el valor declarado (igualdad JSON estructural), **o** una **cadena** que contiene esa subcadena. |
| `not_contains` | objeto `{path: valor}` | Negación exacta de `contains`: el valor en `path` existe (lista o cadena) y **no** contiene lo declarado. Si el path no resuelve, **falla** (no se puede afirmar sobre lo que no existe). |
| `length` | objeto `{path: n}` | El valor en `path` es lista, objeto o cadena y su longitud es exactamente `n`. |
| `min_length` | objeto `{path: n}` | Igual, con longitud **≥ `n`**. Es la clave que sostiene los esperados independientes del filesystem (§6). |
| `type` | objeto `{path: tipo}` | El valor en `path` es del tipo JSON declarado: `"object"`, `"array"`, `"string"`, `"number"`, `"boolean"`, `"null"`. |
| `rc` | int | Solo pasos `shell`/`spawn`: el código de salida del proceso es exactamente ese. |
| `describe` | string | Prosa libre: qué asevera este paso y **por qué** (fuente/hallazgo). No se evalúa; el runner la **imprime** en el detalle de un FAIL. |

En un paso `kind: "raw"`, el arnés publica la respuesta bajo `response`. Si el servidor ignora la
línea ya enviada porque no existe un `id` correlacionable, `response` vale `null`; el silencio se
exige con `equals: {"response": null}`. No debe inventarse un error JSON-RPC para una notificación
o una línea que el transporte descarta. El EOF de stdout o la salida del proceso son estados
terminales, nunca silencio: una vez observados, el arnés no vuelve a escribir operaciones
`raw`, RPC ni barreras de resincronización en esa sesión.

### 3.1 El selector de path

Se reutiliza **el mismo dialecto de los placeholders `@stepN.…`** que el arnés ya implementa
(`resolve_placeholders`), para que no haya dos lenguajes de navegación en el mismo fichero:

- segmentos separados por `.`, aplicados sobre el **resultado del paso** tal como lo devuelve el
  arnés (o sea: la raíz tiene `is_error`, `text`, `structured`, `error_code`, `protocol_error`, o
  `rc`/`stdout`/`stderr` en pasos `shell`/`spawn`, o `tools` en `list_tools`);
- un segmento **entero** indexa una lista (`structured.results.0.path`);
- **no** hay comodines, ni filtros, ni escapes: una clave de frontmatter con punto literal no es
  direccionable por este selector (misma limitación declarada del catálogo de `metadata_inspect`).
  Si hiciera falta, se asevera sobre `text` con `contains`/`matches`.

Un path que **no resuelve** (clave inexistente, índice fuera de rango, o descenso sobre un escalar)
no es una excepción del runner: es un **FAIL** con el motivo «el path no resuelve», salvo en `absent`
(donde es justo lo que se afirma).

## 4. `case.expect` — invariantes entre pasos

Lista de objetos; cada uno relaciona pasos por índice. El caso pasa si todos se cumplen.

| Clave | Tipo | Semántica |
|---|---|---|
| `invariant` | string | Nombre del invariante: `"same"` o `"differs"`. Requerido. |
| `steps` | lista de int | Los índices de paso implicados (mínimo 2). |
| `path` | string | Selector (§3.1) aplicado **a cada** paso de `steps`. |
| `describe` | string | Prosa libre, se imprime en el FAIL. No se evalúa. |

- **`same`** — el valor en `path` **existe en todos** los pasos de `steps` y es **igual** en todos
  (igualdad JSON estructural). Es la forma de aseverar «`workspaceRevision` idéntica entre el paso
  i y el j» (§22.3) y «el `planHash` de dos planes idénticos coincide».
- **`differs`** — el valor existe en todos y **al menos uno** difiere de los demás. Es la forma de
  aseverar «el cursor de la página 2 no es el de la página 1».

Si un path no resuelve en alguno de los pasos, el invariante **falla** nombrando el paso culpable.

## 5. Gate vs demostración: la clave `gate`

`ARCHITECTURE.md §22.3` exige veredicto mecánico por release; BDD-2 exige además un caso que
**falla a propósito** para demostrar que el runner muerde. Los dos conviven con una sola clave:

- **`"gate": true`** (valor **por defecto** de todo caso asertable): el caso computa al veredicto y
  su FAIL hace salir al runner con exit ≠ 0.
- **`"gate": false`**: caso de **demostración**. **No se ejecuta** en la corrida normal: el runner
  solo lo corre cuando se le pide explícitamente con `--incluir-demos` (§7); sin esa bandera se
  omite (`SKIP`). Su veredicto se imprime y se cuenta aparte (`fuera_de_gate`).
- **Un FAIL es un FAIL, venga de donde venga**: si un caso se **ejecutó** y falló, el runner sale con
  exit `1`, tenga `gate: true` o `gate: false`. Es la letra de la historia («exit ≠ 0 si hay algún
  FAIL») y la única lectura que hace del selftest una prueba de que el runner muerde: si un FAIL de
  demostración se tragara el exit code, `--incluir-demos` no demostraría nada mecánicamente.

  Lo que `gate: false` protege es la **corrida por release**, y lo protege *no ejecutando* el caso,
  no perdonando su FAIL. Las dos reglas no se pisan porque no coinciden nunca en la misma corrida:
  sin `--incluir-demos` (el modo del gate y del CI) los `gate: false` ni se ejecutan, así que jamás
  pueden ensuciar su exit code; con `--incluir-demos` (el modo pedagógico del README) el exit `1`
  es justamente el resultado que se quiere enseñar.

  *Nota de procedencia*: la primera redacción de este §5 decía que un FAIL fuera de gate «no afecta
  al exit code», lo que era **incompatible** con la comprobación `BDD-2a` del selftest
  (`rc == 1` con `--incluir-demos`) y con el criterio de aceptación BDD-2. Manda la historia; el
  contrato queda corregido aquí. Si alguna vez hiciera falta una corrida que ejecute las demos
  **sin** que su FAIL tiña el exit code, sería una bandera nueva y explícita, no el silencio de esta.
- A nivel de **lote** puede declararse `"gate": false` en la raíz del spec: entonces ningún caso del
  lote entra al gate (es como declarar `gate: false` en todos ellos). Un `gate` de caso **gana** al
  del lote.

También es lícito `"expect_case": "FAIL"` a nivel de caso — «este caso debe fallar» — pero **no** se
usa en este contrato: un caso que debe fallar es una demostración, y su valor pedagógico está en ver
el FAIL impreso, no en convertirlo en un PASS invertido. El runner **no** tiene que soportarlo.

## 6. Regla de oro de los esperados: nada que dependa del filesystem

El corpus canónico contiene dos pares de nombres que **colisionan en APFS y no en ext4** (caja y
NFC/NFD, ver `README.md`), así que **el número de documentos del corpus depende del sistema de
ficheros** (89 documentos indexados en esta máquina APFS; más en ext4). En consecuencia, para el
banco:

- **prohibido** aseverar conteos absolutos derivados del inventario (`counts.documents`,
  `counts.isolated`, `counts.dangling`, `totalApproximate`, `summary.errors` de scope workspace…)
  con `equals`; se aseveran con `min_length`/`type`/`present`, o con `equals` **solo** cuando el
  conteo es de un scope **documento** concreto (p. ej. «este documento tiene exactamente 1 aviso»);
- **permitido y preferido**: códigos de error, formas de respuesta, `capabilities`, diagnósticos de
  un documento nombrado, invariantes entre pasos, y `equals` sobre campos derivados solo del
  contenido de un documento (su `title`, su `code` de diagnóstico).
- Los `revision`/`workspaceRevision`/`planHash` concretos **tampoco** se aseveran con `equals` a un
  literal (dependen del corpus completo): se aseveran con `matches` de forma (`^blake3:[0-9a-f]{64}$`)
  y con invariantes `same`/`differs` entre pasos.

## 7. Superficie de línea de comandos que el contrato exige

```bash
# Un lote asertable contra un root desechable ya generado
python3 lodestar_harness.py --batch batches/meta_runner.json --root-corpus DIR [--out FILE]

# Incluyendo los casos de demostración (gate: false)
python3 lodestar_harness.py --batch batches/meta_runner.json --root-corpus DIR --incluir-demos

# Todos los lotes del gate
python3 lodestar_harness.py --run-all --root-corpus DIR
```

- **`--root-corpus DIR`** — raíz **desechable** contra la que corren los lotes cuyo spec declara
  `"root": "corpus"`. El runner **copia** el corpus a un directorio efímero por caso `fresh_root`
  (o por lote) para que las mutaciones no lo contaminen; es el sustituto portable del worktree de
  git del homelab. Si el lote declara `"root": "corpus"` y no hay `--root-corpus`, el runner
  **genera** el corpus con `make_corpus.py` en un tempdir.
- **`--binary PATH`** / **`LODESTAR_MCP_BIN`** — el binario MCP; sin ninguno de los dos, el fallback
  es `target/release/lodestar-mcp` **relativo a la raíz del repo** (derivada de la ubicación del
  propio script), nunca una ruta absoluta hardcodeada.
- **Regla dura generalizada**: contra un root **declarado real** (`"root": "real"` o `--root`) solo
  se admite `--profile readonly`, y el preflight rechaza cualquier paso `shell`/`spawn` antes de
  abrir sesión o ejecutar pasos; contra `"root": "corpus"`/`"worktree"` (desechables) se admite
  cualquier perfil.

### Exit codes del runner

| Código | Significado |
|---|---|
| `0` | Ningún caso **ejecutado** falló. Puede haber casos exploratorios (no computan) y casos `gate: false` **omitidos** por no llevar `--incluir-demos`. |
| `1` | **Al menos un caso ejecutado es FAIL** — de gate, o de demostración cuando se corre con `--incluir-demos` (§5). |
| `2` | Uso incorrecto (flags incompatibles, lote inexistente, regla dura violada). |
| `3` | Error de ejecución del propio banco (no se pudo lanzar el binario, corpus ilegible…). |

En la corrida por release —`--run-all` sin `--incluir-demos`— las dos primeras filas se leen como
«`0` = 0 FAIL en el gate» y «`1` = hay FAIL en el gate», que es lo que exige `ARCHITECTURE.md §22.3`.

Son los mismos cuatro que la CLI del producto congela, por coherencia.

### Resumen que el contrato exige imprimir

En **stdout**, al final de la corrida:

1. una línea por caso con veredicto y su id: `PASS META-01`, `FAIL META-02`, `SKIP …`, `EXPLOR …`;
2. por cada **FAIL**, al menos una línea de detalle por aserción incumplida que nombre
   **el caso, el índice del paso, el path del subcampo que discrepó, el valor esperado y el real**
   (BDD-2: «el resumen lo lista con el subcampo que discrepó»), más el `describe` si lo hay;
3. una línea agregada final con los conteos, con el prefijo literal `RESUMEN:` y, como mínimo, los
   términos `gate`, `PASS` y `FAIL` (se admite el plural castellanizado `PASSes`/`FAILes`) seguidos
   de su recuento. Ejemplo de forma (el texto exacto lo fija el implementador):
   `RESUMEN: gate 12 casos · PASS 12 · FAIL 0 · exploratorios 3 · fuera de gate 1 (FAIL 1)`.

   El selftest solo exige que esa línea **exista** y declare el recuento de fallos; **no** ata la
   redacción. En particular, es legítimo que la palabra `FAIL` aparezca en el agregado aunque la
   corrida no tenga ningún fallo —lo dice para declarar que son cero—, y el selftest distingue esa
   línea de las líneas de **veredicto por caso** por su prefijo `RESUMEN:`.

Con `--out FILE` se escribe además el JSON de resultados; ese JSON gana, por caso, las claves
`verdict` (`"PASS"`/`"FAIL"`/`"SKIP"`/`"EXPLORATORY"`) y `failures` (lista de objetos con
`step`, `path`, `expected`, `actual`, `reason`). La salida heredada (`steps[]` con las respuestas
crudas) **se conserva íntegra**: el banco sigue sirviendo de evidencia, no solo de semáforo.
