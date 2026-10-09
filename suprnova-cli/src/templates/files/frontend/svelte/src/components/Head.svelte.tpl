<script lang="ts">
  import type { Snippet } from 'svelte'
  import { pageTitle } from '../lib/title'

  // A page's tab title. `@inertiajs/svelte` 3.8.0 has no `Head` component
  // and its `createInertiaApp` reads no `title` option (see
  // `packages/svelte/src/index.ts` and `createInertiaApp.ts`): a Svelte page
  // titles itself with Svelte's own `<svelte:head>`. This component does
  // that through `pageTitle`, the same function the entries pass as
  // `title`, so every page reads `Notes - My App`. Server rendering puts the
  // title in the document's head, and the server then leaves out its default
  // one. Anything else a page wants in the head goes in as children.
  let { title = '', children }: { title?: string; children?: Snippet } = $props()
</script>

<svelte:head>
  <title>{pageTitle(title)}</title>
  {@render children?.()}
</svelte:head>
