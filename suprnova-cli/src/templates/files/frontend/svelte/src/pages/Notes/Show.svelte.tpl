<script lang="ts">
  import { Link, usePage } from '@inertiajs/svelte'
  import Head from '../../components/Head.svelte'
  import { formatDate } from '../../lib/dates'
  import type { NotesShowProps } from '../../types/inertia-props'

  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to or links is built from it, so one build
  // runs at both.
  // `types/inertia-props.ts` types it, so `usePage()` takes no argument.
  const { root } = usePage().props

  // One of the signed-in user's notes; the server answers 404 for anyone
  // else's. Opened from the notes list, the page first renders from the
  // list's row, which has no body, then from the server's answer.
  let { note }: NotesShowProps = $props()
</script>

<Head title={note.title} />

<article class="rounded-lg bg-white p-6 shadow">
  <h2 class="text-xl font-bold text-gray-900">{note.title}</h2>
  <time class="mt-1 block text-sm text-gray-500" datetime={note.created_at}>
    {formatDate(note.created_at)}
  </time>
  {#if note.body}
    <p class="mt-4 whitespace-pre-line text-gray-800">{note.body}</p>
  {/if}
</article>

<p class="mt-6">
  <Link href={`${root}/notes`} class="text-indigo-600 hover:text-indigo-500">Back to your notes</Link>
</p>
