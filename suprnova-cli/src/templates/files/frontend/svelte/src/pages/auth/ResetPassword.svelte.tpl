<script lang="ts">
  import { untrack } from 'svelte'
  import { useForm } from '@inertiajs/svelte'
  import type { ResetPasswordProps } from '../../types/inertia-props'

  let { token }: ResetPasswordProps = $props()

  // The token came in on the mailed link's query string and goes back in
  // the form body; the server never reads it from the URL on submit. It is
  // read once, on purpose: the link's token does not change while this
  // page is shown, and `untrack` says so where Svelte would otherwise warn
  // about capturing a prop's initial value.
  const form = useForm({
    token: untrack(() => token),
    password: '',
    password_confirmation: '',
  })

  function submit(e: SubmitEvent) {
    e.preventDefault()
    form.post('/reset-password', {
      onFinish: () => form.reset('password', 'password_confirmation'),
    })
  }
</script>

<div
  class="min-h-screen flex items-center justify-center bg-gray-50 py-12 px-4 sm:px-6 lg:px-8"
>
  <div class="max-w-md w-full space-y-8">
    <div>
      <h2 class="mt-6 text-center text-3xl font-extrabold text-gray-900">
        Choose a new password
      </h2>
    </div>

    {#if form.errors.token}
      <p class="text-center text-sm text-red-600">
        {form.errors.token}
        <a href="/forgot-password" class="text-indigo-600 hover:text-indigo-500">
          Request a new link
        </a>
      </p>
    {/if}

    <form class="mt-8 space-y-6" onsubmit={submit}>
      <div class="space-y-4">
        <div>
          <label for="password" class="block text-sm font-medium text-gray-700"
            >New password</label
          >
          <input
            id="password"
            name="password"
            type="password"
            autocomplete="new-password"
            required
            class="mt-1 block w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm"
            bind:value={form.password}
          />
          {#if form.errors.password}
            <p class="mt-1 text-sm text-red-600">{form.errors.password}</p>
          {/if}
        </div>

        <div>
          <label for="password_confirmation" class="block text-sm font-medium text-gray-700"
            >Confirm new password</label
          >
          <input
            id="password_confirmation"
            name="password_confirmation"
            type="password"
            autocomplete="new-password"
            required
            class="mt-1 block w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm"
            bind:value={form.password_confirmation}
          />
          {#if form.errors.password_confirmation}
            <p class="mt-1 text-sm text-red-600">{form.errors.password_confirmation}</p>
          {/if}
        </div>
      </div>

      <div>
        <button
          type="submit"
          disabled={form.processing}
          class="w-full flex justify-center py-2 px-4 border border-transparent rounded-md shadow-sm text-sm font-medium text-white bg-indigo-600 hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 disabled:opacity-50"
        >
          {form.processing ? 'Saving...' : 'Save new password'}
        </button>
      </div>

      <div class="text-center">
        <a href="/login" class="text-indigo-600 hover:text-indigo-500">
          Back to sign in
        </a>
      </div>
    </form>
  </div>
</div>
