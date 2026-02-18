mod logging;
mod sip;
mod webrtc;

use rustls;
use sip::state::SipAppState;
use tauri::State;
use tracing::error;

// ── Audio device enumeration via cpal ──

#[derive(serde::Serialize)]
struct AudioDevice {
    name: String,
    description: String,
}

#[derive(serde::Serialize)]
struct AudioDevices {
    inputs: Vec<AudioDevice>,
    outputs: Vec<AudioDevice>,
}

/// Linux-specific: Query PulseAudio/PipeWire for friendly device names.
/// Returns maps keyed by "card_name:device_num" (compound) and "card_name" (fallback).
#[cfg(target_os = "linux")]
fn get_pulse_friendly_names() -> (
    std::collections::HashMap<String, String>,
    std::collections::HashMap<String, String>,
) {
    use pulsectl::controllers::{DeviceControl, SinkController, SourceController};
    use std::collections::HashMap;
    use tracing::{debug, info, warn};

    let mut input_map = HashMap::new();
    let mut output_map = HashMap::new();

    // Helper: insert both compound key "card:dev" and fallback key "card"
    fn insert_device(
        map: &mut HashMap<String, String>,
        card: &str,
        dev: Option<String>,
        friendly: String,
    ) {
        if let Some(ref d) = dev {
            map.insert(format!("{}:{}", card, d), friendly.clone());
        }
        // Fallback: card-only key (first one wins)
        map.entry(card.to_string()).or_insert(friendly);
    }

    // Get input devices (sources) via SourceController
    match SourceController::create() {
        Ok(mut source_ctrl) => {
            if let Ok(sources) = source_ctrl.list_devices() {
                for source in sources {
                    // Skip monitor sources (loopback of output, not real microphones)
                    if source.monitor.is_some() {
                        continue;
                    }

                    let card_name = source
                        .proplist
                        .get_str("alsa.card_name")
                        .or_else(|| source.proplist.get_str("device.product.name"));
                    let device_num = source.proplist.get_str("alsa.device");

                    if let Some(card) = card_name {
                        let friendly_name = source
                            .description
                            .unwrap_or_else(|| source.name.unwrap_or_default());
                        debug!(card = %card, dev = ?device_num, friendly = %friendly_name, "PulseAudio source");
                        insert_device(&mut input_map, &card, device_num, friendly_name);
                    }
                }
            }
        }
        Err(e) => {
            warn!(error = %e, "Failed to connect to PulseAudio SourceController");
        }
    }

    // Get output devices (sinks) via SinkController
    match SinkController::create() {
        Ok(mut sink_ctrl) => {
            if let Ok(sinks) = sink_ctrl.list_devices() {
                for sink in sinks {
                    let card_name = sink
                        .proplist
                        .get_str("alsa.card_name")
                        .or_else(|| sink.proplist.get_str("device.product.name"));
                    let device_num = sink.proplist.get_str("alsa.device");

                    if let Some(card) = card_name {
                        let friendly_name = sink
                            .description
                            .unwrap_or_else(|| sink.name.unwrap_or_default());
                        debug!(card = %card, dev = ?device_num, friendly = %friendly_name, "PulseAudio sink");
                        insert_device(&mut output_map, &card, device_num, friendly_name);
                    }
                }
            }
        }
        Err(e) => {
            warn!(error = %e, "Failed to connect to PulseAudio SinkController");
        }
    }

    info!(input_keys = ?input_map.keys().collect::<Vec<_>>(), output_keys = ?output_map.keys().collect::<Vec<_>>(), "PulseAudio friendly name map loaded");

    (input_map, output_map)
}

/// Extract card name from ALSA description (e.g., "HDA Intel PCH, ALC897 Analog" → "HDA Intel PCH").
#[cfg(target_os = "linux")]
fn extract_alsa_card_name(alsa_desc: &str) -> Option<String> {
    alsa_desc.split(',').next().map(|s| s.trim().to_string())
}

/// Extract DEV number from cpal local ID (e.g., "plughw:CARD=PCH,DEV=0" → "0").
#[cfg(target_os = "linux")]
fn extract_dev_number(local_id: &str) -> Option<String> {
    local_id
        .split("DEV=")
        .nth(1)
        .map(|s| s.split(&[',', ':']).next().unwrap_or(s).to_string())
}

