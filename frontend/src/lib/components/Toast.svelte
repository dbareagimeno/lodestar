<script lang="ts">
  import { toastState } from "../stores/ui";
</script>

<div class="toast" class:on={!!$toastState}>
  {#if $toastState}
    <span>{$toastState.msg}</span>
    {#if $toastState.action}
      <button class="toast-act" onclick={() => { $toastState!.action!.fn(); $toastState = null; }}>{$toastState.action.label}</button>
    {/if}
  {/if}
</div>
