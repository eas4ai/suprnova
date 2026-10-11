<script lang="ts">
  import { Link, usePage } from '@inertiajs/svelte'
  import Head from '../components/Head.svelte'
  import { t } from '../lib/lang.svelte'

  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to or links is built from it, so one build
  // runs at both.
  // `types/inertia-props.ts` types it, so `usePage()` takes no argument.
  const { root } = usePage().props

  // These props come from the framework, not from one of your handlers:
  // `InertiaConfig::error_page` in `src/bootstrap.rs` routes every
  // framework error response (403, 404, 429, 500, ...) to this page.
  // That is why they are declared here: no handler of yours renders
  // `Error` with a struct, so `suprnova generate-types` writes no
  // `Pages` entry for it in `types/inertia-props.ts`.
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

  let { status, message, request_id }: ErrorProps = $props()
</script>

<Head title={String(status)} />

<div class="mx-auto max-w-md space-y-4 py-12 text-center">
  <h1 class="text-6xl font-extrabold text-gray-900">{status}</h1>
  <p class="text-lg text-gray-700">{message}</p>
  {#if request_id}
    <p class="text-sm text-gray-500">
      {t('error-reference')}
      <code class="rounded bg-gray-100 px-1">{request_id}</code>
    </p>
  {/if}
  <p>
    <Link href={`${root}/`} class="text-indigo-600 hover:text-indigo-500">{t('error-go-home')}</Link>
  </p>
</div>
