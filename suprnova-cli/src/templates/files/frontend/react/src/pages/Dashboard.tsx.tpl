import { useState, type FormEvent } from 'react'
import {
  Deferred,
  Head,
  Link,
  router,
  setLayoutProps,
  useHttp,
  usePage,
  usePoll,
  WhenVisible,
} from '@inertiajs/react'
import type { DashboardProps, UserInfo } from '../types/inertia-props'

// The handler resolves both props apart from the page: `stats` deferred
// (`.defer("stats", ..)`), fetched in a request of its own once the page
// has rendered, and `recent_notes` optional (`.optional("recent_notes",
// ..)`), sent only when a partial reload names it. Both are absent until
// loaded, so the page takes them as optional.
export default function Dashboard({ stats, recent_notes }: Partial<DashboardProps>) {
  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to or links is built from it, so one build
  // runs at both.
  // `types/inertia-props.ts` types it, so `usePage()` takes no argument.
  const { root } = usePage().props
  // The signed-in user, shared with every page. The dashboard is behind
  // the `auth` middleware, so it is set here; the type also covers a guest
  // page, hence the fallbacks below.
  const user = usePage().props.auth.user

  // The application layout shows this heading. A visit to another page
  // clears it, so a page that sets none shows none.
  setLayoutProps({ heading: 'Dashboard' })

  // Reload `stats` every ten seconds. Each tick is a partial reload: the
  // server resolves `stats` alone and the rest of the page stays as it is.
  usePoll(10000, { only: ['stats'] })

  // The display-name form talks JSON to `POST /profile/name` without a page
  // visit. `profile.data.name` is the name the page shows, and `draft` what
  // the visitor is typing. Submitting copies the draft into the shown name
  // at once (`optimistic`); a `422` puts the previous name back and fills
  // `profile.errors.name` from the response's `errors`.
  const [draft, setDraft] = useState(user?.name ?? '')
  const profile = useHttp<{ name: string }, { user: UserInfo }>({ name: user?.name ?? '' })

  const saveName = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    profile
      .optimistic(() => ({ name: draft }))
      .post(`${root}/profile/name`, {
        // The layouts show the signed-in user from the shared `auth`
        // prop; reload just that prop so they show the saved name too.
        onSuccess: () => router.reload({ only: ['auth'] }),
      })
      .catch(() => {
        // Anything but a 2xx or a 422 (a network failure, a 500) rejects
        // after the shown name is put back.
        profile.setError('name', 'Your name could not be saved. Try again.')
      })
  }

  return (
    <div className="space-y-6">
      <Head title="Dashboard" />

      <section className="rounded-lg bg-white p-6 shadow">
        <h2 className="text-xl font-semibold text-gray-900">Welcome, {profile.data.name}!</h2>
        <p className="mt-1 text-sm text-gray-500">Signed in as {user?.email}</p>

        <form className="mt-4 flex flex-wrap items-start gap-3" onSubmit={saveName}>
          <div>
            <label htmlFor="name" className="sr-only">
              Display name
            </label>
            <input
              id="name"
              name="name"
              type="text"
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              className="block w-64 rounded-md border border-gray-300 px-3 py-2 text-sm shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500"
            />
            {profile.errors.name && (
              <p className="mt-1 text-sm text-red-600">{profile.errors.name}</p>
            )}
          </div>
          <button
            type="submit"
            disabled={profile.processing}
            className="rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-50"
          >
            {profile.processing ? 'Saving...' : 'Change name'}
          </button>
        </form>
      </section>

      <section className="rounded-lg bg-white p-6 shadow">
        <h2 className="text-lg font-semibold text-gray-900">Your notes</h2>
        <Deferred data="stats" fallback={<p className="mt-2 text-sm text-gray-500">Counting your notes...</p>}>
          {stats && (
            <dl className="mt-4 grid grid-cols-2 gap-4">
              <div>
                <dt className="text-sm text-gray-500">Notes</dt>
                <dd className="text-2xl font-semibold text-gray-900">{stats.notes}</dd>
              </div>
              <div>
                <dt className="text-sm text-gray-500">Written today</dt>
                <dd className="text-2xl font-semibold text-gray-900">{stats.written_today}</dd>
              </div>
            </dl>
          )}
        </Deferred>
      </section>

      <section className="rounded-lg bg-white p-6 shadow">
        <h2 className="text-lg font-semibold text-gray-900">Recent notes</h2>
        {/* Requested when this section scrolls into view, not before. */}
        <WhenVisible data="recent_notes" fallback={<p className="mt-2 text-sm text-gray-500">Loading your recent notes...</p>}>
          {recent_notes && recent_notes.length > 0 ? (
            <ul className="mt-2 divide-y divide-gray-100">
              {recent_notes.map((note) => (
                <li key={note.id} className="py-2">
                  <Link href={`${root}/notes/${note.id}`} className="text-indigo-600 hover:text-indigo-500">
                    {note.title}
                  </Link>
                </li>
              ))}
            </ul>
          ) : (
            <p className="mt-2 text-sm text-gray-500">
              No notes yet.{' '}
              <Link href={`${root}/notes`} className="text-indigo-600 hover:text-indigo-500">
                Write your first one
              </Link>
              .
            </p>
          )}
        </WhenVisible>
      </section>
    </div>
  )
}
