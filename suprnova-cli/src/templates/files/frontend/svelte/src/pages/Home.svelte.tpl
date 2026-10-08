<script lang="ts">
  import Head from '../components/Head.svelte'
  import type { HomeProps } from '../types/inertia-props'
  import { t } from '../lib/lang.svelte'

  let { title, message }: HomeProps = $props()
</script>

<!-- No title of its own: the tab shows the application's name. -->
<Head />

<div class="mx-auto max-w-xl p-8 font-sans">
  <h1 class="text-3xl font-bold">{t('welcome', { app: title })}</h1>
  <p class="mt-2">{message}</p>
  <p class="mt-8 text-gray-500">
    Edit
    <code class="rounded bg-gray-100 px-1">frontend/src/pages/Home.svelte</code>
    to get started.
  </p>
</div>
