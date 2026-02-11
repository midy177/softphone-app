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
/// Returns a mapping from ALSA device identifiers to user-visible descriptions.
#[cfg(target_os = "linux")]
fn get_pulse_friendly_names() -> (std::collections::HashMap<String, String>, std::collections::HashMap<String, String>) {
    use std::collections::HashMap;
    use pulsectl::controllers::{SinkController, SourceController, DeviceControl};
    use tracing::{info, warn};

    let mut input_map = HashMap::new();
    let mut output_map = HashMap::new();

    // Get input devices (sources) via SourceController
    match SourceController::create() {
        Ok(mut source_ctrl) => {
            if let Ok(sources) = source_ctrl.list_devices() {
                info!(count = sources.len(), "Found PulseAudio input sources");
                for source in sources {
                    let card_name = source.proplist.get_str("alsa.card_name")
                        .or_else(|| source.proplist.get_str("device.product.name"));

                    if let Some(card) = card_name {
                        let friendly_name = source.description.unwrap_or_else(|| {
                            source.name.unwrap_or_default()
                        });
                        info!(card = %card, friendly_name = %friendly_name, "Mapped input device");
                        input_map.insert(card, friendly_name);
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
                info!(count = sinks.len(), "Found PulseAudio output sinks");
                for sink in sinks {
                    let card_name = sink.proplist.get_str("alsa.card_name")
                        .or_else(|| sink.proplist.get_str("device.product.name"));

                    if let Some(card) = card_name {
                        let friendly_name = sink.description.unwrap_or_else(|| {
                            sink.name.unwrap_or_default()
                        });
                        info!(card = %card, friendly_name = %friendly_name, "Mapped output device");
                        output_map.insert(card, friendly_name);
                    }
                }
            }
        }
        Err(e) => {
            warn!(error = %e, "Failed to connect to PulseAudio SinkController");
        }
    }

    (input_map, output_map)
}

/// Try to extract a matchable identifier from ALSA device string (e.g., card name).
#[cfg(target_os = "linux")]
fn extract_alsa_card_name(alsa_desc: &str) -> Option<String> {
    // ALSA descriptions often look like "HDA Intel PCH, ALC897 Analog"
    // Extract the first part (card name)
    alsa_desc.split(',').next().map(|s| s.trim().to_string())
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
    info!(host_id = ?host.id(), "Using audio host");

    let devices = host
        .devices()
        .map_err(|e| format!("Failed to enumerate devices: {}", e))?;

    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let mut device_count = 0;
    let mut skipped_count = 0;

    for device in devices {
        device_count += 1;

        let id = match device.id() {
            Ok(id) => id.to_string(),
            Err(e) => {
                warn!(error = ?e, "Failed to get device ID");
                continue;
            }
        };

        info!(device_id = %id, "Found device");

        // Extract the backend-local part of the ID (e.g. "alsa:plughw:CARD=PCH,DEV=0" → "plughw:CARD=PCH,DEV=0")
        let local_id = id.split_once(':').map(|(_, rest)| rest).unwrap_or(&id);

        // Only keep useful devices, skip ALSA virtual plugins
        if !is_useful_device(local_id) {
            info!(device_id = %id, local_id = %local_id, "Skipping device (not useful)");
            skipped_count += 1;
            continue;
        }

        // Get the base description from cpal
        let cpal_desc = device
            .description()
            .map(|d| d.to_string())
            .unwrap_or_else(|_| id.clone());

        // On Linux, try to find a friendly name from PulseAudio
        #[cfg(target_os = "linux")]
        let desc = {
            if let Some(card_name) = extract_alsa_card_name(&cpal_desc) {
                // Try to find in PulseAudio mappings
                let friendly = pulse_input_map.get(&card_name)
                    .or_else(|| pulse_output_map.get(&card_name))
                    .cloned();

                if let Some(ref friendly_name) = friendly {
                    info!(id = %id, cpal_desc = %cpal_desc, friendly_name = %friendly_name, "Using friendly name");
                    friendly_name.clone()
                } else {
                    info!(id = %id, cpal_desc = %cpal_desc, "No friendly name found, using cpal description");
                    cpal_desc
                }
            } else {
                cpal_desc
            }
        };

        // On macOS/Windows, use cpal description directly
        #[cfg(not(target_os = "linux"))]
        let desc = {
            info!(id = %id, description = %cpal_desc, "Using cpal description");
            cpal_desc
        };

        let has_input = device.default_input_config().is_ok();
        let has_output = device.default_output_config().is_ok();

        info!(device_id = %id, has_input = %has_input, has_output = %has_output, "Device capabilities");

        if has_input {
            inputs.push(AudioDevice {
                name: id.clone(),
                description: desc.clone(),
            });
        }
        if has_output {
            outputs.push(AudioDevice {
                name: id,
                description: desc,
            });
        }
    }

    info!(
        total_devices = device_count,
        skipped = skipped_count,
        input_count = inputs.len(),
        output_count = outputs.len(),
        "Device enumeration complete"
    );

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
