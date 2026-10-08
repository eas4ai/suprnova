import type { ReactNode } from 'react'
import { Link, usePage } from '@inertiajs/react'
import FlashToast from '../components/FlashToast'

// The layout of the pages under `auth/`, applied by `layout` in
// `lib/app.ts`: sign in, register, the password reset and the email
// verification notice. It stays mounted while the visitor moves between
// them.
export default function GuestLayout({ children }: { children?: ReactNode }) {
  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this layout links is built from it, so one build runs at both.
  const { root } = usePage().props

  return (
    <div className="min-h-screen bg-gray-50">
      <nav className="mx-auto flex max-w-md items-center justify-end gap-4 px-4 pt-6 text-sm">
        <Link href={`${root}/login`} className="text-indigo-600 hover:text-indigo-500">
          Sign in
        </Link>
        <Link href={`${root}/register`} className="text-indigo-600 hover:text-indigo-500">
          Register
        </Link>
      </nav>

      <FlashToast />

      <main className="flex items-center justify-center px-4 py-12 sm:px-6 lg:px-8">
        <div className="w-full max-w-md space-y-8">{children}</div>
      </main>
    </div>
  )
}
