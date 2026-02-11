import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useWebRTC } from './useWebRTC'

export type CallState = 'idle' | 'calling' | 'ringing' | 'connected' | 'ended'

const callState = ref<CallState>('idle')
const callee = ref('')
const error = ref<string | null>(null)

let unlisten: (() => void) | null = null

async function setupListener() {
  if (unlisten) return
  unlisten = await listen<{ state: string; call_id?: string; reason?: string }>(
    'sip://call-state',
    (event) => {
      console.debug('[Call] call-state event:', event.payload)
      const s = event.payload.state
      if (s === 'calling' || s === 'ringing' || s === 'connected' || s === 'ended') {
        callState.value = s as CallState
      }
      if (s === 'ended') {
        setTimeout(() => {
          if (callState.value === 'ended') {
            callState.value = 'idle'
          }
        }, 2000)
      }
    }
  )
}

export function useSipCall() {
  const webrtc = useWebRTC()
  setupListener()

  async function dial(number: string) {
    callee.value = number
    error.value = null
    callState.value = 'calling'
    console.debug('[Call] Dialing:', number)

    try {
      // Rust handles SDP generation, audio setup, and INVITE internally
      await invoke('sip_make_call', { callee: number })
      console.debug('[Call] Call established')
    } catch (e) {
      error.value = String(e)
      callState.value = 'idle'
      console.error('[Call] Dial failed:', e)
      throw e
    }
  }

  async function hangup() {
    console.debug('[Call] Hanging up')
    try {
      await invoke('sip_hangup')
      console.debug('[Call] Hangup sent')
    } catch (e) {
      console.error('[Call] Hangup error:', e)
    } finally {
      callState.value = 'idle'
    }
  }

  return {
    callState,
    callee,
    error,
    dial,
    hangup,
    webrtc,
  }
}
