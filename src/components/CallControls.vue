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

function handleMicToggle() {
  console.log('[CallControls] Mic toggle clicked')
  emit('toggleMic')
}

function handleSpeakerToggle() {
  console.log('[CallControls] Speaker toggle clicked')
  emit('toggleSpeaker')
}
</script>

<template>
  <div class="flex items-center justify-center gap-3">
    <Toggle
      :pressed="isMicMuted"
      aria-label="切换麦克风"
      @update:pressed="handleMicToggle"
      @click="handleMicToggle"
      size="sm"
    >
      <MicOff v-if="isMicMuted" class="h-4 w-4" />
      <Mic v-else class="h-4 w-4" />
    </Toggle>

    <Button
      variant="destructive"
      size="lg"
      class="rounded-full h-12 w-12"
      @click="emit('hangup')"
    >
      <PhoneOff class="h-5 w-5" />
    </Button>

    <Toggle
      :pressed="isSpeakerMuted"
      aria-label="切换扬声器"
      @update:pressed="handleSpeakerToggle"
      @click="handleSpeakerToggle"
      size="sm"
    >
      <VolumeX v-if="isSpeakerMuted" class="h-4 w-4" />
      <Volume2 v-else class="h-4 w-4" />
    </Toggle>
  </div>
</template>
