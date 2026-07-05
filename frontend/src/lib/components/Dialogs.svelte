<script lang="ts">
  // Diálogos modales (scrim + dialog). Port de los <div class="dialog"> del prototipo: nueva página,
  // confirmar, guardar versión (commit), ayuda OKF y hoja de atajos.
  import { snapshot } from "../stores/bundle";
  import { dialog, closeDialog, confirmState, createConcept, pendingNewPath } from "../stores/ui";
  import { commitVersion, diffSnap, diffChips, suggestMsg, tipSnapshot } from "../versions";

  let newPath = $state("");
  let newType = $state("Spec");
  let commitMsg = $state("");
  let commitLog = $state(true);

  const TYPES = ["Spec", "Requirement", "ADR", "API Endpoint", "Metric", "Playbook", "Reference", "BigQuery Table", "BigQuery Dataset"];

  // Al abrir "nueva", pre-rellena la ruta (enlace fantasma) y enfoca.
  $effect(() => {
    if ($dialog === "new") {
      newPath = $pendingNewPath;
      newType = "Spec";
      $pendingNewPath = "";
    }
  });

  const files = $derived($snapshot?.files ?? {});
  const commitDiff = $derived($dialog === "commit" ? diffSnap(tipSnapshot(), files) : null);
  const commitChips = $derived(commitDiff ? diffChips(commitDiff) : []);
  const commitPlaceholder = $derived(commitDiff ? suggestMsg(commitDiff, files) : "Describe el cambio…");

  function doCreate() {
    let ty = newType;
    if (ty === "__custom") ty = prompt("Tipo del concept:", "Spec") || "Spec";
    createConcept(newPath, ty);
  }
  function doCommit() {
    commitVersion(commitMsg, { log: commitLog });
    commitMsg = "";
    closeDialog();
  }
</script>

