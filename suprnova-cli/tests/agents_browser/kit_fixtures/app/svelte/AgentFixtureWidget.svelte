<script lang="ts">
  import { usePageTool } from '../lib/agents'

  // A component whose tool lives while it is mounted (AGENT-015).
  usePageTool({
    name: 'fixture.widget.ping',
    description: 'Answer pong while the widget is shown.',
    inputSchema: { type: 'object', properties: {} },
    readOnly: true,
    execute: () => ({ pong: true }),
  })
</script>

<p data-testid="widget">Widget</p>
