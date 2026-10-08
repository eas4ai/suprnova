<script lang="ts">
  import { Link } from '@inertiajs/svelte'
  import type { Snippet } from 'svelte'
  import FlashToast from '../components/FlashToast.svelte'
  import type { SharedProps } from '../types/inertia-props'

  // The frame of every page outside `auth/`. `createInertiaApp`'s `layout`
  // option in `main.ts` and `ssr.ts` applies it, so it is mounted once and
  // stays mounted while visits move between the pages that use it: state
  // kept here survives a visit, and the navigation never flickers.
  //
  // A layout is rendered with the page's own props, the props a page set
  // with `setLayoutProps`, and the page as `children`
  // (`packages/svelte/src/components/App.svelte`, `resolveLayout`). So
  // `root` and `auth` are the props the server shares with every page, and
  // `heading` is there while the page that set it is shown: a visit to
  // another page resets the layout props.
  interface Props extends SharedProps {
    heading?: string
    children?: Snippet
  }

  let { root, auth, heading, children }: Props = $props()
</script>

<div class="min-h-screen bg-gray-100">
  <nav class="bg-white shadow">
    <div class="mx-auto flex h-16 max-w-7xl items-center justify-between px-4 sm:px-6 lg:px-8">
      <div class="flex items-center gap-6 text-sm font-medium">
        <Link href={`${root}/dashboard`} class="text-gray-700 hover:text-gray-900">Dashboard</Link>
        <Link href={`${root}/notes`} class="text-gray-700 hover:text-gray-900">Notes</Link>
      </div>
      <div class="flex items-center gap-4 text-sm">
        {#if auth.user}
          <span class="text-gray-700">{auth.user.name}</span>
          <!-- A post, so it renders as a button. `preserveState={false}`
               starts the page it lands on afresh: a method other than GET
               keeps the page's state by default, and this page's layout
               props would follow the visitor out. -->
          <Link
            href={`${root}/logout`}
            method="post"
            as="button"
            preserveState={false}
            class="rounded-md px-3 py-2 font-medium text-gray-500 hover:text-gray-700"
          >
            Sign out
          </Link>
        {:else}
          <Link href={`${root}/login`} class="text-indigo-600 hover:text-indigo-500">Sign in</Link>
          <Link href={`${root}/register`} class="text-indigo-600 hover:text-indigo-500">
            Register
          </Link>
        {/if}
      </div>
    </div>
  </nav>

  <FlashToast />

  <main class="mx-auto max-w-7xl px-4 py-6 sm:px-6 lg:px-8">
    {#if heading}
      <h1 class="mb-6 text-2xl font-semibold text-gray-900">{heading}</h1>
    {/if}
    {@render children?.()}
  </main>
</div>
