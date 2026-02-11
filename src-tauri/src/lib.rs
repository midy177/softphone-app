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

#[tauri::command]
fn enumerate_audio_devices() -> Result<AudioDevices, String> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let host = cpal::default_host();
    let devices = host
        .devices()
        .map_err(|e| format!("Failed to enumerate devices: {}", e))?;

    let mut inputs = Vec::new();
    let mut outputs = Vec::new();

    for device in devices {
        let id = match device.id() {
            Ok(id) => id.to_string(),
            Err(_) => continue,
        };

        // Extract the backend-local part of the ID (e.g. "alsa:plughw:CARD=PCH,DEV=0" → "plughw:CARD=PCH,DEV=0")
        let local_id = id.split_once(':').map(|(_, rest)| rest).unwrap_or(&id);

        // Only keep useful devices, skip ALSA virtual plugins
        if !is_useful_device(local_id) {
            continue;
        }

        let desc = device
            .description()
            .map(|d| d.to_string())
            .unwrap_or_else(|_| id.clone());

        let has_input = device.default_input_config().is_ok();
        let has_output = device.default_output_config().is_ok();

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

    Ok(AudioDevices { inputs, outputs })
}

/// Filter out ALSA virtual plugins and duplicates, only keep real/useful devices.
/// Keeps: `default`, `plughw:CARD=<name>` (by card name, not number to deduplicate).
/// Skips: pipewire, pulse, sysdefault (redundant with default), raw hw:, all virtual plugins.
fn is_useful_device(local_id: &str) -> bool {
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
