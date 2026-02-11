<script setup lang="ts">
import { Toggle } from '@/components/ui/toggle'
import { Button } from '@/components/ui/button'
import { Mic, MicOff, Volume2, VolumeX, PhoneOff } from 'lucide-vue-next'

defineProps<{
  isMicMuted: boolean
  isSpeakerMuted: boolean
}>()

const emit = defineEmits<{
  toggleMic: []
  toggleSpeaker: []
  hangup: []
}>()
</script>

<template>
  <div class="flex items-center justify-center gap-4">
    <Toggle
      :pressed="isMicMuted"
      aria-label="切换麦克风"
      @update:pressed="emit('toggleMic')"
    >
      <MicOff v-if="isMicMuted" class="h-5 w-5" />
      <Mic v-else class="h-5 w-5" />
    </Toggle>

    <Button
      variant="destructive"
      size="lg"
      class="rounded-full h-14 w-14"
      @click="emit('hangup')"
    >
      <PhoneOff class="h-6 w-6" />
    </Button>

    <Toggle
      :pressed="isSpeakerMuted"
      aria-label="切换扬声器"
      @update:pressed="emit('toggleSpeaker')"
    >
      <VolumeX v-if="isSpeakerMuted" class="h-5 w-5" />
      <Volume2 v-else class="h-5 w-5" />
    </Toggle>
  </div>
</template>
