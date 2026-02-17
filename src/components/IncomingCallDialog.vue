<script setup lang="ts">
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
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
    enter-active-class="transition-opacity duration-300"
    enter-from-class="opacity-0"
    enter-to-class="opacity-100"
    leave-active-class="transition-opacity duration-200"
    leave-from-class="opacity-100"
    leave-to-class="opacity-0"
  >
    <div
      v-if="open"
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-md px-6"
      @click.self="() => {}"
    >
      <!-- Dialog Card -->
      <Transition
        enter-active-class="transition-all duration-300 ease-out"
        enter-from-class="opacity-0 scale-90 translate-y-4"
        enter-to-class="opacity-100 scale-100 translate-y-0"
        leave-active-class="transition-all duration-200 ease-in"
        leave-from-class="opacity-100 scale-100"
        leave-to-class="opacity-0 scale-95"
      >
        <Card
          v-if="open"
          class="w-full max-w-sm shadow-2xl border-2 border-border/50 bg-gradient-to-b from-background to-background/95 backdrop-blur-xl"
        >
          <CardHeader class="pb-4 pt-6">
            <CardTitle class="text-center text-xl font-bold">
              <div class="inline-flex items-center gap-2">
                <div class="h-2 w-2 rounded-full bg-green-500 animate-pulse"></div>
                来电
              </div>
            </CardTitle>
          </CardHeader>

          <CardContent class="space-y-8 pb-8">
            <!-- Caller Info -->
            <div class="flex flex-col items-center space-y-4 text-center">
              <!-- Avatar/Icon -->
              <div class="relative">
                <div class="flex h-24 w-24 items-center justify-center rounded-full bg-gradient-to-br from-primary/20 to-primary/10 border-4 border-primary/20 shadow-lg">
                  <Phone class="h-12 w-12 text-primary" />
                </div>
                <div class="absolute -bottom-1 -right-1 h-6 w-6 rounded-full bg-green-500 border-2 border-background animate-pulse"></div>
              </div>

              <!-- Caller Number -->
              <div class="space-y-1">
                <div class="text-2xl font-bold text-foreground">
                  {{ caller }}
                </div>
                <div class="text-sm text-muted-foreground">
                  来电
                </div>
              </div>

              <!-- Ringing Status -->
              <div class="inline-flex items-center gap-2 text-base text-muted-foreground">
                <div class="flex gap-1">
                  <div class="h-1.5 w-1.5 rounded-full bg-primary/60 animate-bounce" style="animation-delay: 0ms;"></div>
                  <div class="h-1.5 w-1.5 rounded-full bg-primary/60 animate-bounce" style="animation-delay: 150ms;"></div>
                  <div class="h-1.5 w-1.5 rounded-full bg-primary/60 animate-bounce" style="animation-delay: 300ms;"></div>
                </div>
                响铃中
              </div>
            </div>

            <!-- Call Actions -->
            <div class="flex justify-center gap-12 pt-2">
              <!-- Reject Button -->
              <div class="flex flex-col items-center gap-3">
                <button
                  type="button"
                  class="group relative flex h-16 w-16 items-center justify-center rounded-full bg-gradient-to-br from-destructive to-destructive/80 text-destructive-foreground shadow-xl shadow-destructive/30 transition-all hover:scale-110 hover:shadow-2xl hover:shadow-destructive/40 active:scale-95"
                  @click="handleReject"
                >
                  <div class="absolute inset-0 rounded-full bg-white/20 opacity-0 group-hover:opacity-100 transition-opacity"></div>
                  <PhoneOff class="h-7 w-7 relative z-10" />
                </button>
                <span class="text-sm font-medium text-muted-foreground">拒接</span>
              </div>

              <!-- Accept Button -->
              <div class="flex flex-col items-center gap-3">
                <button
                  type="button"
                  class="group relative flex h-16 w-16 items-center justify-center rounded-full bg-gradient-to-br from-green-500 to-green-600 text-white shadow-xl shadow-green-500/30 transition-all hover:scale-110 hover:shadow-2xl hover:shadow-green-500/40 active:scale-95"
                  @click="handleAccept"
                >
                  <div class="absolute inset-0 rounded-full bg-white/20 opacity-0 group-hover:opacity-100 transition-opacity"></div>
                  <Phone class="h-7 w-7 relative z-10" />
                </button>
                <span class="text-sm font-medium text-muted-foreground">接听</span>
              </div>
            </div>
          </CardContent>
        </Card>
      </Transition>
    </div>
  </Transition>
</template>
