<script lang="ts">
  // Pantalla de bienvenida (first-run / sin workspace). Overlay a pantalla completa NO descartable:
  // sin un workspace abierto la app no tiene datos que mostrar. Usa el lenguaje visual de los
  // diálogos (.scrim/.dialog, botones y variables CSS del proyecto). Es reactivo al snapshot: en
  // cuanto hay bundle abierto, desaparece solo.
  import { snapshot } from "../stores/bundle";
  import { openBundle, pickFolder, createBundle } from "../ipc";

  let busy = $state(false);
  let error = $state<string | null>(null);
  // Cuando openBundle falla porque la carpeta elegida no es un workspace, ofrecemos crearlo ahí.
  let offerCreatePath = $state<string | null>(null);

  function persist(path: string) {
    try {
      localStorage.setItem("lodestar:workspace", path);
    } catch {
      /* localStorage no disponible: no es crítico */
    }
  }

  function isNotWorkspace(msg: string): boolean {
    return /no es un workspace/i.test(msg);
  }

  async function doOpen() {
    if (busy) return;
    error = null;
    offerCreatePath = null;
    busy = true;
    try {
      const path = await pickFolder();
      if (!path) return; // cancelado
      try {
        const snap = await openBundle(path);
        persist(path);
        snapshot.set(snap);
      } catch (e) {
        const msg = String((e as Error)?.message ?? e);
        if (isNotWorkspace(msg)) {
          offerCreatePath = path;
        } else {
          error = msg;
        }
      }
    } catch (e) {
      error = String((e as Error)?.message ?? e);
    } finally {
      busy = false;
    }
  }

  async function doCreatePicked() {
    if (busy) return;
    error = null;
    busy = true;
    try {
      const path = await pickFolder();
      if (!path) return; // cancelado
      const snap = await createBundle(path);
      persist(path);
      snapshot.set(snap);
    } catch (e) {
      error = String((e as Error)?.message ?? e);
    } finally {
      busy = false;
    }
  }

  async function doCreateHere(path: string) {
    if (busy) return;
    error = null;
    busy = true;
    try {
      const snap = await createBundle(path);
      persist(path);
      snapshot.set(snap);
    } catch (e) {
      error = String((e as Error)?.message ?? e);
    } finally {
      busy = false;
    }
  }
</script>

{#if $snapshot === null}
  <div class="scrim on welcome-scrim" role="dialog" aria-modal="true" aria-label="Elegir workspace">
    <div class="dialog welcome-dialog">
      <h3>lodestar</h3>
      <p>
        Para empezar necesitas un <b>workspace</b>: un directorio con tus ficheros
        <span class="mono">.md</span>. Ábre uno existente o crea uno nuevo.
      </p>

      {#if offerCreatePath}
        <div class="fld-note warn welcome-note">
          <span
            >Esa carpeta no es un workspace lodestar todavía:
            <span class="mono">{offerCreatePath}</span></span
          >
        </div>
        <div class="row">
          <button class="btn-ghost" disabled={busy} onclick={() => (offerCreatePath = null)}
            >Elegir otra</button
          >
          <button class="btn-primary" disabled={busy} onclick={() => doCreateHere(offerCreatePath!)}
            >Crear workspace aquí</button
          >
        </div>
      {:else}
        <div class="row welcome-actions">
          <button class="btn-ghost" disabled={busy} onclick={doOpen}>Abrir carpeta…</button>
          <button class="btn-primary" disabled={busy} onclick={doCreatePicked}
            >Crear workspace…</button
          >
        </div>
      {/if}

      {#if error}
        <div class="fld-note err welcome-note">
          <span>{error}</span>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  /* Por encima de cualquier otro diálogo/overlay: sin workspace no hay nada debajo con lo que operar. */
  .welcome-scrim {
    z-index: 90;
  }
  .welcome-actions {
    justify-content: flex-start;
  }
  .welcome-note {
    margin-top: 12px;
  }
</style>
