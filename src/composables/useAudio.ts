import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

export interface MediaDevice {
  name: string
  description: string
}

interface AudioDevices {
  inputs: MediaDevice[]
  outputs: MediaDevice[]
}

const isMicMuted = ref(false)
const isSpeakerMuted = ref(false)
const microphones = ref<MediaDevice[]>([])
const speakers = ref<MediaDevice[]>([])
const selectedMic = ref<string>('')
const selectedSpeaker = ref<string>('')
const deviceError = ref<string | null>(null)

const DEVICE_STORAGE_KEY = 'sip-devices'

function loadSavedDevices() {
  try {
    const raw = localStorage.getItem(DEVICE_STORAGE_KEY)
    if (raw) {
      const saved = JSON.parse(raw)
      if (saved.mic) selectedMic.value = saved.mic
      if (saved.speaker) selectedSpeaker.value = saved.speaker
      console.debug('[Audio] Loaded saved devices:', saved)
      return saved
    }
  } catch (e) {
    console.error('[Audio] Failed to load saved devices:', e)
  }
  return null
}

function saveDevices() {
  const config = { mic: selectedMic.value, speaker: selectedSpeaker.value }
  localStorage.setItem(DEVICE_STORAGE_KEY, JSON.stringify(config))
  console.debug('[Audio] Saved devices:', config)
}

export function useAudio() {
  // 加载保存的设备配置
  loadSavedDevices()

  async function enumerateDevices() {
    deviceError.value = null
    console.debug('[Audio] Enumerating audio devices via cpal...')
    try {
      const devices = await invoke<AudioDevices>('enumerate_audio_devices')
      microphones.value = devices.inputs
      speakers.value = devices.outputs
      console.debug('[Audio] Inputs:', devices.inputs.length, devices.inputs)
      console.debug('[Audio] Outputs:', devices.outputs.length, devices.outputs)

      // 优先使用保存的设备，如果不存在则使用第一个
      if (microphones.value.length > 0) {
        const savedMic = microphones.value.find((d) => d.name === selectedMic.value)
        if (savedMic) {
          selectedMic.value = savedMic.name
          // 自动应用到后端
          await invoke('set_input_device', { name: savedMic.name })
          console.debug('[Audio] Auto-applied saved microphone:', savedMic.description)
        } else {
          selectedMic.value = microphones.value[0]!.name
          await invoke('set_input_device', { name: microphones.value[0]!.name })
          console.debug('[Audio] Using default microphone:', microphones.value[0]!.description)
        }
      }

      if (speakers.value.length > 0) {
        const savedSpeaker = speakers.value.find((d) => d.name === selectedSpeaker.value)
        if (savedSpeaker) {
          selectedSpeaker.value = savedSpeaker.name
          // 自动应用到后端
          await invoke('set_output_device', { name: savedSpeaker.name })
          console.debug('[Audio] Auto-applied saved speaker:', savedSpeaker.description)
        } else {
          selectedSpeaker.value = speakers.value[0]!.name
          await invoke('set_output_device', { name: speakers.value[0]!.name })
          console.debug('[Audio] Using default speaker:', speakers.value[0]!.description)
        }
      }

      // 保存当前选择（可能是新的默认设备）
      saveDevices()

      console.debug('[Audio] Selected mic:', selectedMic.value, '| speaker:', selectedSpeaker.value)
    } catch (e) {
      deviceError.value = `设备枚举失败: ${e}`
      console.error('[Audio] enumerate_audio_devices failed:', e)
    }
  }

  async function setMic(name: string | null) {
    if (!name) return
    console.debug('[Audio] Set mic:', name)
    selectedMic.value = name
    saveDevices()
    try {
      await invoke('set_input_device', { name })
      console.debug('[Audio] Microphone applied to backend:', name)
    } catch (e) {
      console.error('[Audio] Failed to set input device:', e)
    }
  }

  async function setSpeaker(name: string | null) {
    if (!name) return
    console.debug('[Audio] Set speaker:', name)
    selectedSpeaker.value = name
    saveDevices()
    try {
      await invoke('set_output_device', { name })
      console.debug('[Audio] Speaker applied to backend:', name)
    } catch (e) {
      console.error('[Audio] Failed to set output device:', e)
    }
  }

  async function toggleMicMute() {
    console.log('[Audio] toggleMicMute called, current state:', isMicMuted.value)
    try {
      console.log('[Audio] Invoking toggle_mic_mute...')
      const muted = await invoke<boolean>('toggle_mic_mute')
      console.log('[Audio] Backend returned:', muted)
      isMicMuted.value = muted
      console.log('[Audio] Mic muted state updated to:', muted)
    } catch (e) {
      console.error('[Audio] toggle_mic_mute failed:', e)
    }
  }

  async function toggleSpeakerMute() {
    console.log('[Audio] toggleSpeakerMute called, current state:', isSpeakerMuted.value)
    try {
      console.log('[Audio] Invoking toggle_speaker_mute...')
      const muted = await invoke<boolean>('toggle_speaker_mute')
      console.log('[Audio] Backend returned:', muted)
      isSpeakerMuted.value = muted
      console.log('[Audio] Speaker muted state updated to:', muted)
    } catch (e) {
      console.error('[Audio] toggle_speaker_mute failed:', e)
    }
  }

  return {
    isMicMuted,
    isSpeakerMuted,
    microphones,
    speakers,
    selectedMic,
    selectedSpeaker,
    deviceError,
    enumerateDevices,
    toggleMicMute,
    toggleSpeakerMute,
    setMic,
    setSpeaker,
  }
}
