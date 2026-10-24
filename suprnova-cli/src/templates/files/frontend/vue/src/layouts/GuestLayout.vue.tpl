<script setup lang="ts">
import AccountLinks from '../components/AccountLinks.vue'
import FlashToast from '../components/FlashToast.vue'

// The guest layout wraps the pages under `auth/` (see the `layout` option
// in `main.ts`): the account links, which are the sign-in and register
// links for a guest and the name and the sign-out for someone signed in
// (on the email verification page), and the toast a sign-out or a reset
// link request leaves.
</script>

<template>
  <div class="min-h-screen bg-gray-50">
    <nav class="mx-auto flex max-w-7xl items-center justify-end gap-4 px-4 py-4 sm:px-6 lg:px-8">
      <AccountLinks />
    </nav>

    <FlashToast />

    <slot />
  </div>
</template>
