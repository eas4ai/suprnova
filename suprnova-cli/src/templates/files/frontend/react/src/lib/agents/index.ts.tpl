// React hooks for agent tools on a page, over the engine in `core.ts`
// (AGENT-015). The page's server tools register through `installPageTools`,
// which `main.tsx` calls before the application starts (AGENT-014).
//
//   usePageTool({ name, description, inputSchema, execute })
//     a tool that lives while the component is mounted
//   useFormTool(form, { name, description, schema, action })
//     a `useForm` form as a tool, its schema from `@/types/tool-schemas`
//
// Tool names and descriptions are what an agent reads as instructions:
// write them as code literals, never from user data.

import type { InertiaFormProps } from '@inertiajs/react'
import { useEffect, useRef } from 'react'
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
 * component unmounts. `execute` may be a new closure every render: the
 * call runs the latest one, and only a change to the tool's name, text,
 * schema or hints registers it again.
 */
export function usePageTool<TInput = Record<string, unknown>>(options: PageToolOptions<TInput>): void {
  const latest = useRef(options)
  latest.current = options
  const { name, title, description, readOnly, untrustedContent, consequential } = options
  const schema = JSON.stringify(options.inputSchema)
  useEffect(
    () => registerTool(componentTool(latest.current, () => latest.current.execute)),
    [name, title, description, schema, readOnly, untrustedContent, consequential],
  )
}

/**
 * Make a `useForm` form a tool while the component is mounted. A call
 * fills the form, which the user sees, and submits it through Inertia; the
 * route's validation errors come back as a tool error naming each field.
 * A destructive form, or one whose action is a `DELETE`, is filled and left
 * for the user to submit. The tool sends no tool-call header, so the
 * route's own throttle applies rather than the server tool budget.
 */
export function useFormTool<TForm extends Record<string, unknown>>(
  form: Pick<InertiaFormProps<TForm>, 'setData' | 'submit' | 'cancel'>,
  options: FormToolOptions,
): void {
  const latest = useRef(form)
  latest.current = form
  const settings = useRef(options)
  settings.current = options
  const { name, title, description, destructive } = options
  const { method, url } = options.action
  const schema = JSON.stringify(options.schema)
  useEffect(
    () =>
      registerTool(
        formTool(settings.current, () => ({
          // `setData` with a function updates the form at once, so a submit
          // in the same tick sends the filled values.
          fill: (values) => latest.current.setData((data: TForm) => ({ ...data, ...values })),
          submit: (method, url, visit) => latest.current.submit(method, url, visit),
          cancel: () => latest.current.cancel(),
        })),
      ),
    [name, title, description, schema, method, url, destructive],
  )
}
