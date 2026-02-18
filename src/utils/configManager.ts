import { invoke } from '@tauri-apps/api/core'

const APP_CONFIG_KEY = 'app-config'

interface AppConfig {
  sip_flow: {
    enabled: boolean
    log_dir: string
  }
  prefer_srtp: boolean
}

// 向后兼容的旧配置 key
const SIP_FLOW_CONFIG_KEY = 'sip-flow-config'

/**
 * 在应用启动时恢复应用配置
 * 应该在 main.ts 或 App.vue 中调用
 */
export async function restoreSipFlowConfig() {
  try {
    // 先尝试读取新的统一配置
    let config = getAppConfig()

    // 如果没有新配置，尝试迁移旧配置
    if (!config) {
      const oldConfig = localStorage.getItem(SIP_FLOW_CONFIG_KEY)
      if (oldConfig) {
        const parsed = JSON.parse(oldConfig)
        config = {
          sip_flow: {
            enabled: parsed.enabled,
            log_dir: parsed.log_dir,
          },
          prefer_srtp: true, // 默认值
        }
        // 保存到新格式
        saveAppConfig(config)
        // 删除旧配置
        localStorage.removeItem(SIP_FLOW_CONFIG_KEY)
        console.log('[Config] Migrated old config to new format')
      }
    }

    if (config) {
      console.log('[Config] Restoring app config:', config)

      // 应用 SIP Flow 配置到后端
      await invoke('set_sip_flow_enabled', { enabled: config.sip_flow.enabled })
      await invoke('set_sip_flow_dir', { dir: config.sip_flow.log_dir })

      // 应用 SRTP 配置到后端
      await invoke('set_prefer_srtp', { enabled: config.prefer_srtp })

      console.log('[Config] App config restored successfully')
      return true
    }
  } catch (e) {
    console.error('[Config] Failed to restore app config:', e)
  }
  return false
}

/**
 * 保存应用配置到 localStorage
 */
export function saveAppConfig(config: AppConfig) {
  localStorage.setItem(APP_CONFIG_KEY, JSON.stringify(config))
  console.log('[Config] App config saved:', config)
}

/**
 * 获取保存的应用配置
 */
export function getAppConfig(): AppConfig | null {
  try {
    const cached = localStorage.getItem(APP_CONFIG_KEY)
    if (cached) {
      return JSON.parse(cached) as AppConfig
    }
  } catch (e) {
    console.error('[Config] Failed to get saved config:', e)
  }
  return null
}

/**
 * 保存 SIP Flow 配置（向后兼容接口）
 * @deprecated 使用 saveAppConfig 替代
 */
export function saveSipFlowConfig(config: { enabled: boolean; log_dir: string }) {
  const appConfig = getAppConfig() || {
    sip_flow: { enabled: false, log_dir: '' },
    prefer_srtp: true,
  }
  appConfig.sip_flow = config
  saveAppConfig(appConfig)
}

/**
 * 获取保存的 SIP Flow 配置（向后兼容接口）
 * @deprecated 使用 getAppConfig 替代
 */
export function getSavedSipFlowConfig(): { enabled: boolean; log_dir: string } | null {
  const config = getAppConfig()
  return config ? config.sip_flow : null
}
