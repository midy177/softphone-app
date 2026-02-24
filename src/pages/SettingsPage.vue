<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { open } from '@tauri-apps/plugin-dialog'
import { useSipRegistration } from '@/composables/useSipRegistration'
import { useAudio } from '@/composables/useAudio'
import { getSavedSipFlowConfig, saveSipFlowConfig, getAppConfig, saveAppConfig } from '@/utils/configManager'
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { ArrowLeft, RefreshCw, FolderOpen } from 'lucide-vue-next'

const router = useRouter()
const { isRegistered } = useSipRegistration()
const audio = useAudio()

interface SipFlowConfig {
  enabled: boolean
  log_dir: string
}

// SIP 消息日志开关
const sipFlowEnabled = ref(false)
const sipFlowLoading = ref(false)
const sipFlowDir = ref('')

// SRTP 优先配置
const preferSrtp = ref(true)
const srtpLoading = ref(false)

// 麦克风降噪
const noiseReduce = ref(false)
const noiseReduceLoading = ref(false)

// 扬声器降噪
const speakerNoiseReduce = ref(false)
const speakerNoiseReduceLoading = ref(false)

// 始终在最前端
const alwaysOnTop = ref(false)
const alwaysOnTopLoading = ref(false)

async function toggleSipFlow() {
  console.log('[SettingsPage] toggleSipFlow called, current state:', sipFlowEnabled.value)
  if (sipFlowLoading.value) {
    console.log('[SettingsPage] Already loading, skipping')
    return
  }

  sipFlowLoading.value = true
  const newEnabled = !sipFlowEnabled.value

  try {
    console.log('[SettingsPage] Setting SIP flow enabled to:', newEnabled)

    await invoke('set_sip_flow_enabled', { enabled: newEnabled })
    console.log('[SettingsPage] Backend command completed')

    sipFlowEnabled.value = newEnabled
    console.log('[SettingsPage] Updated local state to:', sipFlowEnabled.value)

    // 保存到 localStorage
    const config = getAppConfig() || {
      sip_flow: { enabled: newEnabled, log_dir: sipFlowDir.value },
      prefer_srtp: preferSrtp.value,
      noise_reduce: noiseReduce.value,
      speaker_noise_reduce: speakerNoiseReduce.value,
    }
    config.sip_flow.enabled = newEnabled
    saveAppConfig(config)
    console.log('[SettingsPage] Config saved to localStorage')

    const message = newEnabled ? 'SIP 消息日志已开启' : 'SIP 消息日志已关闭'
    console.log('[SettingsPage]', message)
  } catch (e) {
    console.error('[SettingsPage] Error setting SIP flow enabled:', e)
  } finally {
    sipFlowLoading.value = false
    console.log('[SettingsPage] Toggle complete, new state:', sipFlowEnabled.value)
  }
}

onMounted(async () => {
  // 枚举设备
  await audio.enumerateDevices()

  // 加载配置（从 localStorage，因为 App.vue 已经恢复到后端了）
  await loadConfig()
})

async function loadConfig() {
  // 尝试从新的统一配置加载
  const appConfig = getAppConfig()

  if (appConfig) {
    sipFlowEnabled.value = appConfig.sip_flow.enabled
    sipFlowDir.value = appConfig.sip_flow.log_dir
    preferSrtp.value = appConfig.prefer_srtp
    noiseReduce.value = appConfig.noise_reduce ?? false
    speakerNoiseReduce.value = appConfig.speaker_noise_reduce ?? false
    alwaysOnTop.value = appConfig.always_on_top ?? false
    console.log('[SettingsPage] Loaded config from localStorage:', appConfig)
  } else {
    // 如果没有保存的配置，从后端获取默认值
    try {
      const sipFlowConfig = await invoke<SipFlowConfig>('get_sip_flow_config')
      sipFlowEnabled.value = sipFlowConfig.enabled
      sipFlowDir.value = sipFlowConfig.log_dir

      const srtpConfig = await invoke<boolean>('get_prefer_srtp')
      preferSrtp.value = srtpConfig

      const noiseReduceConfig = await invoke<boolean>('get_noise_reduce')
      noiseReduce.value = noiseReduceConfig

      const speakerNoiseReduceConfig = await invoke<boolean>('get_speaker_noise_reduce')
      speakerNoiseReduce.value = speakerNoiseReduceConfig

      // 保存默认配置
      saveAppConfig({
        sip_flow: sipFlowConfig,
        prefer_srtp: srtpConfig,
        noise_reduce: noiseReduceConfig,
        speaker_noise_reduce: speakerNoiseReduceConfig,
        always_on_top: false,
      })
      console.log('[SettingsPage] Loaded default config from backend')
    } catch (e) {
      console.error('[SettingsPage] Failed to load config:', e)
    }
  }
}

