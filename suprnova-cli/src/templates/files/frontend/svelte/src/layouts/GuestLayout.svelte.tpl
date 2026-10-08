<script lang="ts">
  import type { Snippet } from 'svelte'
  import AccountLinks from '../components/AccountLinks.svelte'
  import FlashToast from '../components/FlashToast.svelte'

  // The frame of every page under `auth/`, applied by `createInertiaApp`'s
  // `layout` option like `AppLayout`. The account links follow the shared
  // `auth.user`: `auth/VerifyEmail` is shown to a user who is signed in.
  let { children }: { children?: Snippet } = $props()
</script>

<div class="flex min-h-screen flex-col items-center justify-center bg-gray-50 px-4 py-12 sm:px-6 lg:px-8">
  <div class="w-full max-w-md space-y-8">
    <FlashToast />
    {@render children?.()}
  </div>

  <nav class="mt-8 flex items-center gap-6 text-sm">
    <AccountLinks />
  </nav>
</div>
