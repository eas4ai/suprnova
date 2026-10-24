<script lang="ts">
  import { Form } from '@inertiajs/svelte'
  import type { ClaimProps } from '../../types/inertia-props'

  // The page an agent sends the user to: who the agent will act as, with
  // which permissions, and the form that takes the code the agent gave the
  // user. `controllers::agents::claim` renders it.
  //
  // The user's address and the address the agent named sit in `<bdi>`, as
  // any text someone other than this application chose. The provider's
  // name is the application's trust list entry, never one the provider
  // sent. The page offers no tool to an agent in the browser: consent
  // comes from the person.
  //
  // A wrong code comes back as the code field's error, with the attempts
  // left.
  //
  // The kit's page, in this application's frame: there is no layout, so the
  // page draws its own background, and the application is served at the
  // host root, so its forms post to fixed paths rather than ones built from
  // a shared `root`.
  let { claim, claim_attempt_token }: ClaimProps = $props()

  const minutes = $derived(Math.ceil(claim.expires_in_seconds / 60))
</script>

<svelte:head>
  <title>Let an agent act for you? - Suprnova App</title>
</svelte:head>

<main class="min-h-screen bg-gray-50 px-4 py-12">
  <div class="mx-auto max-w-2xl space-y-4 rounded-lg bg-white p-6 shadow">
    <h2 class="text-xl font-semibold text-gray-900">Let an agent act for you?</h2>
    <p class="text-sm text-gray-700">
      {#if claim.provider !== null}
        An agent signed in through <strong>{claim.provider}</strong> asks to act as you.
      {:else if claim.registration_type === 'service_auth'}
        An agent that named your email address asks to act as you.
      {:else}
        An agent that registered on its own asks to act as you.
      {/if}
    </p>
    <p class="text-sm text-gray-700">
      If you allow it, the agent will use this application as you, with the permissions below, until
      you revoke it.
    </p>
    <p class="text-sm text-gray-700">
      You are signed in as <strong><bdi>{claim.signed_in_as}</bdi></strong>.
      {#if claim.email !== null}
        The agent named <strong><bdi>{claim.email}</bdi></strong>.
      {/if}
    </p>
    {#if claim.first_time_account}
      <p class="rounded-md bg-yellow-50 p-3 text-sm text-yellow-800">
        <strong>Warning:</strong> no agent has acted for your account before. Allow it only if you asked
        an agent to connect.
      </p>
    {/if}
    {#if claim.first_time_provider}
      <p class="rounded-md bg-yellow-50 p-3 text-sm text-yellow-800">
        <strong>Warning:</strong> no agent from this provider has acted for you before.
      </p>
    {/if}
    {#if claim.scopes.length === 0}
      <p class="text-sm text-gray-700">It asks for no permissions: it can use only what needs none.</p>
    {:else}
      <div>
        <h3 class="text-sm font-medium text-gray-900">It will be able to</h3>
        <ul class="mt-2 list-disc space-y-1 pl-5 text-sm text-gray-700">
          {#each claim.scopes as scope (scope.name)}
            <li>{scope.description} (<code>{scope.name}</code>)</li>
          {/each}
        </ul>
      </div>
    {/if}
    <Form action="/agent/claim" method="post" resetOnError={['user_code']} class="space-y-3">
      {#snippet children({ errors, processing })}
        <input type="hidden" name="claim_attempt_token" value={claim_attempt_token} />
        <p class="text-sm text-gray-700">
          Enter the six-digit code the agent gave you. It works for about {minutes} more minutes,
          {claim.attempts_remaining} wrong entries allowed.
        </p>
        <div>
          <label for="user_code" class="block text-sm font-medium text-gray-700">Code</label>
          <input
            id="user_code"
            name="user_code"
            type="text"
            inputmode="numeric"
            autocomplete="one-time-code"
            maxlength="9"
            required
            class="mt-1 block w-40 rounded-md border border-gray-300 px-3 py-2 text-sm shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500"
          />
          {#if errors.user_code}
            <p class="mt-1 text-sm text-red-600">{errors.user_code}</p>
          {/if}
        </div>
        <button
          type="submit"
          disabled={processing}
          class="rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-50"
        >
          Allow
        </button>
      {/snippet}
    </Form>
    <p class="text-sm text-gray-500">If you did not ask an agent to connect, close this page.</p>
  </div>
</main>
