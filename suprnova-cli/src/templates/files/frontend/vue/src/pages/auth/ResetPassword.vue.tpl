<script setup lang="ts">
import { Form, Head, Link, usePage } from '@inertiajs/vue3'
import type { ResetPasswordProps } from '../../types/inertia-props'

// The public root the server shares with every page (`RootShare`): empty
// at the host root, `/billing` behind a proxy that serves the app there.
// Every URL this page posts to or links is built from it, so one build
// runs at both.
// `types/inertia-props.ts` types it, so `usePage()` takes no argument.
const { root } = usePage().props

// The token came in on the mailed link's query string and goes back in
// the form body; the server never reads it from the URL on submit.
defineProps<ResetPasswordProps>()
</script>

<template>
  <Head title="Choose a new password" />

  <div class="flex items-center justify-center px-4 py-12 sm:px-6 lg:px-8">
    <div class="w-full max-w-md space-y-8">
      <div>
        <h2 class="mt-6 text-center text-3xl font-extrabold text-gray-900">
          Choose a new password
        </h2>
      </div>

      <Form
        :action="`${root}/reset-password`"
        method="post"
        :reset-on-error="['password', 'password_confirmation']"
        v-slot="{ errors, processing }"
        class="mt-8 space-y-6"
      >
        <input type="hidden" name="token" :value="token" />

        <p v-if="errors.token" class="text-center text-sm text-red-600">
          {{ errors.token }}
          <Link :href="`${root}/forgot-password`" class="text-indigo-600 hover:text-indigo-500">
            Request a new link
          </Link>
        </p>

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
              class="mt-1 block w-full rounded-md border border-gray-300 px-3 py-2 shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500 sm:text-sm"
            />
            <p v-if="errors.password" class="mt-1 text-sm text-red-600">
              {{ errors.password }}
            </p>
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
              class="mt-1 block w-full rounded-md border border-gray-300 px-3 py-2 shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500 sm:text-sm"
            />
            <p v-if="errors.password_confirmation" class="mt-1 text-sm text-red-600">
              {{ errors.password_confirmation }}
            </p>
          </div>
        </div>

        <div>
          <button
            type="submit"
            :disabled="processing"
            class="flex w-full justify-center rounded-md border border-transparent bg-indigo-600 px-4 py-2 text-sm font-medium text-white shadow-sm hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2 disabled:opacity-50"
          >
            {{ processing ? 'Saving...' : 'Save new password' }}
          </button>
        </div>

        <div class="text-center">
          <Link :href="`${root}/login`" class="text-indigo-600 hover:text-indigo-500">
            Back to sign in
          </Link>
        </div>
      </Form>
    </div>
  </div>
</template>
