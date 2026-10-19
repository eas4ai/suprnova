// Svelte helpers for agent tools on a page, over the engine in `core.ts`
// (AGENT-015). The page's server tools register through `installPageTools`,
// which `main.ts` calls before the application starts (AGENT-014).
//
//   usePageTool({ name, description, inputSchema, execute })
//     a tool that lives while the component is mounted
//   useFormTool(form, { name, description, schema, action })
//     a `useForm` form as a tool, its schema from `../types/tool-schemas`
//
// Call them while the component initializes. Tool names and descriptions
// are what an agent reads as instructions: write them as code literals,
// never from user data. A helper registers when its component mounts, so
// the server render registers nothing.

import type { InertiaFormProps } from '@inertiajs/svelte'
import { onMount } from 'svelte'
import {
  componentTool,
  formTool,
  registerTool,
  type FormToolOptions,
  type PageToolOptions,
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

/**
 * Register a tool while the component is mounted; it leaves when the
 * component is destroyed. Each call runs the `execute` the options hold
 * then.
 */
export function usePageTool<TInput = Record<string, unknown>>(options: PageToolOptions<TInput>): void {
  onMount(() => registerTool(componentTool(options, () => options.execute)))
}

/**
 * Make a `useForm` form a tool while the component is mounted. A call
 * fills the form, which the user sees, and submits it through Inertia; the
 * route's validation errors come back as a tool error naming each field.
 * A destructive form, or one whose action is a `DELETE`, is filled and left
 * for the user to submit. The tool sends no tool-call header, so the
 * route's own throttle applies rather than the server tool budget.
 */
export function useFormTool<TForm extends object>(
  form: Pick<InertiaFormProps<TForm>, 'submit' | 'cancel'> & TForm,
  options: FormToolOptions,
): void {
  onMount(() =>
    registerTool(
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
    ),
  )
}
