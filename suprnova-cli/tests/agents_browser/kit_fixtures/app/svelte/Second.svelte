<script lang="ts">
  import { Link, usePage } from '@inertiajs/svelte'

  let { counter }: { counter: number } = $props()
  const { root } = usePage().props
</script>

<div>
  <h1>Second</h1>
  <p data-testid="counter">{counter}</p>
  <Link href={`${root}/agent-fixture/first`} data-testid="to-first">First</Link>
</div>
