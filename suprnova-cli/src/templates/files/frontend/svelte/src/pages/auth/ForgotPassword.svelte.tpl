<script lang="ts">
  import { Form, usePage } from '@inertiajs/svelte'
  import Head from '../../components/Head.svelte'

  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to or links is built from it, so one build
  // runs at both.
  // `types/inertia-props.ts` types it, so `usePage()` takes no argument.
  const { root } = usePage().props

  // The server answers every address the same way and flashes a toast that
  // says a link is on its way if the address is registered, so the page
  // never learns, and never shows, whether the account exists.
</script>

<Head title="Reset your password" />

<div>
  <h2 class="text-center text-3xl font-extrabold text-gray-900">Reset your password</h2>
  <p class="mt-2 text-center text-sm text-gray-600">
    Enter the email address you verified for your account and we will send you a link to
    choose a new password.
  </p>
</div>

<Form action={`${root}/forgot-password`} method="post" class="mt-8 space-y-6">
  {#snippet children({ errors, processing })}
    <div>
      <label for="email" class="sr-only">Email address</label>
      <input
        id="email"
        name="email"
        type="email"
        autocomplete="email"
        required
        class="relative block w-full appearance-none rounded-md border border-gray-300 px-3 py-2 text-gray-900 placeholder-gray-500 focus:z-10 focus:border-indigo-500 focus:outline-none focus:ring-indigo-500 sm:text-sm"
        placeholder="Email address"
      />
      {#if errors.email}
        <p class="mt-1 text-sm text-red-600">{errors.email}</p>
      {/if}
    </div>

    <div>
      <button
        type="submit"
        disabled={processing}
        class="group relative flex w-full justify-center rounded-md border border-transparent bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2 disabled:opacity-50"
      >
        {processing ? 'Sending...' : 'Send reset link'}
      </button>
    </div>
  {/snippet}
</Form>
