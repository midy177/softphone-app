import { invoke } from '@tauri-apps/api/core'

const SIP_FLOW_CONFIG_KEY = 'sip-flow-config'

interface SipFlowConfig {
  enabled: boolean
  log_dir: string
}

/**
 * 在应用启动时恢复 SIP Flow 配置
 * 应该在 main.ts 或 App.vue 中调用
 */
export async function restoreSipFlowConfig() {
  try {
    const cached = localStorage.getItem(SIP_FLOW_CONFIG_KEY)
    if (cached) {
      const config = JSON.parse(cached) as SipFlowConfig
      console.log('[Config] Restoring SIP flow config:', config)

      // 应用到后端
      await invoke('set_sip_flow_enabled', { enabled: config.enabled })
      await invoke('set_sip_flow_dir', { dir: config.log_dir })

      console.log('[Config] SIP flow config restored successfully')
      return true
    }
  } catch (e) {
    console.error('[Config] Failed to restore SIP flow config:', e)
  }
  return false
}

/**
 * 保存 SIP Flow 配置到 localStorage
 */
export function saveSipFlowConfig(config: SipFlowConfig) {
  localStorage.setItem(SIP_FLOW_CONFIG_KEY, JSON.stringify(config))
  console.log('[Config] SIP flow config saved:', config)
}

/**
 * 获取保存的 SIP Flow 配置
 */
export function getSavedSipFlowConfig(): SipFlowConfig | null {
  try {
    const cached = localStorage.getItem(SIP_FLOW_CONFIG_KEY)
    if (cached) {
      return JSON.parse(cached) as SipFlowConfig
    }
  } catch (e) {
    console.error('[Config] Failed to get saved config:', e)
  }
  return null
}
