<script lang="ts">
  import {
    Deferred,
    Link,
    router,
    setLayoutProps,
    useHttp,
    usePage,
    usePoll,
    WhenVisible,
  } from '@inertiajs/svelte'
  import { untrack } from 'svelte'
  import Head from '../components/Head.svelte'
  import { formatDate } from '../lib/dates'
  import type { DashboardProps } from '../types/inertia-props'

  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to or links is built from it, so one build
  // runs at both.
  // `types/inertia-props.ts` types it, so `usePage()` takes no argument.
  const { root } = usePage().props

  // The signed-in user, which the server shares with every page as
  // `auth.user`. Read through the page so a change to it shows here.
  const page = usePage()
  const user = $derived(page.props.auth.user)

  // `stats` is deferred and `recent_notes` optional on the server, so both
  // are absent at first: `stats` arrives in a request the client sends
  // right after the page renders, `recent_notes` when it scrolls into view.
  let { stats, recent_notes }: DashboardProps = $props()

  // The application layout shows this heading while the dashboard is the
  // page; a visit to another page resets it.
  setLayoutProps({ heading: 'Dashboard' })

  // Ask the server for `stats` again every ten seconds. `only` keeps the
  // reload to that one prop, and the poll stops when the page goes away.
  usePoll(10000, { only: ['stats'] })

  // The display name form posts JSON to `/profile/name` through `useHttp`,
  // which is not a page visit: the server answers `200`, or a `422` whose
  // `errors` the form takes as its own.
  const nameForm = useHttp({ name: untrack(() => user?.name ?? '') })

  // The new name is shown before the server answers: `router.replaceProp`
  // rewrites the shared `auth.user.name` in place, so this page and the
  // layout both show it at once. A `422` puts the old name back and shows
  // the server's `errors.name`; a request that fails another way puts it
  // back too and says so. After a save, `auth` is reloaded from the server.
  async function saveName(event: SubmitEvent) {
    event.preventDefault()
    const previous = user?.name ?? ''
    router.replaceProp('auth.user.name', nameForm.name)
    try {
      await nameForm.post(`${root}/profile/name`, {
        onSuccess: () => router.reload({ only: ['auth'] }),
        onError: () => router.replaceProp('auth.user.name', previous),
      })
    } catch {
      router.replaceProp('auth.user.name', previous)
      nameForm.setError('name', 'Your name could not be saved. Try again.')
    }
  }
</script>

<Head title="Dashboard" />

<!-- Tall on purpose: the recent notes below start out of view, so
     `WhenVisible` loads them only once the visitor scrolls down. -->
<section class="min-h-screen space-y-6">
  {#if user}
    <div class="rounded-lg bg-white p-6 shadow">
      <h2 class="text-xl font-bold text-gray-900">Welcome, {user.name}!</h2>
      <p class="mt-1 text-sm text-gray-500">{user.email}</p>
    </div>
  {/if}

  <div class="rounded-lg bg-white p-6 shadow">
    <h2 class="text-lg font-semibold text-gray-900">Your notes</h2>
    <Deferred data="stats">
      {#snippet fallback()}
        <p class="mt-4 text-sm text-gray-500">Counting your notes...</p>
      {/snippet}

      {#if stats}
        <dl class="mt-4 grid grid-cols-2 gap-4">
          <div>
            <dt class="text-sm text-gray-500">Notes</dt>
            <dd class="text-2xl font-semibold text-gray-900">{stats.notes}</dd>
          </div>
          <div>
            <dt class="text-sm text-gray-500">Written today</dt>
            <dd class="text-2xl font-semibold text-gray-900">{stats.written_today}</dd>
          </div>
        </dl>
      {/if}
    </Deferred>
  </div>

  <form class="rounded-lg bg-white p-6 shadow" onsubmit={saveName}>
    <label for="name" class="block text-sm font-medium text-gray-700">Display name</label>
    <div class="mt-1 flex gap-3">
      <input
        id="name"
        name="name"
        type="text"
        class="block w-full rounded-md border border-gray-300 px-3 py-2 shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500 sm:text-sm"
        bind:value={nameForm.name}
      />
      <button
        type="submit"
        disabled={nameForm.processing}
        class="rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-50"
      >
        {nameForm.processing ? 'Saving...' : 'Save'}
      </button>
    </div>
    {#if nameForm.errors.name}
      <p class="mt-1 text-sm text-red-600">{nameForm.errors.name}</p>
    {/if}
  </form>
</section>

<section class="mt-6 rounded-lg bg-white p-6 shadow">
  <h2 class="text-lg font-semibold text-gray-900">Recent notes</h2>
  <WhenVisible data="recent_notes">
    {#snippet fallback()}
      <p class="mt-4 text-sm text-gray-500">Loading your recent notes...</p>
    {/snippet}

    <ul class="mt-4 divide-y divide-gray-200">
      {#each recent_notes ?? [] as note (note.id)}
        <li class="flex justify-between py-2">
          <Link href={`${root}/notes/${note.id}`} prefetch class="text-indigo-600 hover:text-indigo-500">
            {note.title}
          </Link>
          <time class="text-sm text-gray-500" datetime={note.created_at}>
            {formatDate(note.created_at)}
          </time>
        </li>
      {:else}
        <li class="py-2 text-sm text-gray-500">
          No notes yet. <Link href={`${root}/notes`} class="text-indigo-600 hover:text-indigo-500">Write one</Link>.
        </li>
      {/each}
    </ul>
  </WhenVisible>
</section>
