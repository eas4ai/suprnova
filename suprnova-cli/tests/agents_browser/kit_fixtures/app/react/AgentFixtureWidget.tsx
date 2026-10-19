import { usePageTool } from '../lib/agents'

// A component whose tool lives while it is mounted (AGENT-015).
export default function AgentFixtureWidget() {
  usePageTool({
    name: 'fixture.widget.ping',
    description: 'Answer pong while the widget is shown.',
    inputSchema: { type: 'object', properties: {} },
    readOnly: true,
    execute: () => ({ pong: true }),
  })
  return <p data-testid="widget">Widget</p>
}
