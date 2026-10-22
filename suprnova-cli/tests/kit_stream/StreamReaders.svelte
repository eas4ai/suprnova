<script lang="ts">
  import StreamProbe from './StreamProbe.svelte'

  let { readers }: { readers: Array<() => void> } = $props()
</script>

{#each readers as setup}
  <StreamProbe {setup} />
{/each}
