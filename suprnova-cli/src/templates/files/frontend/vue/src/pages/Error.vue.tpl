<script setup lang="ts">
// These props come from the framework, not from one of your handlers:
// `InertiaConfig::error_page` in `src/bootstrap.rs` routes every
// framework error response (403, 404, 429, 500, ...) to this page.
// That is why they are declared here rather than imported from
// `types/inertia-props.ts`, which `suprnova generate-types` rewrites
// from your `#[derive(InertiaProps)]` structs.
interface ErrorProps {
  status: number
  message: string
  request_id?: string
}

defineProps<ErrorProps>()
</script>

<template>
  <div class="min-h-screen flex items-center justify-center bg-gray-50 py-12 px-4 sm:px-6 lg:px-8">
    <div class="max-w-md w-full text-center space-y-4">
      <h1 class="text-6xl font-extrabold text-gray-900">{{ status }}</h1>
      <p class="text-lg text-gray-700">{{ message }}</p>
      <p v-if="request_id" class="text-sm text-gray-500">
        Reference:
        <code class="bg-gray-100 px-1 rounded">{{ request_id }}</code>
      </p>
      <p>
        <a href="/" class="text-indigo-600 hover:text-indigo-500">Go home</a>
      </p>
    </div>
  </div>
</template>
