import type { ReactNode } from 'react'
import { Link, usePage } from '@inertiajs/react'
import AccountLinks from '../components/AccountLinks'
import FlashToast from '../components/FlashToast'

// The layout of every page outside `auth/`, applied by `layout` in
// `lib/app.ts`. It stays mounted while the visitor moves between those
// pages; only the page inside it changes.
//
// Inertia renders a layout with the page's props, the props a page sets
// with `setLayoutProps`, and the page as `children`.
interface AppLayoutProps {
  children?: ReactNode
  /** A heading a page sets with `setLayoutProps({ heading })`. The next
   *  page that sets none shows none: a visit clears the layout props. */
  heading?: string
}

const linkClass = 'rounded-md px-3 py-2 text-sm font-medium'
const idleClass = `${linkClass} text-gray-600 hover:text-gray-900`
const activeClass = `${linkClass} bg-gray-100 text-gray-900`

export default function AppLayout({ children, heading }: AppLayoutProps) {
  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this layout links is built from it, so one build runs at both.
  const { component, props } = usePage()
  const { root } = props

  return (
    <div className="min-h-screen bg-gray-100">
      <nav className="bg-white shadow">
        <div className="mx-auto flex h-16 max-w-7xl items-center justify-between px-4 sm:px-6 lg:px-8">
          <div className="flex items-center gap-2">
            <Link
              href={`${root}/dashboard`}
              className={component === 'Dashboard' ? activeClass : idleClass}
              aria-current={component === 'Dashboard' ? 'page' : undefined}
            >
              Dashboard
            </Link>
            <Link
              href={`${root}/notes`}
              className={component.startsWith('Notes/') ? activeClass : idleClass}
              aria-current={component.startsWith('Notes/') ? 'page' : undefined}
            >
              Notes
            </Link>
          </div>
          <div className="flex items-center gap-4">
            <AccountLinks className={idleClass} />
          </div>
        </div>
      </nav>

      <FlashToast />

      {heading && (
        <header className="mx-auto max-w-7xl px-4 pt-6 sm:px-6 lg:px-8">
          <h1 className="text-2xl font-bold text-gray-900">{heading}</h1>
        </header>
      )}

      <main className="mx-auto max-w-7xl px-4 py-6 sm:px-6 lg:px-8">{children}</main>
    </div>
  )
}
