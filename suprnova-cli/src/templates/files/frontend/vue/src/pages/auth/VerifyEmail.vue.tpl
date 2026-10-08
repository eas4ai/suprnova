<script setup lang="ts">
import { Form, Head, Link, usePage } from '@inertiajs/vue3'
import type { VerifyEmailProps } from '../../types/inertia-props'

// The public root the server shares with every page (`RootShare`): empty
// at the host root, `/billing` behind a proxy that serves the app there.
// Every URL this page posts to or links is built from it, so one build
// runs at both.
// `types/inertia-props.ts` types it, so `usePage()` takes no argument.
const { root } = usePage().props

// The resend answers with a toast the server flashes, so this page keeps
// no "sent" state of its own.
defineProps<VerifyEmailProps>()
</script>

<template>
  <Head title="Verify your email address" />

  <div class="flex items-center justify-center px-4 py-12 sm:px-6 lg:px-8">
    <div class="w-full max-w-md space-y-8">
      <div>
        <h2 class="mt-6 text-center text-3xl font-extrabold text-gray-900">
          Verify your email address
        </h2>
        <p class="mt-2 text-center text-sm text-gray-600">
          We sent a verification link to <strong>{{ email }}</strong>. Open it while signed in
          to this account.
        </p>
      </div>

      <Form
        :action="`${root}/email/verification-notification`"
        method="post"
        v-slot="{ processing }"
        class="mt-8 space-y-6"
      >
        <div>
          <button
            type="submit"
            :disabled="processing"
            class="group relative flex w-full justify-center rounded-md border border-transparent bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2 disabled:opacity-50"
          >
            {{ processing ? 'Sending...' : 'Resend verification link' }}
          </button>
        </div>
      </Form>

      <div class="text-center text-sm">
        <Link :href="`${root}/dashboard`" class="text-indigo-600 hover:text-indigo-500">
          Continue to your dashboard
        </Link>
      </div>
    </div>
  </div>
</template>
