<script setup lang="ts">
import { Head, Link, usePage } from '@inertiajs/vue3'

// The public root the server shares with every page (`RootShare`): empty
// at the host root, `/billing` behind a proxy that serves the app there.
// Every URL this page posts to or links is built from it, so one build
// runs at both.
// `types/inertia-props.ts` types it, so `usePage()` takes no argument.
const { root } = usePage().props

// One of the signed-in user's own notes; the server answers 404 for
// anyone else's. The notes list opens this page as an instant visit with
// the list's row as `note`, so the page renders before the server answers
// and takes the server's `note` when it arrives.
interface Note {
  id: number
  title: string
  body: string | null
  created_at: string
}

defineProps<{ note: Note }>()
</script>

<template>
  <Head :title="note.title" />

  <article class="rounded-lg bg-white p-6 shadow">
    <Link :href="`${root}/notes`" class="text-sm text-indigo-600 hover:text-indigo-500">
      Back to your notes
    </Link>
    <h2 class="mt-4 text-2xl font-semibold text-gray-900">{{ note.title }}</h2>
    <time :datetime="note.created_at" class="mt-1 block text-sm text-gray-500">{{ note.created_at }}</time>
    <p v-if="note.body" class="mt-6 whitespace-pre-line text-gray-800">{{ note.body }}</p>
  </article>
</template>
