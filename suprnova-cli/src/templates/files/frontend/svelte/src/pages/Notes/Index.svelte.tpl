<script lang="ts">
  import { Form, InfiniteScroll, Link, router, usePage, useRemember } from '@inertiajs/svelte'
  import { untrack } from 'svelte'
  import Head from '../../components/Head.svelte'
  import { formatDate } from '../../lib/dates'
  import type { NotesIndexProps } from '../../types/inertia-props'

  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to or links is built from it, so one build
  // runs at both.
  // `types/inertia-props.ts` types it, so `usePage()` takes no argument.
  const { root } = usePage().props

  // The signed-in user's notes, newest first, a page at a time from a
  // cursor paginator, and the search they were filtered by.
  let { notes, search }: NotesIndexProps = $props()

  // The search box is remembered in the browser history entry, so Back
  // returns to this page with what was typed still in it.
  const filters = useRemember({ search: untrack(() => search) }, 'Notes/Index')

  // A new search replaces the list instead of adding to it: `reset` asks the
  // server for the first page of `notes` again and tells `InfiniteScroll`
  // to start over from it. The page itself stays mounted.
  function applySearch() {
    router.get(`${root}/notes`, { search: filters.search }, {
      preserveState: true,
      replace: true,
      reset: ['notes'],
    })
  }
</script>

<Head title="Notes" />

<div class="space-y-6">
  <Form action={`${root}/notes`} method="post" class="space-y-4 rounded-lg bg-white p-6 shadow">
    {#snippet children({ errors, processing })}
      <h2 class="text-lg font-semibold text-gray-900">New note</h2>
      <div>
        <label for="title" class="block text-sm font-medium text-gray-700">Title</label>
        <input
          id="title"
          name="title"
          type="text"
          required
          maxlength="255"
          class="mt-1 block w-full rounded-md border border-gray-300 px-3 py-2 shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500 sm:text-sm"
        />
        {#if errors.title}
          <p class="mt-1 text-sm text-red-600">{errors.title}</p>
        {/if}
      </div>
      <div>
        <label for="body" class="block text-sm font-medium text-gray-700">Body</label>
        <textarea
          id="body"
          name="body"
          rows="4"
          maxlength="10000"
          class="mt-1 block w-full rounded-md border border-gray-300 px-3 py-2 shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500 sm:text-sm"
        ></textarea>
        {#if errors.body}
          <p class="mt-1 text-sm text-red-600">{errors.body}</p>
        {/if}
      </div>
      <button
        type="submit"
        disabled={processing}
        class="rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-50"
      >
        {processing ? 'Saving...' : 'Save note'}
      </button>
    {/snippet}
  </Form>

  <div class="rounded-lg bg-white p-6 shadow">
    <label for="search" class="sr-only">Search your notes</label>
    <input
      id="search"
      type="search"
      placeholder="Search your notes"
      class="block w-full rounded-md border border-gray-300 px-3 py-2 shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500 sm:text-sm"
      bind:value={filters.search}
      onchange={applySearch}
    />

    <!-- Each time the end of the list comes into view, `InfiniteScroll`
         asks the server for the next page of `notes` by its cursor and
         appends it. -->
    <InfiniteScroll data="notes" class="mt-4">
      {#snippet loading()}
        <p class="py-4 text-center text-sm text-gray-500">Loading more notes...</p>
      {/snippet}

      <ul class="divide-y divide-gray-200">
        {#each notes as note (note.id)}
          <li class="py-3">
            <!-- `prefetch` loads the note when the pointer rests on the
                 link. `component` and `pageProps` make the visit instant:
                 `Notes/Show` renders at once from this row, then from the
                 server's answer. The row is passed with the shared props
                 (`root` among them), which a plain object would leave out. -->
            <Link
              href={`${root}/notes/${note.id}`}
              prefetch
              component="Notes/Show"
              pageProps={(_props, shared) => ({ ...shared, note })}
              class="flex justify-between gap-4"
            >
              <span class="font-medium text-indigo-600 hover:text-indigo-500">{note.title}</span>
              <time class="shrink-0 text-sm text-gray-500" datetime={note.created_at}>
                {formatDate(note.created_at)}
              </time>
            </Link>
          </li>
        {:else}
          <li class="py-3 text-sm text-gray-500">
            {filters.search ? 'No notes match that search.' : 'No notes yet.'}
          </li>
        {/each}
      </ul>
    </InfiniteScroll>
  </div>
</div>
