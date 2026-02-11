<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { useSipRegistration } from '@/composables/useSipRegistration'
import { useSipCall } from '@/composables/useSipCall'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Separator } from '@/components/ui/separator'
import DialPad from '@/components/DialPad.vue'
import CallControls from '@/components/CallControls.vue'
import DeviceSelector from '@/components/DeviceSelector.vue'
import { Phone, LogOut, RefreshCw } from 'lucide-vue-next'
import { toast } from 'vue-sonner'

const router = useRouter()
const { isRegistered, unregister } = useSipRegistration()
const { callState, dial, hangup, webrtc } = useSipCall()

const phoneNumber = ref('')

onMounted(async () => {
  if (!isRegistered.value) {
    await router.push('/')
    return
  }
  await webrtc.enumerateDevices()
})

function onDialPadKey(key: string) {
  phoneNumber.value += key
}

async function handleDial() {
  if (!phoneNumber.value) {
    toast.error('请输入号码')
    return
  }
  try {
    await dial(phoneNumber.value)
  } catch (e) {
    toast.error(`呼叫失败: ${e}`)
  }
}

async function handleHangup() {
  await hangup()
}

async function handleLogout() {
  if (callState.value !== 'idle') {
    await hangup()
  }
  await unregister()
  await router.push('/')
}

const callStateLabel: Record<string, string> = {
  idle: '空闲',
  calling: '呼叫中...',
  ringing: '对方响铃中...',
  connected: '通话中',
  ended: '通话结束',
}
</script>

<template>
  <div class="flex min-h-screen items-center justify-center p-4">
    <Card class="w-full max-w-sm">
      <CardHeader class="pb-3">
        <div class="flex items-center justify-between">
          <CardTitle class="text-lg">拨号面板</CardTitle>
          <div class="flex items-center gap-2">
            <span class="text-xs text-green-600">● 已注册</span>
            <Button variant="ghost" size="sm" @click="handleLogout">
              <LogOut class="h-4 w-4" />
            </Button>
          </div>
        </div>
      </CardHeader>

      <CardContent class="space-y-4">
        <!-- Device selectors -->
        <div class="flex gap-2">
          <DeviceSelector
            label="麦克风"
            :devices="webrtc.microphones.value"
            :model-value="webrtc.selectedMic.value"
            @update:model-value="webrtc.setMic($event)"
          />
          <DeviceSelector
            label="扬声器"
            :devices="webrtc.speakers.value"
            :model-value="webrtc.selectedSpeaker.value"
            @update:model-value="webrtc.setSpeaker($event)"
          />
        </div>
        <div v-if="webrtc.deviceError.value" class="text-xs text-destructive">
          {{ webrtc.deviceError.value }}
        </div>
        <div v-if="!webrtc.microphones.value.length" class="flex items-center gap-2">
          <span class="text-xs text-muted-foreground">未检测到设备</span>
          <Button variant="ghost" size="sm" class="h-6 w-6 p-0" @click="webrtc.enumerateDevices()">
            <RefreshCw class="h-3 w-3" />
          </Button>
        </div>

        <Separator />

        <!-- Phone number input -->
        <div class="flex gap-2">
          <Input
            v-model="phoneNumber"
            placeholder="输入号码"
            class="text-center text-xl font-mono tracking-wider"
            :disabled="callState !== 'idle'"
          />
          <Button
            v-if="phoneNumber && callState === 'idle'"
            variant="ghost"
            size="sm"
            class="shrink-0"
            @click="phoneNumber = phoneNumber.slice(0, -1)"
          >
            ⌫
          </Button>
        </div>

        <!-- Dial pad -->
        <DialPad
          v-if="callState === 'idle'"
          :model-value="phoneNumber"
          @update:model-value="onDialPadKey"
        />

        <!-- Call state indicator -->
        <div
          v-if="callState !== 'idle'"
          class="text-center py-4"
        >
          <p class="text-lg font-medium">
            {{ callStateLabel[callState] || callState }}
          </p>
          <p class="text-sm text-muted-foreground">{{ phoneNumber }}</p>
        </div>

        <!-- Call controls -->
        <div v-if="callState === 'connected'" class="pt-2">
          <CallControls
            :is-mic-muted="webrtc.isMicMuted.value"
            :is-speaker-muted="webrtc.isSpeakerMuted.value"
            @toggle-mic="webrtc.toggleMicMute()"
            @toggle-speaker="webrtc.toggleSpeakerMute()"
            @hangup="handleHangup"
          />
        </div>

        <!-- Dial / Hangup button -->
        <div class="flex justify-center">
          <Button
            v-if="callState === 'idle'"
            size="lg"
            class="rounded-full h-14 w-14 bg-green-600 hover:bg-green-700"
            :disabled="!phoneNumber"
            @click="handleDial"
          >
            <Phone class="h-6 w-6" />
          </Button>
          <Button
            v-else-if="callState === 'calling' || callState === 'ringing'"
            variant="destructive"
            size="lg"
            class="rounded-full h-14 w-14"
            @click="handleHangup"
          >
            <Phone class="h-6 w-6 rotate-135" />
          </Button>
        </div>
      </CardContent>
    </Card>
  </div>
</template>
