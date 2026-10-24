<script lang="ts">
  import { page } from '@inertiajs/svelte'

  // The toast a handler flashed with `Inertia::flash("toast", ..)`. The
  // server sends it in the `flash` field of one page response and removes
  // it, so the next page's `flash` no longer has it
  // (`packages/core/src/types.ts`, `Page.flash`). Rendering it straight
  // from the page shows it exactly once; copying it into component state
  // would keep it on screen after the page that carried it. Its type is the
  // generated `flashDataType` in `types/inertia-props.ts`.
  const toast = $derived(page.flash.toast)
</script>

{#if toast}
  <div
    role="status"
    class="mx-auto mt-4 max-w-md rounded-md border px-4 py-3 text-sm shadow-sm"
    class:border-green-200={toast.kind === 'success'}
    class:bg-green-50={toast.kind === 'success'}
    class:text-green-800={toast.kind === 'success'}
    class:border-blue-200={toast.kind === 'info'}
    class:bg-blue-50={toast.kind === 'info'}
    class:text-blue-800={toast.kind === 'info'}
    class:border-red-200={toast.kind === 'error'}
    class:bg-red-50={toast.kind === 'error'}
    class:text-red-800={toast.kind === 'error'}
  >
    {toast.message}
  </div>
{/if}
