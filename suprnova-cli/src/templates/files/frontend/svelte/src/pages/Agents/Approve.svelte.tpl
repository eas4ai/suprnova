<script lang="ts">
  import { Form, usePage } from '@inertiajs/svelte'
  import Head from '../../components/Head.svelte'
  import type { ApprovalProps } from '../../types/inertia-props'

  // The page an MCP client sends the user to after they sign in: the
  // client, its host, where the user returns and every permission it asks
  // for, with Allow and Deny. `controllers::agents::approval` renders it.
  //
  // The client chose its name and registered its redirect URI, so each
  // sits in `<bdi>`: whatever reordering the text carries stays inside it.
  // The page loads and links nothing the client chose. It offers no tool
  // to an agent in the browser: consent comes from the person.
  //
  // Each button is a form of its own with its decision in a hidden field,
  // so no form relies on the clicked button's name reaching the server.
  // The server answers a decision with a location visit to the client's
  // redirect URI, which the browser follows itself.
  let { approval }: ApprovalProps = $props()

  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to is built from it, so one build runs at
  // both. `types/inertia-props.ts` types it, so `usePage()` takes no
  // argument.
  const { root } = usePage().props
</script>

<Head title="Allow an app to act for you?" />

<div class="mx-auto max-w-2xl space-y-4 rounded-lg bg-white p-6 shadow">
  <h2 class="text-xl font-semibold text-gray-900">
    Allow <bdi>{approval.client_name}</bdi> to act for you?
  </h2>
  <p class="text-sm text-gray-700">
    From <strong><bdi>{approval.client_host}</bdi></strong>, it wants to use this application as you.
  </p>
  {#if approval.scopes.length === 0}
    <p class="text-sm text-gray-700">It asks for no permissions: it can use only what needs none.</p>
  {:else}
    <div>
      <h3 class="text-sm font-medium text-gray-900">It asks to</h3>
      <ul class="mt-2 list-disc space-y-1 pl-5 text-sm text-gray-700">
        {#each approval.scopes as scope (scope.name)}
          <li>{scope.description} (<code>{scope.name}</code>)</li>
        {/each}
      </ul>
    </div>
  {/if}
  <p class="text-sm text-gray-700">
    If you allow it, you return to <bdi><code class="break-all">{approval.redirect_uri}</code></bdi>.
  </p>
  <div class="flex gap-3">
    <Form action={`${root}/oauth/approve`} method="post">
      {#snippet children({ processing })}
        <input type="hidden" name="authorization" value={approval.id} />
        <input type="hidden" name="decision" value="approve" />
        <button
          type="submit"
          disabled={processing}
          class="rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-50"
        >
          Allow
        </button>
      {/snippet}
    </Form>
    <Form action={`${root}/oauth/approve`} method="post">
      {#snippet children({ processing })}
        <input type="hidden" name="authorization" value={approval.id} />
        <input type="hidden" name="decision" value="deny" />
        <button
          type="submit"
          disabled={processing}
          class="rounded-md border border-gray-300 px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50"
        >
          Deny
        </button>
      {/snippet}
    </Form>
  </div>
</div>
