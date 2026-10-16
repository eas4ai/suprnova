import type { ReactNode } from 'react'
import AccountLinks from '../components/AccountLinks'
import FlashToast from '../components/FlashToast'

// The layout of the pages under `auth/`, applied by `layout` in
// `lib/app.ts`: sign in, register, the password reset and the email
// verification notice. It stays mounted while the visitor moves between
// them. The verification notice is for a signed-in user, so the account
// links follow the shared `auth` prop here too.
export default function GuestLayout({ children }: { children?: ReactNode }) {
  return (
    <div className="min-h-screen bg-gray-50">
      <nav className="mx-auto flex max-w-md items-center justify-end gap-4 px-4 pt-6 text-sm">
        <AccountLinks className="text-indigo-600 hover:text-indigo-500" />
      </nav>

      <FlashToast />

      <main className="flex items-center justify-center px-4 py-12 sm:px-6 lg:px-8">
        <div className="w-full max-w-md space-y-8">{children}</div>
      </main>
    </div>
  )
}
