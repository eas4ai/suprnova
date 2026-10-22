<script lang="ts">
  import { Form, usePage } from '@inertiajs/svelte'
  import Head from '../../components/Head.svelte'
  import { formatDate } from '../../lib/dates'
  import type { AgentView, ConnectedAgentsProps } from '../../types/inertia-props'

  // The apps (MCP clients the user approved) and the agents (registrations
  // the user claimed) that act for the signed-in user, newest first, each
  // with a revoke button. `controllers::agents::connected` renders it.
  //
  // A client chose its name, so the name and the host sit in `<bdi>`. An
  // agent's provider is the application's trust list entry. Revoking ends
  // the access and every token at once.
  let { clients, agents }: ConnectedAgentsProps = $props()

  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to is built from it, so one build runs at
  // both. `types/inertia-props.ts` types it, so `usePage()` takes no
  // argument.
  const { root } = usePage().props

  /** How an agent registered, in words. */
  function registeredHow(agent: AgentView): string {
    switch (agent.registration_type) {
      case 'anonymous':
        return 'Registered on its own and claimed by you'
      case 'service_auth':
        return 'Registered with your email address'
      case 'identity_assertion':
        return 'Signed in through a provider you use'
      default:
        return 'An agent account'
    }
  }
</script>

<Head title="Connected agents" />

<div class="space-y-6">
  <section class="rounded-lg bg-white p-6 shadow">
    <h2 class="text-lg font-semibold text-gray-900">Apps</h2>
    {#if clients.length === 0}
      <p class="mt-2 text-sm text-gray-500">No apps can act for you.</p>
    {:else}
      <ul class="mt-4 divide-y divide-gray-100">
        {#each clients as client (client.id)}
          <li class="flex items-start justify-between gap-4 py-3">
            <div class="text-sm text-gray-700">
              <p class="font-medium text-gray-900"><bdi>{client.client_name}</bdi></p>
              <p>
                From <bdi>{client.client_host}</bdi>. Allowed {formatDate(client.created_at)}, last used
                {client.last_used_at === null ? 'never' : formatDate(client.last_used_at)}.
              </p>
              {#if client.scopes.length > 0}
                <ul class="mt-1 list-disc pl-5">
                  {#each client.scopes as scope (scope.name)}
                    <li>{scope.description} (<code>{scope.name}</code>)</li>
                  {/each}
                </ul>
              {/if}
            </div>
            <Form action={`${root}/connected-agents/clients/${encodeURIComponent(client.id)}/revoke`} method="post">
              {#snippet children({ processing })}
                <button
                  type="submit"
                  disabled={processing}
                  class="rounded-md border border-red-300 px-3 py-1 text-sm text-red-700 hover:bg-red-50 disabled:opacity-50"
                >
                  Revoke
                </button>
              {/snippet}
            </Form>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <section class="rounded-lg bg-white p-6 shadow">
    <h2 class="text-lg font-semibold text-gray-900">Agents</h2>
    {#if agents.length === 0}
      <p class="mt-2 text-sm text-gray-500">No agents can act for you.</p>
    {:else}
      <ul class="mt-4 divide-y divide-gray-100">
        {#each agents as agent (agent.id)}
          <li class="flex items-start justify-between gap-4 py-3">
            <div class="text-sm text-gray-700">
              <p class="font-medium text-gray-900">
                {agent.provider === null ? 'Agent' : `Agent through ${agent.provider}`}
              </p>
              <p>
                {registeredHow(agent)}. Registered {formatDate(agent.created_at)}{agent.claimed_at === null
                  ? ''
                  : `, claimed ${formatDate(agent.claimed_at)}`}.
              </p>
              {#if agent.scopes.length > 0}
                <ul class="mt-1 list-disc pl-5">
                  {#each agent.scopes as scope (scope.name)}
                    <li>{scope.description} (<code>{scope.name}</code>)</li>
                  {/each}
                </ul>
              {/if}
            </div>
            <Form action={`${root}/connected-agents/agents/${encodeURIComponent(agent.id)}/revoke`} method="post">
              {#snippet children({ processing })}
                <button
                  type="submit"
                  disabled={processing}
                  class="rounded-md border border-red-300 px-3 py-1 text-sm text-red-700 hover:bg-red-50 disabled:opacity-50"
                >
                  Revoke
                </button>
              {/snippet}
            </Form>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
</div>
