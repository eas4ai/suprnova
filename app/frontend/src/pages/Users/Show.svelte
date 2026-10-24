<script lang="ts">
  import type { UserDetailProps } from '../../types/inertia-props'

  // `GET /users/{id}` eager-loads the `profile` HasOne, so `bio` is
  // `string | null`: null means the user has no profile row, which is a
  // different fact from an empty bio and is rendered as such.
  let { user }: { user: UserDetailProps } = $props()
</script>

<div class="font-sans p-8 max-w-2xl mx-auto">
  <h1 class="text-3xl font-bold">{user.name}</h1>
  <p class="mt-1 text-gray-400 text-sm">#{user.id}</p>

  {#if user.bio === null}
    <p class="mt-6 text-gray-500 italic">This user has no profile.</p>
  {:else}
    <p class="mt-6">{user.bio}</p>
  {/if}

  <a href="/users" class="mt-8 inline-block text-blue-600 hover:underline">
    Back to users
  </a>
</div>