async function toggleNoiseReduce() {
  if (noiseReduceLoading.value) return

  noiseReduceLoading.value = true
  const newEnabled = !noiseReduce.value

  try {
    await invoke('set_noise_reduce', { enabled: newEnabled })
    noiseReduce.value = newEnabled

    const config = getAppConfig() || {
      sip_flow: { enabled: sipFlowEnabled.value, log_dir: sipFlowDir.value },
      prefer_srtp: preferSrtp.value,
      noise_reduce: newEnabled,
      speaker_noise_reduce: speakerNoiseReduce.value,
    }
    config.noise_reduce = newEnabled
    saveAppConfig(config)
  } catch (e) {
    console.error('[SettingsPage] Error setting noise reduce:', e)
  } finally {
    noiseReduceLoading.value = false
  }
}

async function toggleSpeakerNoiseReduce() {
  if (speakerNoiseReduceLoading.value) return

  speakerNoiseReduceLoading.value = true
  const newEnabled = !speakerNoiseReduce.value

  try {
    await invoke('set_speaker_noise_reduce', { enabled: newEnabled })
    speakerNoiseReduce.value = newEnabled

    const config = getAppConfig() || {
      sip_flow: { enabled: sipFlowEnabled.value, log_dir: sipFlowDir.value },
      prefer_srtp: preferSrtp.value,
      noise_reduce: noiseReduce.value,
      speaker_noise_reduce: newEnabled,
    }
    config.speaker_noise_reduce = newEnabled
    saveAppConfig(config)
  } catch (e) {
    console.error('[SettingsPage] Error setting speaker noise reduce:', e)
  } finally {
    speakerNoiseReduceLoading.value = false
  }
}

async function toggleAlwaysOnTop() {
  if (alwaysOnTopLoading.value) return

  alwaysOnTopLoading.value = true
  const newValue = !alwaysOnTop.value

  try {
    const win = getCurrentWindow()
    await win.setAlwaysOnTop(newValue)
    alwaysOnTop.value = newValue

    const config = getAppConfig() || {
      sip_flow: { enabled: sipFlowEnabled.value, log_dir: sipFlowDir.value },
      prefer_srtp: preferSrtp.value,
      noise_reduce: noiseReduce.value,
      speaker_noise_reduce: speakerNoiseReduce.value,
      always_on_top: newValue,
    }
    config.always_on_top = newValue
    saveAppConfig(config)
  } catch (e) {
    console.error('[SettingsPage] Error setting always on top:', e)
  } finally {
    alwaysOnTopLoading.value = false
  }
}

async function toggleSrtp() {
  srtpLoading.value = true
  const newEnabled = !preferSrtp.value

  try {
    await invoke('set_prefer_srtp', { enabled: newEnabled })
    preferSrtp.value = newEnabled

    // 保存到 localStorage
    const config = getAppConfig() || {
      sip_flow: { enabled: sipFlowEnabled.value, log_dir: sipFlowDir.value },
      prefer_srtp: newEnabled,
      noise_reduce: noiseReduce.value,
      speaker_noise_reduce: speakerNoiseReduce.value,
    }
    config.prefer_srtp = newEnabled
    saveAppConfig(config)

    console.log('[SettingsPage] SRTP preference updated and saved:', newEnabled)
  } catch (e) {
    console.error('[SettingsPage] Error setting SRTP preference:', e)
  } finally {
    srtpLoading.value = false
  }
}

