<script lang="ts">
  import { Link, page } from '@inertiajs/svelte'

  // The account end of both layouts' navigation, from what the server
  // shares with every page: `root`, and `auth.user`, the signed-in user or
  // `null`. Both layouts need both branches: the guest frame holds
  // `auth/VerifyEmail`, a page for a user who is signed in, and the
  // application frame holds `Home` and `Error`, which a visitor sees too.
  // Read from the page, so a change to the user (a new name, a sign-out)
  // shows here at once.
  const root = $derived(page.props.root)
  const user = $derived(page.props.auth.user)
</script>

{#if user}
  <span class="text-gray-700">{user.name}</span>
  <!-- A post, so it renders as a button. `preserveState={false}` starts the
       page it lands on afresh: a method other than GET keeps the page's
       state by default, and this page's layout props would follow the
       visitor out. -->
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
  <Link href={`${root}/register`} class="text-indigo-600 hover:text-indigo-500">Register</Link>
{/if}