<div class="scrim" class:on={$dialog !== null} onclick={(e) => { if ((e.target as HTMLElement).classList.contains("scrim")) closeDialog(); }} role="presentation">
  {#if $dialog === "new"}
    <div class="dialog">
      <h3>Nueva página</h3>
      <p>Cada página vive en una ubicación dentro del espacio. Esa ubicación es su dirección.</p>
      <div class="fm-grid" style="grid-template-columns:90px 1fr">
        <label>Ubicación</label><input class="fld" placeholder="specs/mi-pagina" spellcheck="false" bind:value={newPath} onkeydown={(e) => e.key === "Enter" && doCreate()} />
        <label>Tipo</label>
        <select class="fld" bind:value={newType}>
          {#each TYPES as t}<option>{t}</option>{/each}
          <option value="__custom">otro…</option>
        </select>
      </div>
      <div class="row">
        <button class="btn-ghost" onclick={closeDialog}>Cancelar</button>
        <button class="btn-primary" onclick={doCreate}>Crear</button>
      </div>
    </div>
  {:else if $dialog === "confirm" && $confirmState}
    <div class="dialog">
      <h3>{$confirmState.title}</h3>
      <p>{$confirmState.msg}</p>
      <div class="row">
        <button class="btn-ghost" onclick={closeDialog}>Cancelar</button>
        <button class="btn-ghost btn-danger" onclick={() => { const y = $confirmState!.onYes; closeDialog(); y(); }}>Confirmar</button>
      </div>
    </div>
  {:else if $dialog === "commit"}
    <div class="dialog">
      <h3>Guardar versión</h3>
      <p>Una versión es un punto al que siempre podrás volver. Se guarda en el historial del espacio (un commit de git, por debajo).</p>
      <div class="commit-sum">
        {#if commitChips.length}
          {#each commitChips as c}<span class="vchip {c.cls}">{c.t}</span>{/each}
        {:else}
          <span class="none">Sólo se regenerarán índices.</span>
        {/if}
        {#if commitDiff && commitDiff.gen.length}<span class="vchip">{commitDiff.gen.length} generado{commitDiff.gen.length > 1 ? "s" : ""}</span>{/if}
      </div>
      <input class="fld" placeholder={commitPlaceholder} spellcheck="false" style="font-family:var(--sans);font-size:13px" bind:value={commitMsg} onkeydown={(e) => e.key === "Enter" && doCommit()} />
      <label class="commit-opt"><input type="checkbox" bind:checked={commitLog} /> Anotar también en el registro (<span class="mono">log.md</span>)</label>
      <div class="row">
        <button class="btn-ghost" onclick={closeDialog}>Cancelar</button>
        <button class="btn-primary" onclick={doCommit}>Guardar versión</button>
      </div>
    </div>
  {:else if $dialog === "help"}
    <div class="dialog help-body">
      <h3>OKF · Open Knowledge Format</h3>
      <p>Estándar abierto de Google Cloud (v0.1, jun 2026) para representar conocimiento como un directorio de ficheros markdown con frontmatter YAML. Un bundle es "solo ficheros": legible por humanos y por agentes, versionable en git, sin SDK.</p>
      <h4>La única regla dura</h4>
      <p>Todo concept (cualquier <code>.md</code> que no sea reservado) lleva frontmatter YAML con un campo <code>type</code> no vacío. Lo demás es opcional.</p>
      <h4>Campos estructurados</h4>
      <ul>
        <li><code>type</code> — obligatorio. Clase del concept.</li>
        <li><code>title, description, resource, tags, timestamp</code> — recomendados.</li>
        <li>Cualquier otra clave la define el productor (aquí: <code>status</code> para el flujo spec-driven).</li>
      </ul>
      <h4>Ficheros reservados</h4>
      <p><code>index.md</code> (listado para divulgación progresiva, sin frontmatter) y <code>log.md</code> (historial de cambios). No son concepts.</p>
      <h4>Índices generados</h4>
      <p>El botón <b>índice</b> escribe el <code>index.md</code> de la raíz. El botón <b>tags</b> sintetiza una vista por tag escaneando el frontmatter: un <code>tags/&lt;tag&gt;/index.md</code> por cada tag más un <code>tags/index.md</code> raíz que los lista. Se regenera (purga los tags que ya no existan), así que no se edita a mano.</p>
      <h4>Enlaces = grafo</h4>
      <p>Los concepts se enlazan con enlaces markdown normales. Se recomienda la forma absoluta <code>/ruta/concept.md</code>. Un enlace a algo que no existe no es un error: es "conocimiento por escribir" — aquí aparece como nodo fantasma y como backlog.</p>
      <h4>Spec-driven</h4>
      <p>La spec es la fuente de verdad. Este editor trata la conformidad como puerta de CI, el <code>status</code> como ciclo de vida, y el grafo como mapa de impacto: enlaces colgantes = specs por escribir, concepts huérfanos = specs que nadie referencia.</p>
      <h4>Búsqueda por frontmatter</h4>
      <p>Filtra el árbol por campos estructurados; la misma query atenúa en el grafo lo que no casa. Términos combinados con Y implícito:</p>
      <ul>
        <li><code>type:Spec</code> · <code>status:review</code> · <code>tags:auth</code> — subcadena (en tags, pertenencia)</li>
        <li><code>type=Spec</code> exacto · <code>-status:draft</code> niega</li>
        <li><code>has:resource</code> / <code>no:description</code> — presencia de campo (auditoría de specs)</li>
        <li><code>is:orphan</code> · <code>is:invalid</code> · <code>is:linked</code> — hechos derivados</li>
        <li><code>body:login</code> busca en el cuerpo; texto suelto busca nombre, frontmatter y cuerpo</li>
      </ul>
      <h4>Atajos</h4>
      <ul><li>Pulsa <code>?</code> en cualquier momento para ver todos los atajos.</li></ul>
      <div class="row"><button class="btn-primary" onclick={closeDialog}>Entendido</button></div>
    </div>
  {:else if $dialog === "keys"}
    <div class="dialog">
      <h3>Atajos de teclado</h3>
      <div class="keys">
        <div class="keyrow"><span>Nueva página</span><kbd>N</kbd></div>
        <div class="keyrow"><span>Buscar</span><kbd>/</kbd></div>
        <div class="keyrow"><span>Alternar Lista / Mapa</span><kbd>M</kbd></div>
        <div class="keyrow"><span>Ocultar / mostrar páginas</span><kbd>[</kbd></div>
        <div class="keyrow"><span>Ocultar / mostrar enlaces</span><kbd>]</kbd></div>
        <div class="keyrow"><span>Modo Leer</span><kbd>1</kbd></div>
        <div class="keyrow"><span>Modo Editar</span><kbd>2</kbd></div>
        <div class="keyrow"><span>Modo Código</span><kbd>3</kbd></div>
        <div class="keyrow"><span>Ver cambios de la página</span><kbd>4</kbd></div>
        <div class="keyrow"><span>Guardar versión</span><kbd>⌘S</kbd></div>
        <div class="keyrow"><span>Historial de versiones</span><kbd>V</kbd></div>
        <div class="keyrow"><span>Esta ayuda</span><kbd>?</kbd></div>
        <div class="keyrow"><span>Cerrar / limpiar</span><kbd>Esc</kbd></div>
      </div>
      <div class="row"><button class="btn-primary" onclick={closeDialog}>Listo</button></div>
    </div>
  {/if}
</div>