async function selectLogFolder() {
  console.log('[SettingsPage] selectLogFolder called')
  try {
    console.log('[SettingsPage] Opening folder selection dialog...')
    const selected = await open({
      directory: true,
      multiple: false,
      title: '选择 SIP 日志存储目录',
    })
    console.log('[SettingsPage] Dialog result:', selected)

    if (selected) {
      console.log('[SettingsPage] Invoking set_sip_flow_dir with:', selected)
      await invoke('set_sip_flow_dir', { dir: selected })
      sipFlowDir.value = selected

      // 保存到 localStorage
      const config = getAppConfig() || {
        sip_flow: { enabled: sipFlowEnabled.value, log_dir: selected },
        prefer_srtp: preferSrtp.value,
        noise_reduce: noiseReduce.value,
        speaker_noise_reduce: speakerNoiseReduce.value,
      }
      config.sip_flow.log_dir = selected
      saveAppConfig(config)
      console.log('[SettingsPage] Config saved with new dir')

      console.log('[SettingsPage] 日志目录已更新')
    } else {
      console.log('[SettingsPage] User cancelled folder selection')
    }
  } catch (e) {
    console.error('[SettingsPage] Error in selectLogFolder:', e)
  }
}

async function handleRefreshDevices() {
  await audio.enumerateDevices()
  console.log('[SettingsPage] 设备列表已刷新')
}

function handleBack() {
  router.back()
}
</script>

