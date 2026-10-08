<script lang="ts">
  import { Form, Link, usePage } from '@inertiajs/svelte'
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
  //
  // A checked checkbox submits the text `on`, and the JSON body the form
  // sends needs `remember` as a boolean, so `transform` turns it into one.
</script>

<Head title="Sign in" />

<h2 class="text-center text-3xl font-extrabold text-gray-900">Sign in to your account</h2>

<Form
  action={`${root}/login`}
  method="post"
  transform={(data) => ({ ...data, remember: data.remember === 'on' })}
  resetOnError={['password']}
  class="mt-8 space-y-6"
>
  {#snippet children({ errors, processing })}
    <div class="-space-y-px rounded-md shadow-sm">
      <div>
        <label for="email" class="sr-only">Email address</label>
        <input
          id="email"
          name="email"
          type="email"
          autocomplete="email"
          required
          class="relative block w-full appearance-none rounded-none rounded-t-md border border-gray-300 px-3 py-2 text-gray-900 placeholder-gray-500 focus:z-10 focus:border-indigo-500 focus:outline-none focus:ring-indigo-500 sm:text-sm"
          placeholder="Email address"
        />
      </div>
      <div>
        <label for="password" class="sr-only">Password</label>
        <input
          id="password"
          name="password"
          type="password"
          autocomplete="current-password"
          required
          class="relative block w-full appearance-none rounded-none rounded-b-md border border-gray-300 px-3 py-2 text-gray-900 placeholder-gray-500 focus:z-10 focus:border-indigo-500 focus:outline-none focus:ring-indigo-500 sm:text-sm"
          placeholder="Password"
        />
      </div>
    </div>

    {#if errors.email}
      <div class="text-sm text-red-600">{errors.email}</div>
    {/if}

    {#if errors.password}
      <div class="text-sm text-red-600">{errors.password}</div>
    {/if}

    <div class="flex items-center">
      <input
        id="remember"
        name="remember"
        type="checkbox"
        class="h-4 w-4 rounded border-gray-300 text-indigo-600 focus:ring-indigo-500"
      />
      <label for="remember" class="ml-2 block text-sm text-gray-900">Remember me</label>
      <Link
        href={`${root}/forgot-password`}
        class="ml-auto text-sm text-indigo-600 hover:text-indigo-500"
      >
        Forgot your password?
      </Link>
    </div>

    <div>
      <button
        type="submit"
        disabled={processing}
        class="group relative flex w-full justify-center rounded-md border border-transparent bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2 disabled:opacity-50"
      >
        {processing ? 'Signing in...' : 'Sign in'}
      </button>
    </div>
  {/snippet}
</Form>