#[tauri::command]
fn enumerate_audio_devices() -> Result<AudioDevices, String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    use tracing::warn;

    // 枚举音频设备（不打印日志）

    // On Linux, get friendly names from PulseAudio/PipeWire
    #[cfg(target_os = "linux")]
    let (pulse_input_map, pulse_output_map) = get_pulse_friendly_names();

    let host = cpal::default_host();

    let devices = host
        .devices()
        .map_err(|e| format!("Failed to enumerate devices: {}", e))?;

    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let mut _skipped_count = 0;

    for device in devices {
        let id = match device.id() {
            Ok(id) => id.to_string(),
            Err(e) => {
                warn!(error = ?e, "Failed to get device ID");
                continue;
            }
        };

        // Extract the backend-local part of the ID (e.g. "alsa:plughw:CARD=PCH,DEV=0" → "plughw:CARD=PCH,DEV=0")
        let local_id = id.split_once(':').map(|(_, rest)| rest).unwrap_or(&id);

        // Only keep useful devices, skip ALSA virtual plugins
        if !is_useful_device(local_id) {
            _skipped_count += 1;
            continue;
        }

        // Get the base description from cpal
        let cpal_desc = device
            .description()
            .map(|d| d.to_string())
            .unwrap_or_else(|_| id.clone());

        // On Linux, try to find a friendly name from PulseAudio
        // Devices without a PulseAudio match (except "default") are skipped — no active hardware
        #[cfg(target_os = "linux")]
        let (input_desc, output_desc) = {
            if local_id == "default" {
                (Some(cpal_desc.clone()), Some(cpal_desc))
            } else if let Some(card_name) = extract_alsa_card_name(&cpal_desc) {
                let dev_num = extract_dev_number(local_id);
                let compound_key = dev_num.as_ref().map(|d| format!("{}:{}", card_name, d));

                let resolve = |map: &std::collections::HashMap<String, String>| -> Option<String> {
                    compound_key
                        .as_ref()
                        .and_then(|k| map.get(k))
                        .or_else(|| map.get(&card_name))
                        .cloned()
                };

                (resolve(&pulse_input_map), resolve(&pulse_output_map))
            } else {
                (None, None)
            }
        };

        #[cfg(not(target_os = "linux"))]
        let (input_desc, output_desc) = (Some(cpal_desc.clone()), Some(cpal_desc));

        let has_input = device.default_input_config().is_ok();
        let has_output = device.default_output_config().is_ok();

        if has_input {
            if let Some(desc) = input_desc {
                inputs.push(AudioDevice {
                    name: id.clone(),
                    description: desc,
                });
            }
        }
        if has_output {
            if let Some(desc) = output_desc {
                outputs.push(AudioDevice {
                    name: id,
                    description: desc,
                });
            }
        }
    }

    // 只在有错误时打印日志
    // info!(
    //     skipped = skipped_count,
    //     inputs = inputs.len(),
    //     outputs = outputs.len(),
    //     "Device enumeration complete"
    // );
    // for d in &inputs {
    //     info!(name = %d.name, desc = %d.description, "Input device");
    // }
    // for d in &outputs {
    //     info!(name = %d.name, desc = %d.description, "Output device");
    // }

    Ok(AudioDevices { inputs, outputs })
}

/// Filter out ALSA virtual plugins and duplicates, only keep real/useful devices.
/// Keeps: `default`, `plughw:CARD=<name>` (by card name, not number to deduplicate).
/// Skips: pipewire, pulse, sysdefault (redundant with default), raw hw:, all virtual plugins.
/// On non-Linux platforms, accepts all devices.
fn is_useful_device(_local_id: &str) -> bool {
    // On macOS/Windows, accept all devices (no filtering needed)
    #[cfg(not(target_os = "linux"))]
    {
        return true;
    }

    // On Linux, apply ALSA-specific filtering
    #[cfg(target_os = "linux")]
    {
        if _local_id == "default" {
            return true;
        }
        if let Some(rest) = _local_id.strip_prefix("plughw:") {
            // Only keep CARD=<name> form, skip CARD=<number> (duplicate)
            if let Some(card_val) = rest.strip_prefix("CARD=") {
                let card_name = card_val.split(&[',', ':']).next().unwrap_or("");
                return !card_name.chars().all(|c| c.is_ascii_digit());
            }
        }
        false
    }
}

// ── SIP commands ──

