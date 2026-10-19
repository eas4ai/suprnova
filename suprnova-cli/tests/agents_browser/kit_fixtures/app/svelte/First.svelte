<script lang="ts">
  import { Link, usePage } from '@inertiajs/svelte'
  import AgentFixtureWidget from '../../components/AgentFixtureWidget.svelte'

  let { counter }: { counter: number } = $props()
  const { root } = usePage().props
  let shown = $state(true)
</script>

<div>
  <h1>First</h1>
  <p data-testid="counter">{counter}</p>
  <button type="button" data-testid="toggle" onclick={() => (shown = !shown)}>Toggle the widget</button>
  {#if shown}
    <AgentFixtureWidget />
  {/if}
  <Link href={`${root}/agent-fixture/second`} data-testid="to-second">Second</Link>
</div>
