// Vue composables for agent tools on a page, over the engine in `core.ts`
// (AGENT-015). The page's server tools register through `installPageTools`,
// which `main.ts` calls before the application starts (AGENT-014).
//
//   usePageTool({ name, description, inputSchema, execute })
//     a tool that lives while the component is mounted
//   useFormTool(form, { name, description, schema, action })
//     a `useForm` form as a tool, its schema from `src/types/tool-schemas.ts`,
//     which a page under `pages/Notes/` imports as
//     `import { toolSchemas } from '../../types/tool-schemas'`
//
// Tool names and descriptions are what an agent reads as instructions:
// write them as code literals, never from user data. A composable
// registers when its component mounts, so the server render registers
// nothing.

import type { InertiaFormProps } from '@inertiajs/vue3'
import { onBeforeUnmount, onMounted } from 'vue'
import {
  componentTool,
  formTool,
  registerTool,
  type FormToolOptions,
  type PageToolOptions,
  type ToolDefinition,
} from './core'

export { installPageTools } from './inertia'
export {
  callPageTool,
  modelContextOf,
  registerTool,
  type FormAction,
  type FormToolOptions,
  type PageToolOptions,
  type ToolAnnotations,
  type ToolDefinition,
  type ToolInputSchema,
  type ToolResult,
} from './core'

/** Register `definition` from mount to unmount of the calling component. */
function whileMounted(definition: () => ToolDefinition): void {
  let leave: (() => void) | null = null
  onMounted(() => {
    leave = registerTool(definition())
  })
  onBeforeUnmount(() => {
    leave?.()
    leave = null
  })
}

/**
 * Register a tool while the component is mounted; it leaves when the
 * component unmounts. Each call runs the `execute` the options hold then.
 */
export function usePageTool<TInput = Record<string, unknown>>(options: PageToolOptions<TInput>): void {
  whileMounted(() => componentTool(options, () => options.execute))
}

/**
 * Make a `useForm` form a tool while the component is mounted. A call
 * fills the form, which the user sees, and submits it through Inertia; the
 * route's validation errors come back as a tool error naming each field.
 * A destructive form, or one whose action is a `DELETE`, is filled and left
 * for the user to submit. The tool sends no tool-call header, so the
 * route's own throttle applies rather than the server tool budget. It
 * marks its submission with the form-tool header, which the approval and
 * claim pages refuse: consent comes from the person.
 */
export function useFormTool<TForm extends object>(
  form: Pick<InertiaFormProps<TForm>, 'submit' | 'cancel'> & TForm,
  options: FormToolOptions,
): void {
  whileMounted(() =>
    formTool(options, () => ({
      fill: (values) => {
        for (const [key, value] of Object.entries(values)) {
          // The form's fields are reactive properties of the form itself.
          Reflect.set(form, key, value)
        }
      },
      submit: (method, url, visit) => form.submit(method, url, visit),
      cancel: () => form.cancel(),
    })),
  )
}
