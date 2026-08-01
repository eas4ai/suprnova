<script lang="ts">
  import { router } from '@inertiajs/svelte'
  import type { PublicUserProps } from '../../types/inertia-props'

  // `GET /users` serves this component via `Inertia::paginate`, so the
  // rows arrive under `users` and the page cursor under `scrollProps`.
  // The Rust side projects to id + name deliberately — the route is
  // unauthenticated and the full user DTO carries an email address.
  let { users }: { users: PublicUserProps[] } = $props()
</script>

<div class="font-sans p-8 max-w-2xl mx-auto">
  <h1 class="text-3xl font-bold">Users</h1>

  <ul class="mt-6 divide-y">
    {#each users as user (user.id)}
      <li class="py-3 flex items-center justify-between">
        <button
          class="text-left hover:underline"
          onclick={() => router.visit(`/users/${user.id}`)}
        >
          {user.name}
        </button>
        <span class="text-gray-400 text-sm">#{user.id}</span>
      </li>
    {:else}
      <li class="py-3 text-gray-500">No users yet.</li>
    {/each}
  </ul>
</div>
