<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { useSipRegistration } from '@/composables/useSipRegistration'
import { useAudio } from '@/composables/useAudio'
import { getSavedSipFlowConfig, saveSipFlowConfig } from '@/utils/configManager'
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
    const configToSave = { enabled: newEnabled, log_dir: sipFlowDir.value }
    console.log('[SettingsPage] Saving config to localStorage:', configToSave)
    saveSipFlowConfig(configToSave)

    const message = newEnabled ? 'SIP 消息日志已开启' : 'SIP 消息日志已关闭'
    console.log('[SettingsPage]', message)

    // Verify it was saved
    const verified = getSavedSipFlowConfig()
    console.log('[SettingsPage] Verified saved config:', verified)
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
  // 从 localStorage 读取配置（已在 App.vue 中应用到后端）
  const saved = getSavedSipFlowConfig()
  if (saved) {
    sipFlowEnabled.value = saved.enabled
    sipFlowDir.value = saved.log_dir
    console.log('[SettingsPage] Loaded config from localStorage:', saved)
  } else {
    // 如果没有保存的配置，从后端获取默认值
    try {
      const config = await invoke<SipFlowConfig>('get_sip_flow_config')
      sipFlowEnabled.value = config.enabled
      sipFlowDir.value = config.log_dir
      saveSipFlowConfig(config)
      console.log('[SettingsPage] Loaded default config from backend:', config)
    } catch (e) {
      console.error('[SettingsPage] Failed to load config:', e)
    }
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
      const configToSave = { enabled: sipFlowEnabled.value, log_dir: selected }
      console.log('[SettingsPage] Saving config with new dir:', configToSave)
      saveSipFlowConfig(configToSave)

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
