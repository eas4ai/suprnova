<script setup lang="ts">
import { Link, usePage } from '@inertiajs/vue3'
import FlashToast from '../components/FlashToast.vue'

// The public root the server shares with every page (`RootShare`): empty
// at the host root, `/billing` behind a proxy that serves the app there.
// Every URL this layout links is built from it, so one build runs at both.
// `types/inertia-props.ts` types it, so `usePage()` takes no argument.
const { root } = usePage().props

// The guest layout wraps the pages under `auth/` (see the `layout` option
// in `main.ts`): the sign-in and registration links, and the toast a
// sign-out or a reset link request leaves.
</script>

<template>
  <div class="min-h-screen bg-gray-50">
    <nav class="mx-auto flex max-w-7xl justify-end gap-4 px-4 py-4 sm:px-6 lg:px-8">
      <Link :href="`${root}/login`" class="text-sm font-medium text-indigo-600 hover:text-indigo-500">
        Sign in
      </Link>
      <Link :href="`${root}/register`" class="text-sm font-medium text-indigo-600 hover:text-indigo-500">
        Register
      </Link>
    </nav>

    <FlashToast />

    <slot />
  </div>
</template>
