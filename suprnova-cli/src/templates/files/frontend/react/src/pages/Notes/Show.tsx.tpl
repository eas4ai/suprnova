import { Head, Link, usePage } from '@inertiajs/react'
import type { NotesShowProps } from '../../types/inertia-props'

export default function NotesShow({ note }: NotesShowProps) {
  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to or links is built from it, so one build
  // runs at both.
  // `types/inertia-props.ts` types it, so `usePage()` takes no argument.
  const { root } = usePage().props

  return (
    <article className="space-y-4 rounded-lg bg-white p-6 shadow">
      <Head title={note.title} />
      <header>
        <h2 className="text-xl font-semibold text-gray-900">{note.title}</h2>
        <time dateTime={note.created_at} className="text-sm text-gray-500">
          {note.created_at.slice(0, 10)}
        </time>
      </header>
      {/* A visit from the notes list renders this page first from the list's
          row, which carries no `body`, and then from the server's answer:
          until then `body` is undefined and nothing shows here. */}
      {typeof note.body === 'string' && note.body !== '' ? (
        <p className="whitespace-pre-line text-gray-800">{note.body}</p>
      ) : note.body === null || note.body === '' ? (
        <p className="text-sm text-gray-500">This note has a title and nothing else.</p>
      ) : null}
      <Link href={`${root}/notes`} className="text-sm text-indigo-600 hover:text-indigo-500">
        Back to your notes
      </Link>
    </article>
  )
}
