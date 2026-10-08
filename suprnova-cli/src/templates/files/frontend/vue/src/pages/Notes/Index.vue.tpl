<script setup lang="ts">
import { unref } from 'vue'
import { Form, Head, InfiniteScroll, Link, router, usePage, useRemember } from '@inertiajs/vue3'

// The public root the server shares with every page (`RootShare`): empty
// at the host root, `/billing` behind a proxy that serves the app there.
// Every URL this page posts to or links is built from it, so one build
// runs at both.
// `types/inertia-props.ts` types it, so `usePage()` takes no argument.
const { root } = usePage().props

// The signed-in user's own notes, newest first. The handler sends `notes`
// through `Inertia::paginate` over a cursor paginator, so the list is the
// rows of one page and the next page is asked for by cursor. Those props
// are not a struct's fields, so the generated types do not cover them and
// this page declares them.
interface Note {
  id: number
  title: string
  body: string | null
  created_at: string
}

const props = defineProps<{
  notes: Note[]
  search: string
}>()

// The search box keeps what was typed in the browser's history state, so
// going back to this page restores it.
const filters = useRemember({ search: props.search }, 'Notes/Index:filters')

// A new search replaces the list rather than adding to it: `reset` tells
// the server and `InfiniteScroll` to start the `notes` list over.
function searchNotes() {
  router.get(
    `${root}/notes`,
    { search: unref(filters).search },
    { preserveState: true, replace: true, only: ['notes', 'search'], reset: ['notes'] },
  )
}
</script>

<template>
  <Head title="Notes" />

  <div class="space-y-6">
    <section class="rounded-lg bg-white p-6 shadow">
      <h2 class="text-lg font-medium text-gray-900">New note</h2>

      <!--
        A failed submission comes back with its errors and keeps this page
        as it is. A saved note reloads the page, so the list starts over
        from the newest note and the fields are empty again.
      -->
      <Form
        :action="`${root}/notes`"
        method="post"
        :options="{ preserveState: 'errors' }"
        v-slot="{ errors, processing }"
        class="mt-4 space-y-4"
      >
        <div>
          <label for="title" class="block text-sm font-medium text-gray-700">Title</label>
          <input
            id="title"
            name="title"
            type="text"
            required
            maxlength="255"
            class="mt-1 block w-full rounded-md border border-gray-300 px-3 py-2 text-sm shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500"
          />
          <p v-if="errors.title" class="mt-1 text-sm text-red-600">{{ errors.title }}</p>
        </div>

        <div>
          <label for="body" class="block text-sm font-medium text-gray-700">Body</label>
          <textarea
            id="body"
            name="body"
            rows="4"
            maxlength="10000"
            class="mt-1 block w-full rounded-md border border-gray-300 px-3 py-2 text-sm shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500"
          />
          <p v-if="errors.body" class="mt-1 text-sm text-red-600">{{ errors.body }}</p>
        </div>

        <button
          type="submit"
          :disabled="processing"
          class="rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-50"
        >
          {{ processing ? 'Saving...' : 'Save note' }}
        </button>
      </Form>
    </section>

    <section class="rounded-lg bg-white p-6 shadow">
      <label for="search" class="sr-only">Search your notes</label>
      <input
        id="search"
        v-model="filters.search"
        type="search"
        placeholder="Search your notes"
        class="block w-full rounded-md border border-gray-300 px-3 py-2 text-sm shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500"
        @change="searchNotes"
      />

      <p v-if="notes.length === 0" class="mt-4 text-sm text-gray-500">
        {{ search ? 'No note matches that search.' : 'You have not written a note yet.' }}
      </p>

      <!--
        Scrolling to the end of the list asks for the next page by its
        cursor and adds the rows to `notes`. Each row opens its note as an
        instant visit: the `Notes/Show` page renders at once from the row,
        and the server's answer fills in the rest.
      -->
      <InfiniteScroll data="notes" as="ul" class="mt-4 divide-y divide-gray-100">
        <li v-for="note in notes" :key="note.id">
          <Link
            :href="`${root}/notes/${note.id}`"
            prefetch
            component="Notes/Show"
            :page-props="{ note }"
            class="block py-3 hover:bg-gray-50"
          >
            <span class="block text-sm font-medium text-gray-900">{{ note.title }}</span>
            <time :datetime="note.created_at" class="block text-xs text-gray-500">{{ note.created_at }}</time>
          </Link>
        </li>

        <template #loading>
          <p class="py-3 text-sm text-gray-500">Loading more notes...</p>
        </template>
      </InfiniteScroll>
    </section>
  </div>
</template>
