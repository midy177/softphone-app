<script setup lang="ts">
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Phone, PhoneOff } from 'lucide-vue-next'

interface Props {
  open: boolean
  caller: string
  callee?: string
}

interface Emits {
  (e: 'accept'): void
  (e: 'reject'): void
}

defineProps<Props>()
const emit = defineEmits<Emits>()

function handleAccept() {
  emit('accept')
}

function handleReject() {
  emit('reject')
}
</script>

<template>
  <!-- Overlay backdrop -->
  <Transition
    enter-active-class="transition-opacity duration-200"
    enter-from-class="opacity-0"
    enter-to-class="opacity-100"
    leave-active-class="transition-opacity duration-200"
    leave-from-class="opacity-100"
    leave-to-class="opacity-0"
  >
    <div
      v-if="open"
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
      @click.self="() => {}"
    >
      <!-- Dialog Card -->
      <Transition
        enter-active-class="transition-all duration-200"
        enter-from-class="opacity-0 scale-95"
        enter-to-class="opacity-100 scale-100"
        leave-active-class="transition-all duration-200"
        leave-from-class="opacity-100 scale-100"
        leave-to-class="opacity-0 scale-95"
      >
        <Card v-if="open" class="w-full max-w-sm shadow-2xl">
          <CardHeader class="pb-4">
            <CardTitle class="text-center text-xl">来电</CardTitle>
          </CardHeader>

          <CardContent class="space-y-6">
            <!-- Caller Info -->
            <div class="space-y-2 text-center">
              <div class="text-2xl font-semibold text-foreground">
                {{ caller }}
              </div>
              <div v-if="callee" class="text-sm text-muted-foreground">
                呼叫至: {{ callee }}
              </div>
              <div class="text-lg text-primary font-medium">
                响铃中...
              </div>
            </div>

            <!-- Call Actions -->
            <div class="flex justify-center gap-8">
              <!-- Reject Button -->
              <div class="flex flex-col items-center gap-2">
                <button
                  type="button"
                  class="flex h-16 w-16 items-center justify-center rounded-full bg-destructive text-destructive-foreground shadow-lg transition-transform hover:scale-110 active:scale-95"
                  @click="handleReject"
                >
                  <PhoneOff class="h-8 w-8" />
                </button>
                <span class="text-xs text-muted-foreground">挂断</span>
              </div>

              <!-- Accept Button -->
              <div class="flex flex-col items-center gap-2">
                <button
                  type="button"
                  class="flex h-16 w-16 items-center justify-center rounded-full bg-green-600 text-white shadow-lg transition-transform hover:scale-110 active:scale-95"
                  @click="handleAccept"
                >
                  <Phone class="h-8 w-8" />
                </button>
                <span class="text-xs text-muted-foreground">接听</span>
              </div>
            </div>
          </CardContent>
        </Card>
      </Transition>
    </div>
  </Transition>
</template>
