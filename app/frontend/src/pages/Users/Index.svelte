<script lang="ts">
  import { InfiniteScroll, Link, usePage } from '@inertiajs/svelte'
  import type { PublicUserProps } from '../../types/inertia-props'

  // `GET /users` and `GET /api/users` serve this component via
  // `Inertia::paginate`, so the rows arrive under `users` and the page or
  // cursor under `scrollProps.users`, which `InfiniteScroll` reads to load
  // the next page as the list's end scrolls into view. The Rust side
  // projects to id + name deliberately - the route is unauthenticated and
  // the full user DTO carries an email address.
  let { users }: { users: PublicUserProps[] } = $props()

  // The public root the server shares with every page (`RootShare`).
  const page = usePage()
</script>

<svelte:head>
  <title>Users</title>
</svelte:head>

<div class="font-sans p-8 max-w-2xl mx-auto">
  <h1 class="text-3xl font-bold">Users</h1>

  <InfiniteScroll data="users" class="mt-6 divide-y">
    {#snippet loading()}
      <p class="py-3 text-gray-500">Loading more users...</p>
    {/snippet}

    {#each users as user (user.id)}
      <Link
        href={`${page.props.root}/users/${user.id}`}
        prefetch
        class="py-3 flex items-center justify-between hover:underline"
      >
        <span>{user.name}</span>
        <span class="text-gray-400 text-sm">#{user.id}</span>
      </Link>
    {:else}
      <p class="py-3 text-gray-500">No users yet.</p>
    {/each}
  </InfiniteScroll>
</div>
