mod logging;
mod sip;
mod webrtc;

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
fn get_pulse_friendly_names() -> (std::collections::HashMap<String, String>, std::collections::HashMap<String, String>) {
    use std::collections::HashMap;
    use pulsectl::controllers::{SinkController, SourceController, DeviceControl};
    use tracing::{debug, info, warn};

    let mut input_map = HashMap::new();
    let mut output_map = HashMap::new();

    // Helper: insert both compound key "card:dev" and fallback key "card"
    fn insert_device(map: &mut HashMap<String, String>, card: &str, dev: Option<String>, friendly: String) {
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

                    let card_name = source.proplist.get_str("alsa.card_name")
                        .or_else(|| source.proplist.get_str("device.product.name"));
                    let device_num = source.proplist.get_str("alsa.device");

                    if let Some(card) = card_name {
                        let friendly_name = source.description.unwrap_or_else(|| {
                            source.name.unwrap_or_default()
                        });
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
                    let card_name = sink.proplist.get_str("alsa.card_name")
                        .or_else(|| sink.proplist.get_str("device.product.name"));
                    let device_num = sink.proplist.get_str("alsa.device");

                    if let Some(card) = card_name {
                        let friendly_name = sink.description.unwrap_or_else(|| {
                            sink.name.unwrap_or_default()
                        });
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
    local_id.split("DEV=").nth(1).map(|s| {
        s.split(&[',', ':']).next().unwrap_or(s).to_string()
    })
}

#[tauri::command]
fn enumerate_audio_devices() -> Result<AudioDevices, String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    use tracing::{info, warn};

    info!("Enumerating audio devices");

    // On Linux, get friendly names from PulseAudio/PipeWire
    #[cfg(target_os = "linux")]
    let (pulse_input_map, pulse_output_map) = get_pulse_friendly_names();

    let host = cpal::default_host();

    let devices = host
        .devices()
        .map_err(|e| format!("Failed to enumerate devices: {}", e))?;

    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let mut skipped_count = 0;

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
            skipped_count += 1;
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
                    compound_key.as_ref()
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

    info!(
        skipped = skipped_count,
        inputs = inputs.len(),
        outputs = outputs.len(),
        "Device enumeration complete"
    );
    for d in &inputs {
        info!(name = %d.name, desc = %d.description, "Input device");
    }
    for d in &outputs {
        info!(name = %d.name, desc = %d.description, "Output device");
    }

    Ok(AudioDevices { inputs, outputs })
}

/// Filter out ALSA virtual plugins and duplicates, only keep real/useful devices.
/// Keeps: `default`, `plughw:CARD=<name>` (by card name, not number to deduplicate).
/// Skips: pipewire, pulse, sysdefault (redundant with default), raw hw:, all virtual plugins.
/// On non-Linux platforms, accepts all devices.
fn is_useful_device(local_id: &str) -> bool {
    // On macOS/Windows, accept all devices (no filtering needed)
    #[cfg(not(target_os = "linux"))]
    {
        return true;
    }

    // On Linux, apply ALSA-specific filtering
    #[cfg(target_os = "linux")]
    {
        if local_id == "default" {
            return true;
        }
        if let Some(rest) = local_id.strip_prefix("plughw:") {
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

    match sip::SipClient::connect(app_handle, server, username, password, outbound_proxy).await {
        Ok((new_handle, cancel_token)) => {
            *state.handle.lock().await = Some(new_handle);
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
    if let Some(token) = state.cancel_token.lock().await.take() {
        token.cancel();
    }
    state.handle.lock().await.take();
    Ok(())
}

#[tauri::command]
async fn sip_make_call(
    state: State<'_, SipAppState>,
    callee: String,
) -> Result<(), String> {
    let input_device = state.input_device.lock().await.clone();
    let output_device = state.output_device.lock().await.clone();

    let handle_guard = state.handle.lock().await;
    let handle = handle_guard
        .as_ref()
        .ok_or_else(|| "Not registered".to_string())?;

    sip::handle_make_call(handle, callee, input_device, output_device)
        .await
        .map_err(|e| {
            error!(error = ?e, "Make call failed");
            format!("Call failed: {}", e)
        })
}

#[tauri::command]
async fn sip_hangup(state: State<'_, SipAppState>) -> Result<(), String> {
    let handle_guard = state.handle.lock().await;
    let handle = handle_guard
        .as_ref()
        .ok_or_else(|| "Not registered".to_string())?;

    sip::handle_hangup(handle).await.map_err(|e| {
        error!(error = ?e, "Hangup failed");
        format!("Hangup failed: {}", e)
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
    let handle_guard = state.handle.lock().await;
    let handle = handle_guard
        .as_ref()
        .ok_or_else(|| "Not registered".to_string())?;

    sip::handle_toggle_mic_mute(handle).await
}

#[tauri::command]
async fn toggle_speaker_mute(state: State<'_, SipAppState>) -> Result<bool, String> {
    let handle_guard = state.handle.lock().await;
    let handle = handle_guard
        .as_ref()
        .ok_or_else(|| "Not registered".to_string())?;

    sip::handle_toggle_speaker_mute(handle).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::initialize_logging("info", true);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(SipAppState {
            handle: tokio::sync::Mutex::new(None),
            cancel_token: tokio::sync::Mutex::new(None),
            input_device: tokio::sync::Mutex::new(None),
            output_device: tokio::sync::Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            enumerate_audio_devices,
            sip_is_registered,
            sip_register,
            sip_unregister,
            sip_make_call,
            sip_hangup,
            set_input_device,
            set_output_device,
            toggle_mic_mute,
            toggle_speaker_mute,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
