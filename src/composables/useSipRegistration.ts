import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

const isRegistered = ref(false)
const isRegistering = ref(false)
const error = ref<string | null>(null)

let unlisten: (() => void) | null = null

async function setupListener() {
  if (unlisten) return
  unlisten = await listen<{ status: string; message?: string }>(
    'sip://registration-status',
    (event) => {
      console.debug('[SIP] registration-status event:', event.payload)
      if (event.payload.status === 'registered') {
        isRegistered.value = true
      } else {
        isRegistered.value = false
      }
    }
  )
}

export function useSipRegistration() {
  setupListener()

  async function checkRegistered(): Promise<boolean> {
    const result = await invoke<boolean>('sip_is_registered')
    console.debug('[SIP] checkRegistered:', result)
    isRegistered.value = result
    return result
  }

  async function register(
    server: string,
    username: string,
    password: string,
    outboundProxy?: string
  ) {
    if (isRegistered.value) {
      console.debug('[SIP] Already registered, skipping')
      return
    }
    console.debug('[SIP] Registering:', { server, username, outboundProxy })
    isRegistering.value = true
    error.value = null
    try {
      await invoke('sip_register', {
        server,
        username,
        password,
        outboundProxy: outboundProxy || null,
      })
      isRegistered.value = true
      console.debug('[SIP] Registration successful')
    } catch (e) {
      error.value = String(e)
      isRegistered.value = false
      console.error('[SIP] Registration failed:', e)
      throw e
    } finally {
      isRegistering.value = false
    }
  }

  async function unregister() {
    console.debug('[SIP] Unregistering')
    try {
      await invoke('sip_unregister')
      console.debug('[SIP] Unregistered')
    } catch (e) {
      console.error('[SIP] Unregister failed:', e)
    } finally {
      isRegistered.value = false
    }
  }

  return {
    isRegistered,
    isRegistering,
    error,
    checkRegistered,
    register,
    unregister,
  }
}
