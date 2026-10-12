import { Link, usePage } from '@inertiajs/react'

// The account corner of both layouts, from the shared `auth` prop: the
// signed-in user's name and the sign-out link, or the sign-in and register
// links for a guest.
export default function AccountLinks({ className }: { className: string }) {
  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this component links is built from it, so one build runs at
  // both. `auth` is shared on every page; the `?.` covers an error page the
  // framework renders without the shared props.
  const { root, auth } = usePage().props
  const user = auth?.user ?? null

  return user ? (
    <>
      <span className="text-sm text-gray-700">{user.name}</span>
      {/* A `Link` that posts keeps the page's state by default, and with
          it the layout props; this visit starts the next page clean, so
          the dashboard's heading does not follow it. */}
      <Link href={`${root}/logout`} method="post" as="button" preserveState={false} className={className}>
        Sign out
      </Link>
    </>
  ) : (
    <>
      <Link href={`${root}/login`} className={className}>
        Sign in
      </Link>
      <Link href={`${root}/register`} className={className}>
        Register
      </Link>
    </>
  )
}
