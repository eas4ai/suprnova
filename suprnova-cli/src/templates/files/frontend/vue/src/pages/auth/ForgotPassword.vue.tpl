<script setup lang="ts">
import { ref } from 'vue'
import { useForm } from '@inertiajs/vue3'

const form = useForm({
  email: '',
})

// The server answers every address the same way, so "sent" is a fact
// about this submission, not about the account.
const sent = ref(false)

function submit() {
  form.post('/forgot-password', {
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
          Reset your password
        </h2>
        <p class="mt-2 text-center text-sm text-gray-600">
          Enter the email address you verified for your account and we will send you a link
          to choose a new password.
        </p>
      </div>

      <p v-if="sent" role="status" class="text-center text-sm text-green-700">
        If that address belongs to a verified account, a reset link is on its way.
      </p>

      <form class="mt-8 space-y-6" @submit.prevent="submit">
        <div>
          <label for="email" class="sr-only">Email address</label>
          <input
            id="email"
            v-model="form.email"
            name="email"
            type="email"
            autocomplete="email"
            required
            class="appearance-none relative block w-full px-3 py-2 border border-gray-300 placeholder-gray-500 text-gray-900 rounded-md focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 focus:z-10 sm:text-sm"
            placeholder="Email address"
          />
          <p v-if="form.errors.email" class="mt-1 text-sm text-red-600">
            {{ form.errors.email }}
          </p>
        </div>

        <div>
          <button
            type="submit"
            :disabled="form.processing"
            class="group relative w-full flex justify-center py-2 px-4 border border-transparent text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 disabled:opacity-50"
          >
            {{ form.processing ? 'Sending...' : 'Send reset link' }}
          </button>
        </div>

        <div class="text-center">
          <a href="/login" class="text-indigo-600 hover:text-indigo-500">
            Back to sign in
          </a>
        </div>
      </form>
    </div>
  </div>
</template>