<template>
  <div class="h-screen overflow-y-auto p-4">
    <div class="max-w-2xl mx-auto space-y-4">
      <!-- 头部 -->
      <div class="flex items-center gap-2">
        <Button variant="ghost" size="sm" @click="handleBack">
          <ArrowLeft class="h-4 w-4" />
        </Button>
        <h1 class="text-2xl font-bold">设置</h1>
      </div>

      <!-- 音频设备设置 -->
      <Card>
        <CardHeader>
          <div class="flex items-center justify-between">
            <div>
              <CardTitle>音频设备</CardTitle>
              <CardDescription>选择麦克风和扬声器设备</CardDescription>
            </div>
            <Button variant="ghost" size="sm" @click="handleRefreshDevices">
              <RefreshCw class="h-4 w-4" />
            </Button>
          </div>
        </CardHeader>
        <CardContent class="space-y-4">
          <div class="grid grid-cols-2 gap-3">
            <!-- 麦克风 -->
            <div class="space-y-2 min-w-0">
              <Label>麦克风</Label>
              <Select
                :model-value="audio.selectedMic.value"
                @update:model-value="v => audio.setMic(v as string | null)"
              >
                <SelectTrigger class="w-full">
                  <SelectValue>
                    <span class="truncate block">
                      {{ audio.microphones.value.find(d => d.name === audio.selectedMic.value)?.description || '选择麦克风' }}
                    </span>
                  </SelectValue>
                </SelectTrigger>
                <SelectContent>
                  <SelectItem
                    v-for="device in audio.microphones.value"
                    :key="device.name"
                    :value="device.name"
                  >
                    {{ device.description }}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>

            <!-- 扬声器 -->
            <div class="space-y-2 min-w-0">
              <Label>扬声器</Label>
              <Select
                :model-value="audio.selectedSpeaker.value"
                @update:model-value="v => audio.setSpeaker(v as string | null)"
              >
                <SelectTrigger class="w-full">
                  <SelectValue>
                    <span class="truncate block">
                      {{ audio.speakers.value.find(d => d.name === audio.selectedSpeaker.value)?.description || '选择扬声器' }}
                    </span>
                  </SelectValue>
                </SelectTrigger>
                <SelectContent>
                  <SelectItem
                    v-for="device in audio.speakers.value"
                    :key="device.name"
                    :value="device.name"
                  >
                    {{ device.description }}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>

          <div v-if="audio.deviceError.value" class="text-sm text-destructive">
            {{ audio.deviceError.value }}
          </div>
          <div v-if="!audio.microphones.value.length" class="text-sm text-muted-foreground">
            未检测到音频设备
          </div>
        </CardContent>
      </Card>

      <!-- 通话设置 -->
      <Card>
        <CardHeader>
          <CardTitle>通话设置</CardTitle>
          <CardDescription>控制通话加密和媒体传输</CardDescription>
        </CardHeader>
        <CardContent class="space-y-4">
          <div class="flex items-center justify-between">
            <div class="space-y-0.5">
              <Label>优先使用 SRTP</Label>
              <p class="text-sm text-muted-foreground">
                启用后优先尝试加密的媒体传输，若服务器不支持则自动降级为 RTP
              </p>
            </div>
            <Button
              :variant="preferSrtp ? 'default' : 'outline'"
              size="sm"
              @click="toggleSrtp"
              :disabled="srtpLoading"
            >
              {{ preferSrtp ? '已开启' : '已关闭' }}
            </Button>
          </div>

          <div class="flex items-center justify-between">
            <div class="space-y-0.5">
              <Label>麦克风降噪</Label>
              <p class="text-sm text-muted-foreground">
                使用 RNNoise 神经网络算法实时过滤背景噪音，通话中可随时切换
              </p>
            </div>
            <Button
              :variant="noiseReduce ? 'default' : 'outline'"
              size="sm"
              @click="toggleNoiseReduce"
              :disabled="noiseReduceLoading"
            >
              {{ noiseReduce ? '已开启' : '已关闭' }}
            </Button>
          </div>

          <div class="flex items-center justify-between">
            <div class="space-y-0.5">
              <Label>扬声器降噪</Label>
              <p class="text-sm text-muted-foreground">
                使用 RNNoise 神经网络算法过滤通话对端的背景噪音，通话中可随时切换
              </p>
            </div>
            <Button
              :variant="speakerNoiseReduce ? 'default' : 'outline'"
              size="sm"
              @click="toggleSpeakerNoiseReduce"
              :disabled="speakerNoiseReduceLoading"
            >
              {{ speakerNoiseReduce ? '已开启' : '已关闭' }}
            </Button>
          </div>
        </CardContent>
      </Card>

      <!-- 界面设置 -->
      <Card>
        <CardHeader>
          <CardTitle>界面设置</CardTitle>
          <CardDescription>控制窗口显示行为</CardDescription>
        </CardHeader>
        <CardContent class="space-y-4">
          <div class="flex items-center justify-between">
            <div class="space-y-0.5">
              <Label>始终在最前端</Label>
              <p class="text-sm text-muted-foreground">
                窗口将浮动于所有其他应用之上，方便通话时快速操作
              </p>
            </div>
            <Button
              :variant="alwaysOnTop ? 'default' : 'outline'"
              size="sm"
              @click="toggleAlwaysOnTop"
              :disabled="alwaysOnTopLoading"
            >
              {{ alwaysOnTop ? '已开启' : '已关闭' }}
            </Button>
          </div>
        </CardContent>
      </Card>

      <!-- 日志设置 -->
      <Card>
        <CardHeader>
          <CardTitle>日志设置</CardTitle>
          <CardDescription>控制 SIP 消息流日志记录</CardDescription>
        </CardHeader>
        <CardContent class="space-y-4">
          <div class="flex items-center justify-between">
            <div class="space-y-0.5">
              <Label>SIP 消息日志</Label>
              <p class="text-sm text-muted-foreground">
                记录所有 SIP 消息到日志文件（包括注册过程）
              </p>
            </div>
            <Button
              :variant="sipFlowEnabled ? 'default' : 'outline'"
              size="sm"
              @click="toggleSipFlow"
              :disabled="sipFlowLoading"
            >
              {{ sipFlowEnabled ? '已开启' : '已关闭' }}
            </Button>
          </div>

          <!-- 日志目录选择 -->
          <div class="space-y-2">
            <Label>日志存储目录</Label>
            <div class="flex gap-2">
              <div class="flex-1 px-3 py-2 text-sm border rounded-md bg-muted/50 truncate">
                {{ sipFlowDir || '未设置' }}
              </div>
              <Button variant="outline" size="sm" @click="selectLogFolder">
                <FolderOpen class="h-4 w-4 mr-2" />
                选择
              </Button>
            </div>
            <p class="text-xs text-muted-foreground">
              日志文件将保存为：{{ sipFlowDir }}/sip-flow.log
            </p>
            <p v-if="!isRegistered" class="text-xs text-amber-600">
              💡 建议在注册前开启日志，以便记录注册过程的 SIP 消息
            </p>
          </div>
        </CardContent>
      </Card>
    </div>
  </div>
</template>
