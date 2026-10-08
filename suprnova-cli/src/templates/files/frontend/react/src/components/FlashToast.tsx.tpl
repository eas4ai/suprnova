import { usePage } from '@inertiajs/react'
import type { Toast } from '../types/inertia-props'

// The message a handler flashed with `Inertia::flash("toast", Toast { .. })`
// after a sign-in, a registration, a reset or a saved note. The server sends
// it in the next page's `flash` and removes it from the session there, so
// the toast is read from the page on every render and never copied into
// state: the page after it carries no toast, and this renders nothing.
export default function FlashToast() {
  const toast = flashedToast(usePage().flash)

  if (!toast) {
    return null
  }

  const tone =
    toast.kind === 'error'
      ? 'border-red-200 bg-red-50 text-red-800'
      : toast.kind === 'info'
        ? 'border-blue-200 bg-blue-50 text-blue-800'
        : 'border-green-200 bg-green-50 text-green-800'

  return (
    <div className="mx-auto mt-4 max-w-7xl px-4 sm:px-6 lg:px-8">
      <p
        role={toast.kind === 'error' ? 'alert' : 'status'}
        className={`rounded-md border px-4 py-3 text-sm ${tone}`}
      >
        {toast.message}
      </p>
    </div>
  )
}

/**
 * The toast in a page's flash data, or `null` when the page carries none.
 *
 * `page.flash` holds each flashed value under the key it was flashed with,
 * so the toast is `flash.toast`. It is read through `object` because the
 * generated `flashDataType` names the `Toast` struct itself rather than the
 * map that holds it under `toast`.
 */
function flashedToast(flash: object): Toast | null {
  if (!('toast' in flash)) {
    return null
  }

  const toast = flash.toast

  if (
    typeof toast !== 'object' ||
    toast === null ||
    !('message' in toast) ||
    typeof toast.message !== 'string'
  ) {
    return null
  }

  return toast as Toast
}
