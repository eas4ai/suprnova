import { Form, Head, InfiniteScroll, Link, router, usePage, useRemember } from '@inertiajs/react'
import type { Pages } from '../../types/inertia-props'

// A row of the list: the note as `Notes/Show` receives it, so a row can open
// its note through an instant visit.
type Note = Pages['Notes/Show']['note']

// The handler renders this page with `Inertia::paginate("Notes/Index",
// "notes", paginator)`: `notes` is one page of the signed-in user's notes,
// newest first, with the cursor of the next page in the page's
// `scrollProps`, and `search` the filter it applied. A paginator is not a
// props struct, so `suprnova generate-types` cannot type it and this page
// declares it.
interface Props {
  notes: Note[]
  search: string
}

export default function NotesIndex({ notes, search }: Props) {
  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to or links is built from it, so one build
  // runs at both.
  // `types/inertia-props.ts` types it, so `usePage()` takes no argument.
  const { root } = usePage().props

  // The search box's text, kept in the browser history's state for this
  // visit: back from a note, it holds what was typed before.
  const [query, setQuery] = useRemember(search, 'Notes/Index:search')

  const searchFor = (value: string) => {
    setQuery(value)
    // `preserveState` keeps this page mounted, and with it the focus in the
    // search box. `reset` makes the server send `notes` as a fresh first
    // page instead of rows to append, and tells the infinite scroll to
    // start over from that page's cursor.
    router.get(`${root}/notes`, { search: value }, {
      preserveState: true,
      replace: true,
      only: ['notes', 'search'],
      reset: ['notes'],
    })
  }

  return (
    <div className="space-y-6">
      <Head title="Notes" />

      {/* A saved note redirects back to this page with the new note first.
          `preserveState: 'errors'` keeps the typed text when validation
          fails and starts the page over when the note is saved. */}
      <Form
        action={`${root}/notes`}
        method="post"
        options={{ preserveState: 'errors' }}
        className="space-y-3 rounded-lg bg-white p-6 shadow"
      >
        {({ errors, processing }) => (
          <>
            <div>
              <label htmlFor="title" className="block text-sm font-medium text-gray-700">
                Title
              </label>
              <input
                id="title"
                name="title"
                type="text"
                required
                maxLength={255}
                className="mt-1 block w-full rounded-md border border-gray-300 px-3 py-2 text-sm shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500"
              />
              {errors.title && <p className="mt-1 text-sm text-red-600">{errors.title}</p>}
            </div>
            <div>
              <label htmlFor="body" className="block text-sm font-medium text-gray-700">
                Note
              </label>
              <textarea
                id="body"
                name="body"
                rows={4}
                className="mt-1 block w-full rounded-md border border-gray-300 px-3 py-2 text-sm shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500"
              />
              {errors.body && <p className="mt-1 text-sm text-red-600">{errors.body}</p>}
            </div>
            <button
              type="submit"
              disabled={processing}
              className="rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-50"
            >
              {processing ? 'Saving...' : 'Save note'}
            </button>
          </>
        )}
      </Form>

      <section className="rounded-lg bg-white p-6 shadow">
        <label htmlFor="search" className="sr-only">
          Search your notes
        </label>
        <input
          id="search"
          type="search"
          value={query}
          onChange={(event) => searchFor(event.target.value)}
          placeholder="Search your notes"
          className="block w-full rounded-md border border-gray-300 px-3 py-2 text-sm shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500"
        />

        {notes.length === 0 ? (
          <p className="mt-4 text-sm text-gray-500">
            {search ? 'No note matches your search.' : 'No notes yet. Write your first one above.'}
          </p>
        ) : (
          // Loads the next page of rows, by cursor, as the end of the list
          // scrolls into view, and appends it.
          <InfiniteScroll
            data="notes"
            as="ul"
            className="mt-4 divide-y divide-gray-100"
            loading={<p className="py-2 text-sm text-gray-500">Loading more notes...</p>}
          >
            {notes.map((note) => (
              <li key={note.id} className="py-3">
                {/* `component` and `pageProps` make the visit instant: the
                    show page renders from this row before the server answers,
                    then takes the server's props. `shared` carries the shared
                    props, `root` among them, into that first render.
                    `prefetch` loads the page on hover. */}
                <Link
                  href={`${root}/notes/${note.id}`}
                  prefetch
                  component="Notes/Show"
                  pageProps={(_props, shared) => ({ ...shared, note })}
                  className="block hover:bg-gray-50"
                >
                  <span className="font-medium text-gray-900">{note.title}</span>
                  <time dateTime={note.created_at} className="ml-2 text-sm text-gray-500">
                    {note.created_at.slice(0, 10)}
                  </time>
                </Link>
              </li>
            ))}
          </InfiniteScroll>
        )}
      </section>
    </div>
  )
}
