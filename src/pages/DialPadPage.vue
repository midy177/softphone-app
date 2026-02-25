<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { useSipRegistration } from '@/composables/useSipRegistration'
import { useSipCall } from '@/composables/useSipCall'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import DialPad from '@/components/DialPad.vue'
import CallControls from '@/components/CallControls.vue'
import IncomingCallDialog from '@/components/IncomingCallDialog.vue'
import { Phone, LogOut, Settings } from 'lucide-vue-next'
import { toast } from 'vue-sonner'

const router = useRouter()
const { isRegistered, currentExtension, unregister } = useSipRegistration()
const { callState, callee, incomingCall, dial, hangup, answerCall, rejectCall, sendDtmf, audio } = useSipCall()

const phoneNumber = ref('')
const dtmfInput = ref('')

onMounted(async () => {
  if (!isRegistered.value) {
    await router.push('/')
    return
  }
  await audio.enumerateDevices()
})

function onDialPadKey(key: string) {
  if (callState.value === 'connected') {
    // 通话中发送 DTMF 并显示在输入框
    dtmfInput.value += key
    sendDtmf(key).catch((e) => {
      toast.error(`发送 DTMF 失败: ${e}`)
    })
  } else {
    // 空闲时添加到号码
    phoneNumber.value += key
  }
}

async function handleDial() {
  if (!phoneNumber.value) {
    toast.error('请输入号码')
    return
  }
  try {
    await dial(phoneNumber.value)
    // 拨号成功后清空 DTMF 输入
    dtmfInput.value = ''
  } catch (e) {
    toast.error(`呼叫失败: ${e}`)
  }
}

async function handleHangup() {
  await hangup()
  // 挂断后清空 DTMF 输入
  dtmfInput.value = ''
}

async function handleLogout() {
  try {
    await unregister()
  } catch (e) {
    console.error('[Logout] Unregister error:', e)
  } finally {
    // 无论注销是否成功，都返回登录页
    await router.replace('/')
  }
}

async function handleAnswer() {
  try {
    await answerCall()
    // 接听成功后清空 DTMF 输入
    dtmfInput.value = ''
  } catch (e) {
    toast.error(`接听失败: ${e}`)
  }
}

async function handleReject() {
  try {
    await rejectCall()
  } catch (e) {
    toast.error(`拒绝失败: ${e}`)
  }
}

async function handleToggleMic() {
  console.log('[DialPadPage] handleToggleMic called')
  await audio.toggleMicMute()
}

async function handleToggleSpeaker() {
  console.log('[DialPadPage] handleToggleSpeaker called')
  await audio.toggleSpeakerMute()
}

const callStateLabel: Record<string, string> = {
  idle: '空闲',
  calling: '呼叫中...',
  ringing: '对方响铃中...',
  connected: '通话中',
  incoming: '来电中...',
  ended: '通话结束',
}
</script>

<template>
  <div class="h-screen flex flex-col overflow-hidden">
    <!-- Incoming Call Dialog -->
    <IncomingCallDialog
      :open="callState === 'incoming' && !!incomingCall"
      :caller="incomingCall?.caller || ''"
      :callee="incomingCall?.callee"
      @accept="handleAnswer"
      @reject="handleReject"
    />

    <Card class="w-full border-0 shadow-none bg-transparent backdrop-blur-md rounded-none flex-1 min-h-0">
      <CardHeader v-if="callState !== 'connected'" class="pb-0">
        <div class="flex items-center justify-between">
          <CardTitle class="text-lg font-bold">拨号面板</CardTitle>
          <div class="flex items-center gap-2">
            <span class="text-md font-bold text-green-600">
              ● 已注册{{ currentExtension ? ` (${currentExtension})` : '' }}
            </span>
            <Button variant="ghost" size="sm" @click="router.push('/settings')">
              <Settings class="h-4 w-4" />
            </Button>
            <Button variant="ghost" size="sm" @click="handleLogout">
              <LogOut class="h-4 w-4" />
            </Button>
          </div>
        </div>
      </CardHeader>

      <CardContent class="flex-1 flex flex-col min-h-0" :class="callState === 'connected' ? 'space-y-2 pt-4' : 'space-y-3'">
        <!-- Phone number input -->
        <div v-if="callState === 'idle'" class="relative">
          <Input
            v-model="phoneNumber"
            placeholder="输入号码"
            class="text-center text-xl font-mono tracking-wider pr-10"
          />
          <Button
            v-if="phoneNumber"
            variant="ghost"
            size="sm"
            class="absolute right-1 top-1/2 -translate-y-1/2 h-7 w-7 p-0"
            @click="phoneNumber = phoneNumber.slice(0, -1)"
          >
            ⌫
          </Button>
        </div>

        <!-- DTMF input display (during call) -->
        <div v-if="callState === 'connected'" class="relative">
          <Input
            v-model="dtmfInput"
            placeholder="DTMF"
            class="text-center text-xl font-mono tracking-wider pr-10"
            readonly
          />
          <Button
            v-if="dtmfInput"
            variant="ghost"
            size="sm"
            class="absolute right-1 top-1/2 -translate-y-1/2 h-7 w-7 p-0"
            @click="dtmfInput = dtmfInput.slice(0, -1)"
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
          v-if="callState !== 'idle' && callState !== 'connected'"
          class="flex flex-col items-center justify-center gap-6 flex-1"
        >
          <p class="text-lg font-medium">
            {{ callStateLabel[callState] || callState }}
          </p>
          <p class="text-sm text-muted-foreground">{{ callee || phoneNumber }}</p>
          <Button
            variant="destructive"
            size="lg"
            class="rounded-full h-14 w-14"
            @click="handleHangup"
          >
            <Phone class="h-6 w-6 rotate-135" />
          </Button>
        </div>

        <!-- Call controls -->
        <div v-if="callState === 'connected'" class="space-y-2">
          <div class="text-center py-1">
            <p class="text-sm font-medium text-muted-foreground">
              {{ callStateLabel[callState] || callState }}
            </p>
            <p class="text-base font-medium">{{ callee || phoneNumber }}</p>
          </div>
          <CallControls
            :is-mic-muted="audio.isMicMuted.value"
            :is-speaker-muted="audio.isSpeakerMuted.value"
            @toggle-mic="handleToggleMic"
            @toggle-speaker="handleToggleSpeaker"
            @hangup="handleHangup"
          />
          <!-- DTMF 拨号盘 -->
          <div class="pt-1">
            <DialPad
              :model-value="''"
              @update:model-value="onDialPadKey"
            />
          </div>
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
        </div>
      </CardContent>
    </Card>
  </div>
</template>
