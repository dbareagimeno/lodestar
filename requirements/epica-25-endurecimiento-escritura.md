# E25 — Endurecimiento del camino de escritura: TOCTOU, durabilidad y propiedad

> **Fase**: posterior a v0.3.1 y al bloque C de E24 (v0.4.0). No es una fase de `§20.14` ni de
> `§19.8`: es la épica de **endurecimiento** del motor transaccional que abrió la auditoría del
> camino de escritura.
> **Objetivo de la épica**: que ninguna salvaguarda del camino de escritura —permisos, copias de
> recuperación, journal, control optimista, lock— se pueda esquivar porque el estado que la motivó
> ya no es el estado sobre el que se ejerce; y que lo que se escribe para poder deshacer sea tan
> durable como lo que se escribe para publicar.
> Referencias maestras: `ARCHITECTURE.md §19.4`/`§19.5` (modelo transaccional), `ARCHITECTURE.md
> §20.5`/`§20.13`, `docs/REFACTOR_PHASE_2.md §5.2`/`§11.2`/`§11.3`, `CLAUDE.md` (invariantes #1 y #5).

**Origen**: auditoría del camino de escritura (2026-07-29), posterior a la publicación de v0.3.1.
Los seis defectos se localizaron leyendo el orquestador y sus primitivas, y **cada historia declara
el escenario que hoy los reproduce**: la fase roja de cada una es ese escenario, no una aserción
sobre el código.

La v0.3.1 pasa todas sus puertas —suite en verde, `clippy -D warnings`, los tests de crash-recovery
tras `--features test-failpoints`, pureza del core— y el invariante nuclear (un crash nunca deja un
`.md` a medias) aguanta `SIGKILL` reales durante `change_apply`. **Nada de lo que sigue lo
contradice**: son defectos que la suite no mira porque todos necesitan **dos actores** —dos procesos,
o un proceso y una caída— y la suite ejerce uno.

**Principio rector**: *una salvaguarda vale por el estado sobre el que se ejerce, no por el estado
sobre el que se computó.* Los seis defectos son la misma forma: `writable`, `backup`, `journal`,
`revisión base` y `propiedad del lock` se calculan en un instante y se ejercen en otro, sin volver a
mirar en medio. Cuando la duda sea «¿esto sigue siendo cierto aquí?», la respuesta de esta épica es
**re-mirar bajo el lock y abortar si cambió**, nunca «seguramente sí».

**Fuera de alcance (explícito)**:

- **Convertir la publicación en bloqueante o reintentable.** El modelo es *fail-fast* (`§19.5`,
  E13-H02): un conflicto se reporta, no se espera ni se reintenta. Estas historias añaden puntos de
  detección, no una política de reintento.
- **Un `lodestar recover` como subcomando.** Sigue siendo el hueco residual declarado por E24-H04, y
  E25-H02 lo estrecha (un journal irrecuperable deja de encallar el workspace) sin cerrarlo.
- **Conectar el store** (`DECISIONES §14`). Sigue sin consumidor.
- **La publicación de la release y los documentos de estado** (`CHANGELOG.md`,
  `IMPLEMENTATION_STATUS.md`, `requirements/README.md`, `requirements/trazabilidad.md`). Esta épica y
  E26 comparten rama y nota de release; el cierre documental se planifica aparte, con las
  **consecuencias declaradas** que cada historia marca como material de nota de release.

---

## Bloque A — La ventana entre planificar y publicar

### E25-H01 — La publicación no escribe fuera de lo que respaldó

- **Objetivo**: que el conjunto de ficheros que la transacción sustituye sea **exactamente** el que
  pasó por `assert_writable`, por el backup y por el journal — o que la transacción aborte antes del
  primer rename.
- **Defecto (S1, crítico) y escenario que lo reproduce hoy**: `apply_transaction` computa el
  canónico, el resultado y el conjunto afectado en **T1**
  (`crates/lodestar-workspace/src/transaction.rs:127-129`), y sobre **ese** conjunto ejerce
  `assert_writable` (`:133-135`), el backup (`:156`) y el journal (`:161`). Pero `publish_result`
  **vuelve a leer el canónico** con `discover_files()` en **T3**
  (`crates/lodestar-workspace/src/publish.rs:104`) y **recomputa `affected`** contra el `result` de
  T1 (`publish.rs:114-124`), escribiendo con `io::write_atomic` o borrando con `io::delete`
  (`publish.rs:127-134`) todo lo que difiera — sin `assert_writable`, sin copia de recuperación y
  **sin entrada de journal**. Tres consecuencias, cada una con su escenario:
  1. **Edición externa pisada sin copia**: otro proceso (o el usuario) modifica un `.md` afectado
     dentro de la ventana `[T1, T3)`; la publicación lo sobrescribe con el contenido del plan y su
     backup contiene la versión de T1, así que `change_revert` restaura un estado que nunca existió.
  2. **Fichero nuevo borrado**: un `.md` **creado** dentro de la ventana no está en `result`, así que
     el bucle de `publish.rs:120-124` lo mete en `affected` y `publish.rs:130` lo **borra**. No hay
     backup (nunca estuvo en `affected` en T1) ni entrada de journal: el borrado es **irrecuperable**
     y el recibo ni lo menciona.
  3. **`referenceRoots` sobrescribibles**: un `.md` aparecido en la ventana bajo un `referenceRoot`
     sufre lo mismo, y el control optimista no lo salva porque
     `lodestar_core::types::workspace_revision` **excluye** lo que queda fuera de `writableRoots`
     (`crates/lodestar-core/src/types.rs:1247-1249`, fijado por
     `core.rs::revision_excluye_reference_roots`): `reverify_base_revision`
     (`crates/lodestar-workspace/src/lib.rs:203`) no puede verlo ni en principio.
- **Referencias**: `ARCHITECTURE.md §19.5` (orden exacto de la transacción, replicado en el rustdoc
  de módulo de `transaction.rs:9-31`) · `docs/REFACTOR_PHASE_2.md §5.2` paso 11 ·
  `crates/lodestar-workspace/src/publish.rs` (`publish:52`, `publish_result:85`) ·
  `crates/lodestar-workspace/src/transaction.rs` (`affected_paths:59`, `apply_transaction:102`) ·
  `crates/lodestar-workspace/src/external_refs.rs` (`assert_writable:60`) · `CLAUDE.md` invariantes
  #1 y #5.
- **Alcance**:
  - `publish_result` recibe el **canónico de T1** (el mismo `FileMap` con el que se computaron
    `result_files` y `affected`) y, tras releer el canónico en T3, **compara ambos**. Si difieren en
    cualquier path, devuelve `WorkspaceError::WriteConflict` **antes del primer rename**. El conjunto
    ya respaldado y anotado en el journal es la **única** escritura legítima.
  - El bucle de publicación deja de derivar `affected` de una diferencia recomputada: publica
    exactamente el conjunto que el journal declara. La equivalencia «lo materializado y validado en
    staging es lo que se publica» (rustdoc de `publish.rs:63-71`) pasa a valer también para el
    conjunto de paths, no solo para el contenido.
  - `Workspace::publish` (`publish.rs:52`, hoy con llamadores solo en
    `crates/lodestar-workspace/tests/transactions.rs:724` y `:802`) se ajusta a la firma nueva
    computando el canónico una sola vez.
  - **Seam de test nuevo**, bajo `#[cfg(feature = "test-failpoints")]`: hoy `failpoint!`
    (`crates/lodestar-workspace/src/lib.rs:38`) solo sabe **abortar**. Hace falta un punto que
    **ejecute un gancho del test y continúe**, armado por hilo igual que `failpoints::armar`
    (`crates/lodestar-workspace/src/failpoints.rs:75`), colocado dentro de la ventana `[T1, T3)` —
    es la única forma de inyectar una edición externa donde el defecto vive. La forma exacta la elige
    el implementador con dos restricciones: en compilación normal no genera ni una instrucción, y el
    gancho vive en el orquestador real, no en una reconstrucción del flujo (lección de E24-H13).
- **Fuera de alcance**: cambiar el modelo a *fail-fast → reintento*. Un `WRITE_CONFLICT` sigue siendo
  terminal para esa transacción; el agente replanifica.
- **Criterios de aceptación**:
  - **Dado** un apply en curso con el gancho armado en la ventana `[T1, T3)`, **Cuando** el gancho
    modifica un `.md` afectado, **Entonces** la transacción falla con `WRITE_CONFLICT` y **ni uno**
    de los `.md` canónicos ha cambiado → `edicion_externa_en_la_ventana_aborta_sin_publicar`.
  - **Dado** ese mismo apply, **Cuando** el gancho **crea** un `.md` nuevo que el plan no menciona,
    **Entonces** ese fichero **sigue existiendo** con su contenido intacto tras el fallo →
    `fichero_nuevo_en_la_ventana_no_se_borra` (hoy desaparece sin copia ni journal).
  - **Dado** un workspace con `referenceRoots`, **Cuando** el gancho crea un `.md` bajo un
    `referenceRoot` dentro de la ventana, **Entonces** ese fichero no se toca →
    `reference_root_no_se_borra_en_la_ventana`.
  - **Dado** un apply **sin** interferencia, **Cuando** se aplica, **Entonces** publica exactamente
    los mismos paths y contenidos que en v0.3.1, con el mismo `resultWorkspaceRevision` y el mismo
    `changedPaths` → `apply_sin_interferencia_publica_igual` (control anti-vacuo: el arreglo no puede
    consistir en abortar más a menudo).
  - **Dado** el conjunto publicado y el conjunto respaldado, **Cuando** termina cualquier apply con
    éxito, **Entonces** son idénticos → `publicado_igual_a_respaldado` (propiedad, sobre los mismos
    change sets del arnés de transacciones).
  - **No regresión**: `cargo test --workspace` y
    `cargo test -p lodestar-workspace --features test-failpoints` en verde;
    `recovery_sin_parciales_por_el_orquestador_real` y `caida_entre_backup_y_journal`
    (`transactions.rs:1836`/`:1895`) siguen pasando **sin tocarse**.
- **Dependencias**: ninguna.
- **Pruebas**: `crates/lodestar-workspace/tests/transactions.rs` (gancho + los cuatro escenarios) ·
  `crates/lodestar-app/tests/escritura.rs` (que el `WRITE_CONFLICT` llega a la fachada con su código).
- **Frontera (mcp.yml)**: no (cambia cuándo se emite `WRITE_CONFLICT`, no la forma del wire ni el
  catálogo).

---

## Bloque B — Durabilidad y propiedad del plano de recuperación

### E25-H02 — Las copias de recuperación son durables, verificadas y nunca encallan el workspace

- **Objetivo**: que una copia de recuperación sea tan fiable como el `.md` que protege, y que un
  journal que no se puede restaurar deje de cerrar el workspace a la escritura para siempre.
- **Defecto (S2, crítico) y escenarios que lo reproducen hoy**:
  1. **Copias no durables**: `backup_originals` copia con `std::fs::copy`
     (`crates/lodestar-workspace/src/recovery.rs:140`) — sin `sync_all` y sin fsync del directorio— y
     escribe el manifiesto `.absent` con `std::fs::write` (`recovery.rs:155-158`), también sin fsync.
     El journal, en cambio, **sí** se fsynca antes del primer rename (`§19.5` paso 9). Tras un corte
     de energía puede quedar un journal **durable** que apunta a una copia **truncada o ausente**:
     exactamente la combinación que la recuperación da por buena.
  2. **Restauración verbatim de una copia rota**: `restore_backups` lee la copia y la escribe tal
     cual sobre el canónico (`recovery.rs:358-364`), sin verificar nada. Una copia truncada se
     **publica** como si fuera el original.
  3. **Workspace encallado para siempre**: si la copia es ilegible, `restore_backups` devuelve `Err`
     y `recover()` lo propaga con `?` en sus **tres** brazos (`recovery.rs:271`, `:275-276`,
     `:297-298`). No hay cuarentena, ni descarte, ni `--force`: `pending_journals`
     (`recovery.rs:206`) sigue viendo ese journal, `recovery_pending()` sigue en `true`
     (`recovery.rs:237`) y **toda** escritura futura muere en el paso (2) de `apply_transaction`
     (`transaction.rs:114-116`). Un solo fichero ilegible cierra el workspace.
  4. **`.absent` perdido → estado híbrido**: `read_absent_manifest` trata cualquier fallo de lectura
     como conjunto vacío (`recovery.rs:179-188`), así que si el manifiesto no llegó a disco la
     restauración **no borra** los ficheros que la transacción creó: el canónico queda con los
     originales restaurados **más** los ficheros nuevos. Ni un borde ni el otro — justo lo que el
     rustdoc de `recover` promete que no puede pasar (`recovery.rs:253-258`).
  5. **La recuperación deshace lo que el aborto de ventana acababa de proteger** *(estado nuevo,
     señalado por la implementación de **E25-H01**)*: cuando la publicación aborta con
     `WRITE_CONFLICT` por divergencia de la ventana `[T1, T3)`, quedan en disco el journal
     `prepared` (creado en `transaction.rs:161`, antes de la comprobación) y su árbol de recuperación
     con las copias de **T1** — y **cero renames aplicados**. La siguiente operación ve
     `recovery_pending()` en `true` (`recovery.rs:237`), `recover()` clasifica ese journal como
     `prepared` → **RESTAURAR** (`recovery.rs:274-277`) y `restore_from_recovery` escribe las copias
     de T1 **encima de la edición externa** que el aborto existía para no pisar. El defecto es
     literalmente el de H01 con un rodeo: en vez de sobrescribir la edición al publicar, la
     sobrescribe al recuperar, una operación más tarde y sin que nadie lo pida.
     Y con el manifiesto `.absent` es peor: un path que no existía en T1 y que el **usuario** creó
     dentro de la ventana está marcado «no existía», así que la restauración lo **borra**
     (`recovery.rs:331-333`) — deshaciendo también la garantía de
     `fichero_nuevo_en_la_ventana_no_se_borra`. **Sin esta enmienda, las tres garantías de E25-H01
     duran exactamente hasta la siguiente operación.**
- **Referencias**: `ARCHITECTURE.md §19.5` (copias de recuperación y recuperación determinista) ·
  `docs/REFACTOR_PHASE_2.md §5.2` · `crates/lodestar-workspace/src/recovery.rs`
  (`backup_originals:113`, `read_absent_manifest:179`, `recover:262`, `restore_from_recovery:316`,
  `restore_backups:340`, `finish_recovery:376`) · `crates/lodestar-workspace/src/io.rs`
  (`write_atomic:18`, el patrón durable que ya existe) · `contracts/mcp.yml:651-666`
  (`codigos_sin_emisor`, fila `RECOVERY_FAILED`).
- **Alcance**:
  - **Durabilidad**: cada copia se escribe con el mismo protocolo durable que ya usa el único
    escritor (`io.rs:18-43`: contenido + `sync_all` antes de dar la copia por hecha) y el manifiesto
    `.absent` también. El directorio de recuperación se fsynca **una vez**, al terminar
    `backup_originals`, antes de que la transacción avance al journal — el orden `copias durables →
    journal durable → renames` es lo que hace verdad la premisa de la recuperación.
  - **Verificación al restaurar**: `backup_originals` registra el tamaño y el hash `blake3` de cada
    original respaldado (blake3 ya está en el árbol: lo usan `DocumentRevision` y
    `workspace_revision`), y `restore_backups` **verifica antes de escribir**. Una copia que no casa
    no se restaura: es un fallo de recuperación, no un original.
  - **Cuarentena en vez de encalle**: cuando la restauración de un journal falla —copia ausente,
    ilegible o que no verifica—, ese journal y su árbol de recuperación se **mueven** a
    `.lodestar/runtime/journal/quarantine/<txnId>/` (nada se borra: es material forense), `recover()`
    **sigue** con los demás journals pendientes, y la operación que disparó la recuperación falla
    **una vez** con `RECOVERY_FAILED` y un mensaje que nombra la ruta de cuarentena. La siguiente
    operación ya no ve un journal pendiente y procede.
  - `RECOVERY_FAILED` gana así su **primer emisor real** y sale de `codigos_sin_emisor`
    (`contracts/mcp.yml:665`). **No** se añade ningún `ErrorCode`: el catálogo sigue teniendo 16 filas.
  - **El aborto de ventana sella su propio journal** (defecto 5): cuando la comprobación de E25-H01
    detecta divergencia, el camino de aborto —que sabe por control de flujo que **no ha entrado en el
    bucle de renames**— sella su transacción **bajo el mismo lock, antes de devolver el
    `WRITE_CONFLICT`**: borra el fichero de journal **primero** (es lo que levanta el gate de
    `recovery_pending`) y luego su árbol de recuperación, reusando la limpieza de
    `finish_recovery` (`recovery.rs:376`) en vez de escribir una segunda copia del sellado. No hay
    nada que restaurar: cero renames significa que el canónico nunca se movió, así que sellar es
    exacto, no una amnistía. Si el proceso muere entre los dos borrados queda un árbol de
    recuperación **sin journal**, que es un huérfano legítimo y lo recoge el GC (E24-H06, con el
    criterio de propiedad de E25-H03) — por eso el journal va primero.
    - **Alternativa admisible con el mismo efecto observable**: cualquier variante en la que, tras un
      `WRITE_CONFLICT` de ventana, la siguiente operación no encuentre recuperación pendiente de esa
      transacción y la edición externa siga intacta. Lo que **no** es admisible es la generalización
      tentadora —*«`recover()` no restaura un journal `prepared` con cero entradas `applied`»*—:
      `mark_applied` re-persiste el journal **después** de cada rename (`publish.rs:129-132`), así
      que «cero `applied` durables» también describe el estado de una caída **entre** el primer
      rename y su anotación. Sellar por esa inferencia daría por buena una publicación parcial, que
      es justo lo que la recuperación existe para impedir. La decisión tiene que tomarla el camino
      que **sabe** que no publicó, no una lectura del journal a posteriori.
    - Tampoco vale mover la comprobación de ventana **antes** del journal: su valor está en ser lo
      más tardía posible, inmediatamente antes del primer rename. Adelantarla reabre la ventana entre
      la comprobación y el rename, que es el defecto de E25-H01 otra vez.
- **Consecuencia declarada (material de nota de release)**: la promesa «el canónico converge a uno de
  los dos bordes» pasa a ser **incondicional solo mientras las copias verifiquen**. Con copias
  corruptas, lo que se garantiza es: (a) nada se escribe a partir de una copia que no verifica; (b)
  el material se preserva en cuarentena; (c) el fallo se reporta con código propio; (d) el workspace
  vuelve a ser escribible. Es estrictamente mejor que hoy —hoy (b) no existe, (a) se incumple y (d)
  es imposible—, pero es una promesa distinta y hay que escribirla como tal.
- **Riesgo residual declarado (no se cierra aquí)**: el contenido en cuarentena no se surface por el
  wire (`workspace_status` no gana un campo), así que quien no lea el mensaje de error o el stderr no
  sabrá que hay material forense esperando. Cerrarlo pide un `lodestar recover`, que sigue fuera de
  alcance por la decisión de E24.
- **Ajuste declarado sobre un test de E25-H01**: `edicion_externa_en_la_ventana_aborta_sin_publicar`
  comprueba hoy, como precondición del escenario, que **tras el fallo existe** el árbol de
  recuperación de la transacción abortada. Con el sellado del aborto esa aserción queda
  **incompatible**: después del `WRITE_CONFLICT` de ventana no hay ni journal ni árbol. El ciclo de
  H02 debe **invertirla** —el test pasa a exigir que no queden ninguno de los dos— y no relajarla ni
  borrarla: es exactamente el estado que esta historia hace imposible, así que su aserción tiene que
  seguir mordiendo, con el signo contrario. Se declara aquí para que el ajuste se haga **con
  conocimiento de causa** y no como un test que se toca hasta que pasa.
- **Criterios de aceptación**:
  - **Dado** un journal `prepared` cuya copia de recuperación está **truncada**, **Cuando** se
    reabre el workspace y se recupera, **Entonces** el canónico **no** se sobrescribe con la copia
    rota → `copia_truncada_no_se_restaura_verbatim` (hoy la escribe encima).
  - **Dado** un journal `prepared` cuya copia es **ilegible**, **Cuando** se recupera y luego se
    intenta una transacción nueva, **Entonces** la primera falla con `RECOVERY_FAILED` nombrando la
    cuarentena y **la segunda tiene éxito** → `journal_irrecuperable_no_encalla_el_workspace` (hoy
    ninguna de las dos tiene éxito, nunca).
  - **Dado** dos journals pendientes, uno sano y uno irrecuperable, **Cuando** se recupera,
    **Entonces** el sano se recupera igualmente → `un_journal_roto_no_arrastra_a_los_demas`.
  - **Dado** una transacción que **crea** ficheros y cuyo manifiesto `.absent` se pierde antes de la
    caída, **Cuando** se recupera, **Entonces** el canónico converge al borde «original» —los
    ficheros creados no sobreviven— o la recuperación va a cuarentena; **nunca** al híbrido →
    `absent_perdido_no_deja_estado_hibrido`.
  - **Dado** el material en cuarentena, **Cuando** se inspecciona, **Entonces** el journal y el árbol
    de recuperación siguen ahí completos → `la_cuarentena_no_borra_nada`.
  - **Dado** un `WRITE_CONFLICT` de ventana (el escenario de E25-H01, con la edición externa ya en
    disco), **Cuando** la siguiente operación abre el workspace y recupera, **Entonces** la edición
    externa sigue **intacta** byte a byte →
    `el_aborto_de_ventana_no_deja_recuperacion_que_pise_la_edicion` (hoy la recuperación la
    sobrescribe con la copia de T1).
  - **Dado** ese mismo `WRITE_CONFLICT` de ventana con un `.md` **creado por el usuario** dentro de
    la ventana, **Cuando** la siguiente operación recupera, **Entonces** ese fichero **sigue
    existiendo** → `el_aborto_de_ventana_no_borra_el_fichero_nuevo_al_recuperar` (el manifiesto
    `.absent` lo marca «no existía», así que hoy la restauración lo borra: es la garantía de
    `fichero_nuevo_en_la_ventana_no_se_borra` deshecha una operación más tarde).
  - **Dado** ese mismo aborto, **Cuando** termina, **Entonces** `recovery_pending()` es `false` y no
    queda ni journal ni árbol de recuperación de esa transacción →
    `el_aborto_de_ventana_no_deja_recuperacion_pendiente`.
  - **Dado** un aborto de ventana interrumpido **entre** el borrado del journal y el del árbol,
    **Cuando** se reabre y corre el GC, **Entonces** no hay recuperación pendiente y el huérfano se
    purga → `el_sellado_del_aborto_es_seguro_a_mitad`.
  - **Dado** una caída **entre el primer rename y su anotación en el journal** (journal `prepared`
    con cero entradas `applied` pero con un rename ya hecho), **Cuando** se recupera, **Entonces**
    **sí** se restaura → `cero_applied_no_significa_cero_renames` (control anti-vacuo: el sellado del
    aborto no puede degenerar en «un journal `prepared` sin `applied` no hay que restaurarlo», que
    sellaría publicaciones parciales).
  - **Dado** una recuperación normal (copias sanas), **Cuando** se recupera, **Entonces** el
    comportamiento es idéntico a v0.3.1 → `recovery_sin_parciales_por_el_orquestador_real` y
    `caida_entre_backup_y_journal` siguen verdes sin tocarse (control anti-vacuo).
  - **Estructural (checklist binario)**: ninguna escritura de `recovery/` usa `std::fs::copy` ni
    `std::fs::write` sin volcado — revisión de diff sobre `recovery.rs` + `grep` de que las copias
    pasan por el helper durable.
- **Dependencias**: **E25-H01** (comparten el contrato «lo respaldado es lo publicable» y el mismo
  fichero de tests de recuperación).
- **Pruebas**: `crates/lodestar-workspace/tests/transactions.rs` (los escenarios de durabilidad se
  construyen corrompiendo el árbol de `recovery/` antes de reabrir, como ya hace `mod recuperacion`;
  los cuatro del aborto de ventana reusan el **gancho de E25-H01** y siguen con una reapertura +
  `recover()`, que es donde se manifiesta el defecto) · `crates/lodestar-app/tests/error.rs` (el
  código `RECOVERY_FAILED` en la fachada).
- **Frontera (mcp.yml)**: **sí** — `RECOVERY_FAILED` gana emisor y `codigos_sin_emisor` pierde una
  fila.
- **Delta de contrato**:
  ```yaml
  # contracts/mcp.yml — cabecera (bloque E25) + codigos_sin_emisor
  codigos_sin_emisor:
    nota: >-
      De las 16 filas congeladas de `ErrorCode` (§19.3), estas CUATRO no se emiten desde ningún
      camino del producto.   # eran CINCO; E25-H02 dio emisor a RECOVERY_FAILED
    filas:
      # - { codigo: RECOVERY_FAILED, … }   ← SE RETIRA
      - { codigo: WORKSPACE_NOT_FOUND, motivo: "…" }
      - { codigo: RESULT_TOO_LARGE, motivo: "…" }
      - { codigo: RELATION_CONSTRAINT_VIOLATION, motivo: "…" }
      - { codigo: AMBIGUOUS_REFERENCE, motivo: "…" }

  tools:
    - nombre: change_apply
      errores: [..., "RECOVERY_FAILED (una transacción interrumpida no se pudo restaurar: su journal
                y sus copias quedan en .lodestar/runtime/journal/quarantine/<txnId>/ y el mensaje
                nombra la ruta; la SIGUIENTE llamada ya no encuentra recuperación pendiente)"]
    - nombre: change_revert
      errores: [..., "RECOVERY_FAILED (mismo caso: la reversión también recupera antes de revertir)"]
  ```

### E25-H03 — El GC no destruye el plano de recuperación de una transacción viva

- **Objetivo**: que el recolector de basura del plano de control no pueda dejar sin copias a una
  transacción que está publicando en **otro proceso**.
- **Defecto (S3, crítico) y escenario que lo reproduce hoy**: `gc_receipts` se invoca **después** de
  que `apply_transaction`/`revert_transaction` hayan soltado el lock —`crates/lodestar-app/src/lib.rs:1697`
  (apply) y `:1826` (revert), ambos fuera del `_lock` que vive dentro de la transacción—, y
  `gc_runtime_huerfanos` (`crates/lodestar-workspace/src/receipts.rs:314`) purga **todo** directorio
  de `staging/` y `recovery/` cuyo nombre no aparezca ni en `journal/` ni en `receipts/`
  (`receipts.rs:319-351`). Ese criterio es correcto para un solo proceso y **falso** con dos: entre
  `backup_originals` (`transaction.rs:156`) y `create_journal` (`transaction.rs:161`) hay una ventana
  —la que el propio `FailPoint::TrasBackupSinJournal` (`transaction.rs:158`) modela— en la que la
  transacción **tiene copias y no tiene journal ni recibo**. Un `change_apply` del proceso B que
  termine en ese instante lanza su GC y **borra el árbol de recuperación del proceso A**. A publica
  entonces sin copias; si cae, `restore_from_recovery` no encuentra directorio y devuelve `Ok(())`
  de inmediato (`recovery.rs:323-325`), con lo que la recuperación **sella un estado parcial en
  silencio**. El test `caida_entre_backup_y_journal` (`transactions.rs:1918-1929`) afirma hoy que ese
  árbol **debe** desaparecer con el GC: es correcto para un dueño muerto y es exactamente lo que hace
  peligroso el mismo criterio aplicado a un dueño vivo.
- **Referencias**: `ARCHITECTURE.md §19.5` (lock de publicación y retención) ·
  `crates/lodestar-workspace/src/receipts.rs` (`gc_receipts:224`, `gc_runtime_huerfanos:314`) ·
  `crates/lodestar-workspace/src/transaction.rs:154-161` · `crates/lodestar-workspace/src/lock.rs`
  (`acquire_lock:73`, `reclamar_si_huerfano:162`) · `crates/lodestar-app/src/lib.rs:1697`/`:1826`.
- **Alcance**:
  - Fijar la **invariante**: el GC nunca purga material de una transacción **en curso**, sea de este
    proceso o de otro. Dos mecanismos son admisibles y el implementador elige uno, con la condición
    de que el criterio quede en **un solo sitio** (invariante #3):
    1. **GC bajo el lock**: `gc_receipts` adquiere el lock de publicación (fail-fast: si no lo
       consigue, **no barre** y devuelve `Ok(())` — el GC es best-effort por definición), y la
       llamada desde dentro de la transacción usa una variante interna que asume el lock tomado.
    2. **Marca durable de transacción en curso** creada **antes** de `backup_originals` y respetada
       por `gc_runtime_huerfanos` como tercer conjunto de «vivos», junto a `journal/` y `receipts/`.
       La marca lleva dueño (pid, host) y timestamp, y se considera **rancia** con el mismo criterio
       de propiedad que el lock (`reclamar_si_huerfano`, que **E25-H06** endurece) — si no, un crash
       en esa ventana volvería a dejar basura inmortal y se cambiaría un defecto por otro.
  - Actualizar `caida_entre_backup_y_journal` (`transactions.rs:1895`): su aserción «el GC recoge el
    huérfano» sigue valiendo, pero ahora **porque el dueño está muerto**, no porque no haya journal.
    Es un test que hoy pasa por la razón equivocada y hay que dejarlo pasando por la correcta.
  - El GC sigue sin poder tumbar la operación que lo invocó: un fallo de barrido se degrada, nunca
    convierte un apply publicado en `Err` (esto lo cierra del todo **E25-H04**).
- **Fuera de alcance**: cambiar la política de retención (`transactions.retainReceiptsFor`,
  `maximumReceipts`). Cambia **a quién** mira el GC, no cuánto guarda.
- **Criterios de aceptación**:
  - **Dado** un proceso A detenido en la ventana `[backup, journal)` (failpoint
    `TrasBackupSinJournal` armado en un hilo, o un `Workspace` de prueba que reproduce el estado con
    su marca), **Cuando** un proceso B ejecuta el GC, **Entonces** el árbol de recuperación de A
    **sigue intacto** → `gc_no_destruye_una_transaccion_en_curso_de_otro_proceso` (hoy lo borra).
  - **Dado** ese mismo estado pero con el dueño **muerto** (proceso inexistente / marca rancia),
    **Cuando** corre el GC, **Entonces** el árbol se purga →
    `gc_sigue_purgando_huerfanos_de_dueno_muerto` (control anti-vacuo: el arreglo no puede consistir
    en dejar de barrer).
  - **Dado** una transacción que publica con éxito, **Cuando** termina, **Entonces** no queda ninguna
    marca de «en curso» en `.lodestar/runtime/` → `la_marca_no_sobrevive_a_la_transaccion`.
  - **Dado** un GC que no consigue barrer (lock tomado, marca ilegible), **Cuando** se invoca,
    **Entonces** devuelve `Ok(())` y la operación que lo llamó no se ve afectada →
    `el_gc_nunca_tumba_a_quien_lo_llama`.
  - **No regresión**: `gc_purga_huerfanos_sin_recibo`, `gc_purga_temporales_huerfanos`,
    `gc_no_toca_transacciones_vivas` y `sin_manifiesto_absent_vacio` (E24-H06) siguen verdes.
- **Dependencias**: **E25-H02**.
- **Pruebas**: `crates/lodestar-workspace/tests/transactions.rs` (dos handles `Workspace` sobre la
  misma raíz; el segundo hace de «otro proceso») · `crates/lodestar-mcp/tests/concurrencia.rs` si el
  escenario se quiere además con dos procesos reales.
- **Frontera (mcp.yml)**: no.

### E25-H04 — Publicar implica recibo: nada se pierde después del punto de no retorno

- **Objetivo**: que si el canónico cambió, **siempre** exista el recibo que permite deshacerlo — y
  que `change_apply` no devuelva `Err` cuando en realidad publicó.
- **Defecto (S5) y escenario que lo reproduce hoy**: tras `publish_result`
  (`crates/lodestar-workspace/src/transaction.rs:167`) el disco **ya está cambiado**, pero quedan
  pasos que salen por `?`: el sellado (`transaction.rs:180-185`), y en la fachada
  `write_receipt` (`crates/lodestar-app/src/lib.rs:1694-1696`) y `gc_receipts` (`:1697-1699`).
  Cualquiera de ellos convierte una transacción **publicada** en un `Err` **sin recibo**. El agente
  concluye que no se aplicó nada; el workspace dice lo contrario. Y no hay salida: `change_revert`
  carga el recibo primero y, al no encontrarlo, responde `PLAN_EXPIRED`
  (`crates/lodestar-app/src/lib.rs:1777-1780`) **para siempre**; un segundo `change_apply` del mismo
  plan muere con `PLAN_STALE` (`:1668-1671`) porque la base cambió. El mismo agujero existe con un
  crash real: los failpoints `TrasPublicarSinSellar` (`transaction.rs:169`) y `AntesDeSellar`
  (`:179`) dejan exactamente ese estado —canónico publicado, recibo inexistente— y **ningún** test
  llega a mirar qué le pasa a `change_revert` después, porque los dos se ejercen desde
  `lodestar-workspace`, por debajo de la capa que escribe los recibos.
- **Referencias**: `ARCHITECTURE.md §19.5` (paso 11 y recibo) · `docs/REFACTOR_PHASE_2.md §11.2` ·
  `crates/lodestar-workspace/src/transaction.rs:165-188` ·
  `crates/lodestar-workspace/src/failpoints.rs:45-64` (la taxonomía) ·
  `crates/lodestar-app/src/lib.rs` (`change_apply_uncounted:1644`, `change_revert_uncounted:1771`) ·
  `crates/lodestar-workspace/src/receipts.rs` (`write_receipt`, `gc_receipts:224`).
- **Alcance**:
  - **El recibo se persiste antes del punto de no retorno.** Las revisiones que lo componen ya se
    conocen antes del primer rename: `previous` en el paso (3) (`transaction.rs:119`) y `result_rev`
    en `transaction.rs:152`, que es la que estampa el journal. Degradar los fallos
    post-publicación a *warning* **no basta**: no cubre el `SIGKILL`, que es el caso que de verdad
    ocurre. Lo que cubre los dos es escribir el recibo (o su registro durable equivalente) **con el
    journal**, y que la recuperación por la vía «COMPLETAR» (`recovery.rs:270-272`) lo dé por bueno.
  - **Ningún paso posterior a la publicación puede convertir el resultado en `Err`.** Sellado,
    limpieza de staging y GC pasan a ser best-effort con aviso por stderr, con el mismo criterio ya
    escrito para el GC en `receipts.rs:346-348` y para el `.gitignore` en `gitignore.rs:69-71`.
  - **Taxonomía de failpoints ampliada al camino de la fachada**: hoy `FailPoint`
    (`failpoints.rs:45-64`) cubre seis puntos, todos dentro de `apply_transaction`; los dos
    post-publicación existen pero **nadie los ejerce a través de `App::change_apply`**. Se añade el
    punto que falta —entre el retorno de `apply_transaction` y el recibo— y se propaga la feature
    `test-failpoints` a `lodestar-app` (passthrough de Cargo) para poder armarlo desde sus tests. El
    step de CI que hoy corre `cargo test -p lodestar-workspace --features test-failpoints` se amplía
    al crate nuevo.
- **Criterios de aceptación**:
  - **Dado** un failpoint armado **después** de la publicación y **antes** del recibo, **Cuando** se
    llama a `App::change_apply`, **Entonces** el canónico está publicado **y** existe un recibo
    válido → `publicar_implica_recibo`.
  - **Dado** ese mismo workspace, **Cuando** se llama a `change_revert` con ese `receiptId`,
    **Entonces** revierte correctamente → `tras_fallo_post_publicacion_el_revert_funciona` (hoy:
    `PLAN_EXPIRED` para siempre).
  - **Dado** un fallo de sellado, de limpieza de staging o del GC, **Cuando** ocurre después de
    publicar, **Entonces** `change_apply` devuelve **éxito** →
    `el_cierre_no_convierte_un_apply_publicado_en_error`.
  - **Dado** un `SIGKILL` real entre el último rename y el sellado, **Cuando** se reabre y se
    recupera, **Entonces** hay recibo y la transacción es reversible →
    `crash_tras_publicar_deja_transaccion_reversible` (extiende el arnés de señal de E24-H14).
  - **Dado** un apply que falla **antes** del primer rename, **Cuando** termina, **Entonces**
    **no** hay recibo y `change_revert` sigue respondiendo `PLAN_EXPIRED` →
    `un_apply_no_publicado_no_deja_recibo` (control anti-vacuo: el arreglo no puede consistir en
    escribir recibos siempre).
- **Dependencias**: **E25-H03**.
- **Pruebas**: `crates/lodestar-app/tests/escritura.rs` (con la feature propagada) ·
  `crates/lodestar-workspace/tests/transactions.rs` · `crates/lodestar-mcp/tests/crash_senal.rs`
  (el escenario de señal) · `.github/workflows/ci.yml` (el step ampliado).
- **Frontera (mcp.yml)**: no (cambia cuándo se emiten `PLAN_EXPIRED`/`PLAN_STALE`, no el catálogo ni
  la forma).

### E25-H05 — Borrar es durable, y revertir re-verifica bajo el lock

- **Objetivo**: cerrar los dos huecos que quedan por donde un cambio publicado puede deshacerse solo
  (un borrado que reaparece) o pisar a otro (un revert que sobrescribe una edición ajena).
- **Defectos (S6 + S7) y escenarios que los reproducen hoy**:
  - **(a) Borrado no durable**: `io::delete` (`crates/lodestar-workspace/src/io.rs:46-52`) hace
    `remove_file` y nada más — sin fsync del directorio padre. Un corte de energía después de que el
    journal quede `applied` puede dejar el unlink sin persistir: al reabrir, el documento **reaparece**
    y el recibo afirma que se borró. Además, el fsync de directorio de `write_atomic` es
    **best-effort silencioso** (`io.rs:35-41`, `let _ = dir.sync_all()`): un fallo de durabilidad del
    rename no se entera nadie. Es la mitad que falta de la durabilidad que `write_atomic` sí cuida
    para el contenido (`io.rs:29`, `sync_all` antes del rename).
  - **(b) Revert sin re-verificación bajo el lock**: `change_revert_uncounted` compara la revisión
    actual con `receipt.result_revision` en `crates/lodestar-app/src/lib.rs:1788-1801`, **antes** de
    tomar el lock — el lock lo toma `revert_transaction` en `recovery.rs:442`. En la ventana entre
    las dos, otro escritor puede tocar un `.md` afectado: la reversión lo sobrescribe con la copia
    respaldada, en silencio. La simetría con el apply está rota: `apply_transaction` **sí** vuelve a
    comprobar la base bajo el lock (`reverify_base_revision`, `transaction.rs:147`) y `revert_transaction`
    **no tiene equivalente**.
- **Referencias**: `ARCHITECTURE.md §19.5` (único escritor, `§11.3` reversión) ·
  `crates/lodestar-workspace/src/io.rs` (`write_atomic:18`, `delete:46`) ·
  `crates/lodestar-workspace/src/recovery.rs` (`revert_transaction:436`, pasos (2)-(9)) ·
  `crates/lodestar-workspace/src/lib.rs` (`reverify_base_revision:203`) ·
  `crates/lodestar-app/src/lib.rs` (`change_revert_uncounted:1771`).
- **Alcance**:
  - `io::delete` fsynca el directorio padre tras el unlink, con el mismo `#[cfg(unix)]` que ya usa
    `write_atomic`.
  - El fsync de directorio deja de ser silencioso en los dos sitios: un fallo se propaga como
    `WorkspaceError::Io`. En plataformas donde abrir un directorio no es posible el comportamiento no
    cambia (el bloque sigue siendo `#[cfg(unix)]`).
  - `revert_transaction` recibe la revisión que la fachada observó y la **re-verifica bajo el lock**,
    después de la recuperación del paso (2) y antes de la primera escritura; si difiere →
    `WriteConflict`. Se reusa `reverify_base_revision` (`lib.rs:203`), no se escribe una segunda
    comprobación (invariante #3).
- **Fuera de alcance, con motivo**: pasar la reversión por `validate_staging`. El estado al que
  revierte es un estado que **ya estuvo publicado y validado**; someterlo al gate diferencial de
  E20-H04 podría bloquear un *undo* legítimo, que es justo la operación que nunca debe estar
  bloqueada. Queda declarado aquí para que la próxima auditoría no lo redescubra como olvido.
- **Criterios de aceptación**:
  - **Dado** un `change_revert` en curso, **Cuando** otro escritor modifica un `.md` afectado entre
    la comprobación de la fachada y la toma del lock (mismo gancho de E25-H01), **Entonces** la
    reversión falla con `WRITE_CONFLICT` y **no** escribe nada →
    `revert_con_edicion_externa_en_la_ventana_da_write_conflict` (hoy la pisa en silencio).
  - **Dado** un `change_revert` sin interferencia, **Cuando** se ejecuta, **Entonces** restaura
    exactamente igual que en v0.3.1 → `revert_sin_interferencia_sigue_funcionando` (control
    anti-vacuo).
  - **Dado** un directorio cuyo fsync falla, **Cuando** se escribe o se borra un `.md`, **Entonces**
    la operación devuelve `WorkspaceError::Io` en vez de seguir como si nada →
    `fallo_de_fsync_de_directorio_es_visible`.
  - **Estructural (checklist binario)**: ni `io::delete` ni `io::write_atomic` contienen ya un
    `let _ =` sobre una operación de durabilidad — revisión de diff sobre `io.rs`.
  - **No regresión**: `reference_roots_inmutable`, los tests de reversión de
    `crates/lodestar-app/tests/escritura.rs` y el benchmark `§17` siguen verdes.
- **Dependencias**: **E25-H04**.
- **Pruebas**: `crates/lodestar-workspace/tests/transactions.rs` ·
  `crates/lodestar-app/tests/escritura.rs`.
- **Frontera (mcp.yml)**: no.

---

## Bloque C — Propiedad y ficheros del usuario

### E25-H06 — El lock tiene dueño demostrable, y el `.gitignore` del usuario se respeta

- **Objetivo**: que nadie libere un lock que no es suyo, y que el único fichero **versionado** del
  usuario que Lodestar toca se escriba con el mismo cuidado que un `.md`.
- **Defectos (S4 + S9, menores) y escenarios que los reproducen hoy**:
  - **(a) Lock sin prueba de propiedad**:
    - `Drop for WorkspaceLock` borra el fichero **por ruta**, sin comprobar que sigue siendo el suyo
      (`crates/lodestar-workspace/src/lock.rs:35-42`). Si otro proceso reclamó el lock por huérfano
      (`reclamar_si_huerfano:162`) y lo recreó, el `Drop` del dueño original **borra el lock del
      nuevo dueño** — y a partir de ahí la cascada: cada `Drop` libera el lock del siguiente.
    - El TTL de 15 minutos (`lock.rs:137`) es **wall-clock** (`lock.rs:176-184`): reclama el lock de
      un dueño **vivo pero suspendido** (portátil dormido, proceso parado en un breakpoint, máquina
      con el reloj movido). Que el dueño esté vivo se comprueba (`dueño_muerto`, `lock.rs:183`) pero
      **no manda**: basta con `caducado` para reclamar (`lock.rs:186`).
    - La identidad es incompleta: `lock_metadata` escribe `owner`/`pid`/`timestamp`
      (`lock.rs:234-245`) sin host ni boot-id, y `proceso_muerto` (`lock.rs:207-220`) pregunta por el
      pid **en la máquina local** — sobre un workspace en red o entre namespaces de PID, un pid de
      otra máquina se juzga como si fuera de esta.
  - **(b) `.gitignore` reescrito sin atomicidad y con el fin de línea normalizado**:
    `gitignore::ensure_gitignore` reconstruye el fichero con `str::lines`
    (`crates/lodestar-workspace/src/gitignore.rs:53`), lo que **descarta los `\r`**, y lo escribe con
    `std::fs::write` (`gitignore.rs:69`) — no atómico. Es el **único** fichero versionado del usuario
    que el motor modifica, se toca en cada `acquire_lock` (`lock.rs:78`) y en cada escritura
    (E23-H12), y un `.gitignore` con CRLF se convierte a LF sin avisar: un diff espurio en el repo
    del usuario, o un fichero a medias si el proceso muere durante la escritura.
- **Referencias**: `ARCHITECTURE.md §19.5` (lock de publicación) · `ARCHITECTURE.md §20.13` (el
  `.gitignore` como texto plano, sin git) · `crates/lodestar-workspace/src/lock.rs` ·
  `crates/lodestar-workspace/src/gitignore.rs` · `crates/lodestar-workspace/src/io.rs:18`
  (el protocolo atómico que ya existe).
- **Alcance**:
  - **Token de propiedad**: el cuerpo del lock gana un token único por adquisición, y
    `Drop for WorkspaceLock` **solo borra si el token del fichero coincide** con el suyo. Un token
    distinto significa «este lock ya no es mío»: se deja intacto (best-effort, sin panic — la regla
    de `lock.rs:37-39` sigue mandando).
  - **Identidad de máquina**: `lock_metadata` incorpora un identificador de host (y de arranque
    donde sea barato obtenerlo). `proceso_muerto` solo se consulta cuando el host coincide; si no
    coincide, el único criterio admisible es el TTL.
  - **No reclamar a un vivo**: un pid **vivo local** impide el reclamo aunque el TTL haya vencido.
    El TTL sigue siendo la red portable para cuando no se puede afirmar nada (Windows, cuerpo
    ilegible, otro host), tal como su rustdoc ya declara (`lock.rs:130-137`).
  - **`.gitignore` atómico y respetuoso**: la reescritura pasa por el protocolo temp+fsync+rename, y
    el estilo de fin de línea dominante del fichero se **preserva** (si el fichero venía en CRLF, se
    reemite en CRLF, incluidas las líneas nuevas). La idempotencia byte-a-byte
    (`gitignore.rs:40-45`) se mantiene: un fichero ya gestionado no se reescribe.
- **Fuera de alcance**: convertir el lock en bloqueante, o añadir un `--force` de liberación manual.
- **Criterios de aceptación**:
  - **Dado** un lock reclamado por huérfano y recreado por un segundo dueño, **Cuando** el guard del
    **primer** dueño se dropea, **Entonces** el fichero de lock del segundo **sigue existiendo** →
    `drop_no_borra_un_lock_ajeno` (hoy lo borra, y encadena).
  - **Dado** un lock cuyo `timestamp` es más viejo que el TTL pero cuyo pid está **vivo** en esta
    máquina, **Cuando** otro proceso intenta adquirirlo, **Entonces** falla con `WRITE_CONFLICT` y el
    lock no se reclama → `no_se_reclama_el_lock_de_un_pid_vivo` (hoy se reclama).
  - **Dado** un lock cuyo metadata declara **otro host**, **Cuando** se examina, **Entonces** el pid
    no se usa como criterio y solo decide el TTL → `pid_de_otro_host_no_decide`.
  - **Dado** un lock realmente huérfano (dueño muerto, mismo host), **Cuando** otro proceso lo
    adquiere, **Entonces** lo reclama como hasta ahora → los tests de reclamo de E23-H23 siguen
    verdes (control anti-vacuo).
  - **Dado** un `.gitignore` con CRLF, **Cuando** se ajusta el bloque gestionado, **Entonces** el
    fichero conserva CRLF en todas sus líneas → `gitignore_conserva_crlf` (hoy las convierte a LF).
  - **Dado** un `.gitignore` ya gestionado, **Cuando** se vuelve a ajustar, **Entonces** no se
    reescribe ni un byte → `preserva_contenido_propio_y_es_idempotente` (existente, sigue verde) y no
    queda ningún `*.lodestar-tmp` en la raíz → `gitignore_no_deja_temporales`.
  - **Estructural (checklist binario)**: `gitignore.rs` no contiene ya `std::fs::write` sobre el
    `.gitignore` — revisión de diff.
- **Dependencias**: **E25-H05**. (Además, si **E25-H03** eligió la marca durable, esta historia es la
  que endurece el criterio de propiedad que aquella reusa: la marca hereda token y host.)
- **Pruebas**: `crates/lodestar-workspace/tests/transactions.rs` (bloque de lock) ·
  los tests unitarios de `crates/lodestar-workspace/src/gitignore.rs:74-109`.
- **Frontera (mcp.yml)**: no.

---

## Orden de construcción

```
H01 ─→ H02 ─→ H03 ─→ H04 ─→ H05 ─→ H06
```

**Estrictamente secuencial**, y no por conveniencia: las seis tocan el mismo camino y las cuatro
primeras el mismo fichero (`transaction.rs`/`recovery.rs`/`receipts.rs`). **H01** fija el contrato
«lo respaldado es lo publicable», del que **H02** depende para poder hablar de copias verificadas;
**H03** necesita ese contrato para decidir qué es «una transacción viva»; **H04** cierra el otro
extremo del mismo camino (después de publicar) y su seam de failpoints en `lodestar-app` es el que
**H05** reusa para el revert; **H06** es la única independiente en contenido, y va al final porque el
criterio de propiedad que endurece lo puede haber adoptado H03.

Ninguna historia está **[BLOQUEADA]**: no dependen de ninguna decisión abierta de `DECISIONES.md`.

## Proceso por historia

| Ciclo | Historias | Por qué |
|---|---|---|
| **Completo** (spec → roja → verde → juez ciego con *mutation testing*) | H01 · H02 · H03 · H04 | Pérdida silenciosa de datos o sellado silencioso de estados parciales |
| **Corto** (regresión en rojo → fix → verificación) | H05 · H06 | Defectos acotados, con escenario reproducible directo |

**Advertencia de proceso, heredada de E23 y E24**: los seis defectos son de concurrencia o de
durabilidad, y **leer el código no basta para dar ninguno por arreglado**. Cada historia se cierra
ejecutando su escenario, no revisando su diff — y el juez ciego de las cuatro del ciclo completo
recibe el encargo explícito de hacer *mutation testing* sobre el arreglo.

## Criterio de salida

Una publicación escribe exactamente lo que respaldó, o aborta antes del primer rename; una copia de
recuperación es durable y se verifica antes de restaurarse, y un journal irrecuperable manda su
material a cuarentena en vez de cerrar el workspace para siempre; el GC no puede desarmar a una
transacción viva de otro proceso; si el canónico cambió, hay recibo y el *undo* funciona —también
tras un `SIGKILL`—; un borrado es tan durable como una escritura y una reversión re-verifica bajo el
lock; y ni el lock se libera por manos ajenas ni el `.gitignore` del usuario cambia de fin de línea
a sus espaldas.
