<script lang="ts">
  // Runs a stream hook while the component initializes, as a page does.
  let { setup }: { setup: () => void } = $props()
  setup()
</script>
