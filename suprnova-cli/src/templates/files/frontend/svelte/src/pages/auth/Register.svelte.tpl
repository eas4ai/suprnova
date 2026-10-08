<script lang="ts">
  import { Form, usePage } from '@inertiajs/svelte'
  import Head from '../../components/Head.svelte'

  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to or links is built from it, so one build
  // runs at both.
  // `types/inertia-props.ts` types it, so `usePage()` takes no argument.
  const { root } = usePage().props

  // Inertia's `Form` component posts the form's fields and hands its
  // children `errors` and `processing`. Validation errors arrive through
  // it: a failed submission is a `303` back to this page with the errors
  // flashed, and the form takes the page's `errors` as its own. The page
  // itself takes no props - declaring an `errors` prop would replace the
  // flashed bag.
</script>

<Head title="Register" />

<h2 class="text-center text-3xl font-extrabold text-gray-900">Create your account</h2>

<Form
  action={`${root}/register`}
  method="post"
  resetOnError={['password', 'password_confirmation']}
  class="mt-8 space-y-6"
>
  {#snippet children({ errors, processing })}
    <div class="space-y-4">
      <div>
        <label for="name" class="block text-sm font-medium text-gray-700">Name</label>
        <input
          id="name"
          name="name"
          type="text"
          autocomplete="name"
          required
          class="mt-1 block w-full rounded-md border border-gray-300 px-3 py-2 shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500 sm:text-sm"
        />
        {#if errors.name}
          <p class="mt-1 text-sm text-red-600">{errors.name}</p>
        {/if}
      </div>

      <div>
        <label for="email" class="block text-sm font-medium text-gray-700">Email address</label>
        <input
          id="email"
          name="email"
          type="email"
          autocomplete="email"
          required
          class="mt-1 block w-full rounded-md border border-gray-300 px-3 py-2 shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500 sm:text-sm"
        />
        {#if errors.email}
          <p class="mt-1 text-sm text-red-600">{errors.email}</p>
        {/if}
      </div>

      <div>
        <label for="password" class="block text-sm font-medium text-gray-700">Password</label>
        <input
          id="password"
          name="password"
          type="password"
          autocomplete="new-password"
          required
          class="mt-1 block w-full rounded-md border border-gray-300 px-3 py-2 shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500 sm:text-sm"
        />
        {#if errors.password}
          <p class="mt-1 text-sm text-red-600">{errors.password}</p>
        {/if}
      </div>

      <div>
        <label for="password_confirmation" class="block text-sm font-medium text-gray-700"
          >Confirm Password</label
        >
        <input
          id="password_confirmation"
          name="password_confirmation"
          type="password"
          autocomplete="new-password"
          required
          class="mt-1 block w-full rounded-md border border-gray-300 px-3 py-2 shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500 sm:text-sm"
        />
        {#if errors.password_confirmation}
          <p class="mt-1 text-sm text-red-600">{errors.password_confirmation}</p>
        {/if}
      </div>
    </div>

    <div>
      <button
        type="submit"
        disabled={processing}
        class="flex w-full justify-center rounded-md border border-transparent bg-indigo-600 px-4 py-2 text-sm font-medium text-white shadow-sm hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2 disabled:opacity-50"
      >
        {processing ? 'Creating account...' : 'Register'}
      </button>
    </div>
  {/snippet}
</Form>
