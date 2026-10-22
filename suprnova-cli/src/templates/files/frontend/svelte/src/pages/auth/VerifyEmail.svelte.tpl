<script lang="ts">
  import { Form, Link, usePage } from '@inertiajs/svelte'
  import Head from '../../components/Head.svelte'
  import type { VerifyEmailProps } from '../../types/inertia-props'

  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to or links is built from it, so one build
  // runs at both.
  // `types/inertia-props.ts` types it, so `usePage()` takes no argument.
  const { root } = usePage().props

  // A resend redirects back here with a toast saying a new link is on its
  // way; the layout shows it.
  let { email }: VerifyEmailProps = $props()
</script>

<Head title="Verify your email address" />

<div>
  <h2 class="text-center text-3xl font-extrabold text-gray-900">Verify your email address</h2>
  <p class="mt-2 text-center text-sm text-gray-600">
    We sent a verification link to <strong>{email}</strong>. Open it while signed in to this
    account.
  </p>
</div>

<Form action={`${root}/email/verification-notification`} method="post" class="mt-8 space-y-6">
  {#snippet children({ errors, processing })}
    {#each Object.entries(errors) as [field, message] (field)}
      <p class="text-center text-sm text-red-600">{message}</p>
    {/each}

    <div>
      <button
        type="submit"
        disabled={processing}
        class="group relative flex w-full justify-center rounded-md border border-transparent bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2 disabled:opacity-50"
      >
        {processing ? 'Sending...' : 'Resend verification link'}
      </button>
    </div>

    <div class="text-center">
      <Link href={`${root}/dashboard`} class="text-indigo-600 hover:text-indigo-500">
        Continue to your dashboard
      </Link>
    </div>
  {/snippet}
</Form>
