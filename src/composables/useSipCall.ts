import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useWebRTC } from './useWebRTC'

export type CallState = 'idle' | 'calling' | 'ringing' | 'connected' | 'incoming' | 'ended'

export interface IncomingCallPayload {
  call_id: string
  caller: string
  callee?: string
}

const callState = ref<CallState>('idle')
const callee = ref('')
const incomingCall = ref<IncomingCallPayload | null>(null)
const error = ref<string | null>(null)

let unlistenCallState: (() => void) | null = null
let unlistenIncoming: (() => void) | null = null

async function setupListeners() {
  if (!unlistenCallState) {
    unlistenCallState = await listen<{ state: string; call_id?: string; reason?: string }>(
      'sip://call-state',
      (event) => {
        console.debug('[Call] call-state event:', event.payload)
        const s = event.payload.state
        if (s === 'calling' || s === 'ringing' || s === 'connected' || s === 'ended' || s === 'incoming') {
          callState.value = s as CallState
        }
        if (s === 'ended') {
          // Clear incoming call state
          incomingCall.value = null
          setTimeout(() => {
            if (callState.value === 'ended') {
              callState.value = 'idle'
            }
          }, 2000)
        }
      }
    )
  }

  if (!unlistenIncoming) {
    unlistenIncoming = await listen<IncomingCallPayload>(
      'sip://incoming-call',
      (event) => {
        console.debug('[Call] incoming-call event:', event.payload)
        incomingCall.value = event.payload
        callState.value = 'incoming'
      }
    )
  }
}

export function useSipCall() {
  const webrtc = useWebRTC()
  setupListeners()

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
      incomingCall.value = null
    }
  }

  async function answerCall() {
    if (!incomingCall.value) {
      console.error('[Call] No incoming call to answer')
      return
    }

    console.debug('[Call] Answering call:', incomingCall.value.call_id)
    try {
      await invoke('sip_answer_call', { callId: incomingCall.value.call_id })
      console.debug('[Call] Call answered')
    } catch (e) {
      error.value = String(e)
      console.error('[Call] Answer failed:', e)
      throw e
    }
  }

  async function rejectCall(reason?: number) {
    if (!incomingCall.value) {
      console.error('[Call] No incoming call to reject')
      return
    }

    console.debug('[Call] Rejecting call:', incomingCall.value.call_id, 'reason:', reason)
    try {
      await invoke('sip_reject_call', {
        callId: incomingCall.value.call_id,
        reason: reason || 486
      })
      console.debug('[Call] Call rejected')
      incomingCall.value = null
      callState.value = 'idle'
    } catch (e) {
      error.value = String(e)
      console.error('[Call] Reject failed:', e)
      throw e
    }
  }

  return {
    callState,
    callee,
    incomingCall,
    error,
    dial,
    hangup,
    answerCall,
    rejectCall,
    webrtc,
  }
}
