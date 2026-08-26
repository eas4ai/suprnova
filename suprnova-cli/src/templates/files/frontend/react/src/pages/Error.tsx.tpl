import { useLang } from '../lib/lang'

// These props come from the framework, not from one of your handlers:
// `InertiaConfig::error_page` in `src/bootstrap.rs` routes every
// framework error response (403, 404, 429, 500, ...) to this page.
// That is why they are declared here rather than imported from
// `types/inertia-props.ts`, which `suprnova generate-types` rewrites
// from your `#[derive(InertiaProps)]` structs.
//
// `message` is the server's, so it arrives already localized only if
// your handlers translate it; the chrome below uses `t()` like every
// other page. That works because `src/bootstrap.rs` registers
// `LocaleMiddleware` ahead of `Inertia::install` - keep it that way, or
// this page renders in the default locale.
interface ErrorProps {
  status: number
  message: string
  request_id?: string
}

export default function ErrorPage({ status, message, request_id }: ErrorProps) {
  const { t } = useLang()

  return (
    <div className="min-h-screen flex items-center justify-center bg-gray-50 py-12 px-4 sm:px-6 lg:px-8">
      <div className="max-w-md w-full text-center space-y-4">
        <h1 className="text-6xl font-extrabold text-gray-900">{status}</h1>
        <p className="text-lg text-gray-700">{message}</p>
        {request_id && (
          <p className="text-sm text-gray-500">
            {t('error-reference')} <code className="bg-gray-100 px-1 rounded">{request_id}</code>
          </p>
        )}
        <p>
          <a href="/" className="text-indigo-600 hover:text-indigo-500">
            {t('error-go-home')}
          </a>
        </p>
      </div>
    </div>
  )
}
