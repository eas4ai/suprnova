<script setup lang="ts">
import { ref } from 'vue'
import { useForm } from '@inertiajs/vue3'
import type { VerifyEmailProps } from '../../types/inertia-props'

defineProps<VerifyEmailProps>()

const form = useForm({})
const sent = ref(false)

function resend() {
  form.post('/email/verification-notification', {
    onSuccess: () => {
      sent.value = true
    },
  })
}
</script>

<template>
  <div
    class="min-h-screen flex items-center justify-center bg-gray-50 py-12 px-4 sm:px-6 lg:px-8"
  >
    <div class="max-w-md w-full space-y-8">
      <div>
        <h2 class="mt-6 text-center text-3xl font-extrabold text-gray-900">
          Verify your email address
        </h2>
        <p class="mt-2 text-center text-sm text-gray-600">
          We sent a verification link to <strong>{{ email }}</strong>. Open it while signed in
          to this account.
        </p>
      </div>

      <p v-if="sent" role="status" class="text-center text-sm text-green-700">
        A new link has been sent.
      </p>

      <form class="mt-8 space-y-6" @submit.prevent="resend">
        <div>
          <button
            type="submit"
            :disabled="form.processing"
            class="group relative w-full flex justify-center py-2 px-4 border border-transparent text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 disabled:opacity-50"
          >
            {{ form.processing ? 'Sending...' : 'Resend verification link' }}
          </button>
        </div>

        <div class="text-center">
          <a href="/dashboard" class="text-indigo-600 hover:text-indigo-500">
            Continue to your dashboard
          </a>
        </div>
      </form>
    </div>
  </div>
</template>
