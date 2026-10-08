<script setup lang="ts">
import { Form, Head, Link, usePage } from '@inertiajs/vue3'
import type { FormDataConvertible } from '@inertiajs/core'

// The public root the server shares with every page (`RootShare`): empty
// at the host root, `/billing` behind a proxy that serves the app there.
// Every URL this page posts to or links is built from it, so one build
// runs at both.
// `types/inertia-props.ts` types it, so `usePage()` takes no argument.
const { root } = usePage().props

// Validation errors arrive through the form: a failed submission is a
// `303` back to this page with the errors flashed, and the `Form`
// component hands the page's `errors` to its slot. The page itself takes
// no props - declaring an `errors` prop would replace the flashed bag.

// The form sends its fields as JSON. A checked checkbox is the string
// `"on"` in the form's data and an unchecked one is absent, while the
// server reads `remember` as a boolean, so the field is sent as one.
function withRememberFlag(data: Record<string, FormDataConvertible>) {
  return { ...data, remember: 'remember' in data }
}
</script>

<template>
  <Head title="Sign in" />

  <div class="flex items-center justify-center px-4 py-12 sm:px-6 lg:px-8">
    <div class="w-full max-w-md space-y-8">
      <div>
        <h2 class="mt-6 text-center text-3xl font-extrabold text-gray-900">
          Sign in to your account
        </h2>
      </div>
      <Form
        :action="`${root}/login`"
        method="post"
        :transform="withRememberFlag"
        :reset-on-error="['password']"
        v-slot="{ errors, processing }"
        class="mt-8 space-y-6"
      >
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

        <div v-if="errors.email" class="text-sm text-red-600">
          {{ errors.email }}
        </div>

        <div v-if="errors.password" class="text-sm text-red-600">
          {{ errors.password }}
        </div>

        <div class="flex items-center">
          <input
            id="remember"
            name="remember"
            type="checkbox"
            class="h-4 w-4 rounded border-gray-300 text-indigo-600 focus:ring-indigo-500"
          />
          <label for="remember" class="ml-2 block text-sm text-gray-900">
            Remember me
          </label>
          <Link
            :href="`${root}/forgot-password`"
            class="ml-auto text-sm text-indigo-600 hover:text-indigo-500"
          >
            Forgot your password?
          </Link>
        </div>

        <div>
          <button
            type="submit"
            :disabled="processing"
            class="group relative flex w-full justify-center rounded-md border border-transparent bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2 disabled:opacity-50"
          >
            {{ processing ? 'Signing in...' : 'Sign in' }}
          </button>
        </div>

        <div class="text-center">
          <Link :href="`${root}/register`" class="text-indigo-600 hover:text-indigo-500">
            Don't have an account? Register
          </Link>
        </div>
      </Form>
    </div>
  </div>
</template>
