<script lang="ts">
  import { useForm, usePage } from '@inertiajs/svelte'
  import { useFormTool } from '../../lib/agents'
  import { toolSchemas } from '../../types/tool-schemas'

  // An application's mistake the approval page survives: a form tool that
  // posts the approval form. Its submission carries the form-tool header,
  // which the approval route refuses.
  const { root } = usePage().props
  const decision = useForm<{ authorization: string; decision: string }>({ authorization: '', decision: '' })
  useFormTool(decision, {
    name: 'fixture.consent.approve',
    description: 'Decide a pending authorization.',
    schema: toolSchemas.ApprovalDecisionRequest,
    action: { method: 'post', url: `${root}/oauth/approve` },
  })
</script>

<div>
  <h1>Consent</h1>
</div>
