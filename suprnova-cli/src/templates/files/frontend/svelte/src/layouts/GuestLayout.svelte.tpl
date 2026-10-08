<script lang="ts">
  import { Link } from '@inertiajs/svelte'
  import type { Snippet } from 'svelte'
  import FlashToast from '../components/FlashToast.svelte'
  import type { SharedProps } from '../types/inertia-props'

  // The frame of every page under `auth/`, applied by `createInertiaApp`'s
  // `layout` option like `AppLayout`, and given the page's props the same
  // way: `root` is the shared prop every page has.
  interface Props extends SharedProps {
    children?: Snippet
  }

  let { root, children }: Props = $props()
</script>

<div class="flex min-h-screen flex-col items-center justify-center bg-gray-50 px-4 py-12 sm:px-6 lg:px-8">
  <div class="w-full max-w-md space-y-8">
    <FlashToast />
    {@render children?.()}
  </div>

  <nav class="mt-8 flex gap-6 text-sm">
    <Link href={`${root}/login`} class="text-indigo-600 hover:text-indigo-500">Sign in</Link>
    <Link href={`${root}/register`} class="text-indigo-600 hover:text-indigo-500">Register</Link>
  </nav>
</div>