#[tauri::command]
async fn sip_is_registered(state: State<'_, SipAppState>) -> Result<bool, String> {
    Ok(state.handle.lock().await.is_some())
}

#[tauri::command]
async fn sip_register(
    state: State<'_, SipAppState>,
    app_handle: tauri::AppHandle,
    server: String,
    username: String,
    password: String,
    outbound_proxy: Option<String>,
) -> Result<(), String> {
    if state.handle.lock().await.is_some() {
        return Err("Already registered".to_string());
    }

    // 获取 SIP flow 配置
    let sip_flow_config = state.sip_flow_config.lock().await.clone();

    match sip::SipClient::connect(
        app_handle,
        server,
        username,
        password,
        outbound_proxy,
        Some(sip_flow_config.enabled),
        Some(sip_flow_config.log_dir),
    )
    .await
    {
        Ok((new_handle, cancel_token)) => {
            *state.handle.lock().await = Some(std::sync::Arc::new(new_handle));
            *state.cancel_token.lock().await = Some(cancel_token);
            Ok(())
        }
        Err(e) => {
            error!(error = ?e, "SIP registration failed");
            Err(format!("Registration failed: {}", e))
        }
    }
}

#[tauri::command]
async fn sip_unregister(state: State<'_, SipAppState>) -> Result<(), String> {
    // Cancel global token - this will cascade to all child tokens (active calls)
    if let Some(token) = state.cancel_token.lock().await.take() {
        token.cancel();
        // Give child tokens time to propagate cancellation and clean up
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    state.handle.lock().await.take();
    Ok(())
}

#[tauri::command]
async fn sip_make_call(state: State<'_, SipAppState>, callee: String) -> Result<(), String> {
    let input_device = state.input_device.lock().await.clone();
    let output_device = state.output_device.lock().await.clone();

    // Clone Arc<SipClientHandle> and release the lock immediately
    // so that sip_hangup can also acquire the lock concurrently
    let handle = {
        let handle_guard = state.handle.lock().await;
        handle_guard
            .as_ref()
            .ok_or_else(|| "Not registered".to_string())?
            .clone()
    };

    let cancel_token = state
        .cancel_token
        .lock()
        .await
        .as_ref()
        .ok_or_else(|| "No cancel token available".to_string())?
        .clone();

    sip::handle_make_call(&handle, callee, input_device, output_device, cancel_token)
        .await
        .map_err(|e| {
            error!(error = ?e, "Make call failed");
            format!("Call failed: {}", e)
        })
}

#[tauri::command]
async fn sip_hangup(state: State<'_, SipAppState>) -> Result<(), String> {
    let handle = {
        let handle_guard = state.handle.lock().await;
        handle_guard
            .as_ref()
            .ok_or_else(|| "Not registered".to_string())?
            .clone()
    };

    sip::handle_hangup(&handle).await.map_err(|e| {
        error!(error = ?e, "Hangup failed");
        format!("Hangup failed: {}", e)
    })
}

#[tauri::command]
async fn sip_answer_call(state: State<'_, SipAppState>, call_id: String) -> Result<(), String> {
    let input_device = state.input_device.lock().await.clone();
    let output_device = state.output_device.lock().await.clone();

    let handle = {
        let handle_guard = state.handle.lock().await;
        handle_guard
            .as_ref()
            .ok_or_else(|| "Not registered".to_string())?
            .clone()
    };

    let cancel_token = state
        .cancel_token
        .lock()
        .await
        .as_ref()
        .ok_or_else(|| "No cancel token available".to_string())?
        .clone();

    sip::handle_answer_call(&handle, call_id, input_device, output_device, cancel_token)
        .await
        .map_err(|e| {
            error!(error = ?e, "Answer call failed");
            format!("Answer failed: {}", e)
        })
}

#[tauri::command]
async fn sip_reject_call(
    state: State<'_, SipAppState>,
    call_id: String,
    reason: Option<u16>,
) -> Result<(), String> {
    let handle = {
        let handle_guard = state.handle.lock().await;
        handle_guard
            .as_ref()
            .ok_or_else(|| "Not registered".to_string())?
            .clone()
    };

    sip::handle_reject_call(&handle, call_id, reason)
        .await
        .map_err(|e| {
            error!(error = ?e, "Reject call failed");
            format!("Reject failed: {}", e)
        })
}

// ── Audio device commands ──

#[tauri::command]
async fn set_input_device(state: State<'_, SipAppState>, name: String) -> Result<(), String> {
    *state.input_device.lock().await = Some(name);
    Ok(())
}

#[tauri::command]
async fn set_output_device(state: State<'_, SipAppState>, name: String) -> Result<(), String> {
    *state.output_device.lock().await = Some(name);
    Ok(())
}

#[tauri::command]
async fn toggle_mic_mute(state: State<'_, SipAppState>) -> Result<bool, String> {
    let handle = {
        let handle_guard = state.handle.lock().await;
        handle_guard
            .as_ref()
            .ok_or_else(|| "Not registered".to_string())?
            .clone()
    };

    sip::handle_toggle_mic_mute(&handle).await
}

#[tauri::command]
async fn toggle_speaker_mute(state: State<'_, SipAppState>) -> Result<bool, String> {
    let handle = {
        let handle_guard = state.handle.lock().await;
        handle_guard
            .as_ref()
            .ok_or_else(|| "Not registered".to_string())?
            .clone()
    };

    sip::handle_toggle_speaker_mute(&handle).await
}

#[tauri::command]
async fn send_dtmf(state: State<'_, SipAppState>, digit: String) -> Result<(), String> {
    let handle = {
        let handle_guard = state.handle.lock().await;
        handle_guard
            .as_ref()
            .ok_or_else(|| "Not registered".to_string())?
            .clone()
    };

    sip::handle_send_dtmf(&handle, digit).await
}

// ── SIP Flow 配置命令（统一接口，支持注册前后使用）──

/// 设置 SIP 消息日志开关
#[tauri::command]
async fn set_sip_flow_enabled(state: State<'_, SipAppState>, enabled: bool) -> Result<(), String> {
    // 更新配置
    state.sip_flow_config.lock().await.enabled = enabled;

    // 如果已注册，同时更新运行中的实例
    let handle_guard = state.handle.lock().await;
    if let Some(handle) = handle_guard.as_ref() {
        if enabled {
            sip::handle_enable_sip_flow(handle)?;
        } else {
            sip::handle_disable_sip_flow(handle)?;
        }
    }    Ok(())
}

/// 设置 SIP 消息日志目录
#[tauri::command]
async fn set_sip_flow_dir(state: State<'_, SipAppState>, dir: String) -> Result<(), String> {
    // 更新配置
    state.sip_flow_config.lock().await.log_dir = dir.clone();

    // 如果已注册，同时更新运行中的实例
    let handle_guard = state.handle.lock().await;
    if let Some(handle) = handle_guard.as_ref() {
        sip::handle_set_sip_flow_dir(handle, dir)?;
    }

    Ok(())
}

/// 获取 SIP 消息日志配置
#[tauri::command]
async fn get_sip_flow_config(
    state: State<'_, SipAppState>,
) -> Result<sip::state::SipFlowConfig, String> {
    // 优先从已注册的 handle 获取实际运行状态
    let handle_guard = state.handle.lock().await;
    if let Some(handle) = handle_guard.as_ref() {
        let enabled = sip::handle_is_sip_flow_enabled(handle)?;
        let log_dir = sip::handle_get_sip_flow_dir(handle)?;
        Ok(sip::state::SipFlowConfig { enabled, log_dir })
    } else {
        // 否则返回配置
        Ok(state.sip_flow_config.lock().await.clone())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Install ring as the default rustls CryptoProvider before any TLS operations.
    // Required in rustls 0.23+ when multiple crypto features could be available.
    let _ = rustls::crypto::ring::default_provider().install_default();

    logging::initialize_logging("info", true);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(SipAppState {
            handle: tokio::sync::Mutex::new(None),
            cancel_token: tokio::sync::Mutex::new(None),
            input_device: tokio::sync::Mutex::new(None),
            output_device: tokio::sync::Mutex::new(None),
            sip_flow_config: tokio::sync::Mutex::new(sip::state::SipFlowConfig::default()),
        })
        .invoke_handler(tauri::generate_handler![
            enumerate_audio_devices,
            sip_is_registered,
            sip_register,
            sip_unregister,
            sip_make_call,
            sip_hangup,
            sip_answer_call,
            sip_reject_call,
            set_input_device,
            set_output_device,
            toggle_mic_mute,
            toggle_speaker_mute,
            send_dtmf,
            set_sip_flow_enabled,
            set_sip_flow_dir,
            get_sip_flow_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
