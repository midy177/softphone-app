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
    }
  } catch {
    // ignore
  }
}

function saveDevices() {
  localStorage.setItem(
    DEVICE_STORAGE_KEY,
    JSON.stringify({ mic: selectedMic.value, speaker: selectedSpeaker.value })
  )
}

export function useWebRTC() {
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

      if (microphones.value.length > 0) {
        const saved = microphones.value.find((d) => d.name === selectedMic.value)
        if (!saved) selectedMic.value = microphones.value[0]!.name
      }
      if (speakers.value.length > 0) {
        const saved = speakers.value.find((d) => d.name === selectedSpeaker.value)
        if (!saved) selectedSpeaker.value = speakers.value[0]!.name
      }
      console.debug('[Audio] Selected mic:', selectedMic.value, '| speaker:', selectedSpeaker.value)
    } catch (e) {
      deviceError.value = `设备枚举失败: ${e}`
      console.error('[Audio] enumerate_audio_devices failed:', e)
    }
  }

  async function setMic(name: string) {
    console.debug('[Audio] Set mic:', name)
    selectedMic.value = name
    saveDevices()
    await invoke('set_input_device', { name })
  }

  async function setSpeaker(name: string) {
    console.debug('[Audio] Set speaker:', name)
    selectedSpeaker.value = name
    saveDevices()
    await invoke('set_output_device', { name })
  }

  async function toggleMicMute() {
    try {
      const muted = await invoke<boolean>('toggle_mic_mute')
      isMicMuted.value = muted
      console.debug('[Audio] Mic muted:', muted)
    } catch (e) {
      console.error('[Audio] toggle_mic_mute failed:', e)
    }
  }

  async function toggleSpeakerMute() {
    try {
      const muted = await invoke<boolean>('toggle_speaker_mute')
      isSpeakerMuted.value = muted
      console.debug('[Audio] Speaker muted:', muted)
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
