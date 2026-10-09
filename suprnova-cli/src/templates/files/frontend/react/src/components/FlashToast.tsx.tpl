import { usePage } from '@inertiajs/react'

// The message a handler flashed with `Inertia::flash("toast", Toast { .. })`
// after a sign-in, a registration, a reset or a saved note. `page.flash` is
// typed by the generated `flashDataType`, `Flash`, whose `toast` it is. The
// server sends it in the next page's `flash` and removes it from the
// session there, so the toast is read from the page on every render and
// never copied into state: the page after it carries no toast, and this
// renders nothing.
export default function FlashToast() {
  const { toast } = usePage().flash

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
