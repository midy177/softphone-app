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

/// Linux-specific: Read `/proc/asound/cards` to build a map of card index → ALSA short name.
///
/// Example line: " 0 [PCH            ]: HDA-Intel - HDA Intel PCH"
/// Produces: {0: "PCH", 1: "Headset", ...}
///
/// The ALSA short name is the authoritative bridge between:
/// - cpal device IDs:     `alsa:plughw:CARD=PCH,DEV=0`  (contains short name directly)
/// - PulseAudio proplist: `alsa.card = "0"`              (numeric index → short name lookup)
#[cfg(target_os = "linux")]
fn get_alsa_card_short_names() -> std::collections::HashMap<u32, String> {
    let mut map = std::collections::HashMap::new();
    let Ok(content) = std::fs::read_to_string("/proc/asound/cards") else {
        return map;
    };
    for line in content.lines() {
        let line = line.trim();
        let Some(b_start) = line.find('[') else { continue };
        let Some(b_end) = line.find(']') else { continue };
        if b_start == 0 {
            continue; // no card number before '['
        }
        let Ok(num) = line[..b_start].trim().parse::<u32>() else { continue };
        let short = line[b_start + 1..b_end].trim().to_string();
        if !short.is_empty() {
            map.insert(num, short);
        }
    }
    map
}

/// Linux-specific: Query PulseAudio/PipeWire for friendly device names.
///
/// Returns two maps keyed by ALSA card short name (e.g. "PCH", "Headset"):
/// - compound key: "PCH:0"  (card_short:dev_num, preferred)
/// - fallback key: "PCH"    (card_short only, first match wins)
///
/// Matching strategy (most → least reliable):
/// 1. `alsa.card` (numeric index) → look up in /proc/asound/cards → ALSA short name
/// 2. Fallback: `alsa.card_name` used directly as key
#[cfg(target_os = "linux")]
fn get_pulse_friendly_names() -> (
    std::collections::HashMap<String, String>,
    std::collections::HashMap<String, String>,
) {
    use pulsectl::controllers::{DeviceControl, SinkController, SourceController};
    use std::collections::HashMap;
    use tracing::{debug, warn};

    let card_short_names = get_alsa_card_short_names();
    debug!(cards = ?card_short_names, "ALSA card short names from /proc/asound/cards");

    let mut input_map = HashMap::new();
    let mut output_map = HashMap::new();

    fn insert_device(
        map: &mut HashMap<String, String>,
        card: &str,
        dev: Option<String>,
        friendly: String,
    ) {
        if let Some(ref d) = dev {
            map.insert(format!("{}:{}", card, d), friendly.clone());
        }
        map.entry(card.to_string()).or_insert(friendly);
    }

    // Resolve ALSA short name from a PulseAudio device proplist.
    // Primary: alsa.card (numeric) → /proc/asound/cards short name
    // Fallback: alsa.card_name string (older PulseAudio / non-PipeWire)
    macro_rules! resolve_short_name {
        ($proplist:expr) => {
            $proplist
                .get_str("alsa.card")
                .and_then(|s| s.parse::<u32>().ok())
                .and_then(|n| card_short_names.get(&n).cloned())
                .or_else(|| $proplist.get_str("alsa.card_name"))
        };
    }

    match SourceController::create() {
        Ok(mut ctrl) => {
            if let Ok(sources) = ctrl.list_devices() {
                for src in sources {
                    if src.monitor.is_some() {
                        continue;
                    }
                    if let Some(short) = resolve_short_name!(src.proplist) {
                        let dev = src.proplist.get_str("alsa.device");
                        let friendly = src
                            .description
                            .unwrap_or_else(|| src.name.unwrap_or_default());
                        debug!(card = %short, dev = ?dev, friendly = %friendly, "PulseAudio source");
                        insert_device(&mut input_map, &short, dev, friendly);
                    }
                }
            }
        }
        Err(e) => warn!(error = %e, "Failed to connect to PulseAudio SourceController"),
    }

    match SinkController::create() {
        Ok(mut ctrl) => {
            if let Ok(sinks) = ctrl.list_devices() {
                for sink in sinks {
                    if let Some(short) = resolve_short_name!(sink.proplist) {
                        let dev = sink.proplist.get_str("alsa.device");
                        let friendly = sink
                            .description
                            .unwrap_or_else(|| sink.name.unwrap_or_default());
                        debug!(card = %short, dev = ?dev, friendly = %friendly, "PulseAudio sink");
                        insert_device(&mut output_map, &short, dev, friendly);
                    }
                }
            }
        }
        Err(e) => warn!(error = %e, "Failed to connect to PulseAudio SinkController"),
    }

    debug!(
        input_keys = ?input_map.keys().collect::<Vec<_>>(),
        output_keys = ?output_map.keys().collect::<Vec<_>>(),
        "PulseAudio friendly name map loaded"
    );

    (input_map, output_map)
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

    // Enumerate audio devices (no logging)

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

        // On Linux, resolve friendly name from PulseAudio using the ALSA card short name.
        // The short name is embedded directly in the cpal device ID:
        //   "plughw:CARD=PCH,DEV=0"  →  card_short = "PCH", dev_num = "0"
        // The pulse maps were built with the same short names (via /proc/asound/cards),
        // so the lookup is exact and does not depend on description string matching.
        #[cfg(target_os = "linux")]
        let (input_desc, output_desc) = {
            if local_id == "default" {
                (Some(cpal_desc.clone()), Some(cpal_desc))
            } else if let Some(card_short) = local_id
                .strip_prefix("plughw:CARD=")
                .and_then(|s| s.split(',').next())
            {
                let dev_num = extract_dev_number(local_id);
                let compound_key = dev_num.as_ref().map(|d| format!("{}:{}", card_short, d));

                let resolve =
                    |map: &std::collections::HashMap<String, String>| -> Option<String> {
                        compound_key
                            .as_ref()
                            .and_then(|k| map.get(k))
                            .or_else(|| map.get(card_short))
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

    // Only log on error
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

    // Get SIP flow config
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
    let prefer_srtp = *state.prefer_srtp.lock().await;
    let noise_reduce = *state.noise_reduce.lock().await;
    let speaker_noise_reduce = *state.speaker_noise_reduce.lock().await;

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

    sip::handle_make_call(&handle, callee, input_device, output_device, cancel_token, prefer_srtp, noise_reduce, speaker_noise_reduce)
        .await
        .map_err(|e| {
            error!(error = ?e, "Make call failed");
            e.to_string().trim_start_matches("Error: ").to_string()
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
    let noise_reduce = *state.noise_reduce.lock().await;
    let speaker_noise_reduce = *state.speaker_noise_reduce.lock().await;

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

    sip::handle_answer_call(&handle, call_id, input_device, output_device, cancel_token, noise_reduce, speaker_noise_reduce)
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
async fn get_noise_reduce(state: State<'_, SipAppState>) -> Result<bool, String> {
    Ok(*state.noise_reduce.lock().await)
}

#[tauri::command]
async fn set_noise_reduce(state: State<'_, SipAppState>, enabled: bool) -> Result<(), String> {
    *state.noise_reduce.lock().await = enabled;

    // Apply immediately to the active call if one exists
    let handle_opt = state.handle.lock().await.clone();
    if let Some(handle) = handle_opt {
        sip::handle_set_noise_reduce(&handle, enabled).await;
    }
    Ok(())
}

#[tauri::command]
async fn get_speaker_noise_reduce(state: State<'_, SipAppState>) -> Result<bool, String> {
    Ok(*state.speaker_noise_reduce.lock().await)
}

#[tauri::command]
async fn set_speaker_noise_reduce(state: State<'_, SipAppState>, enabled: bool) -> Result<(), String> {
    *state.speaker_noise_reduce.lock().await = enabled;

    // Apply immediately to the active call if one exists
    let handle_opt = state.handle.lock().await.clone();
    if let Some(handle) = handle_opt {
        sip::handle_set_speaker_noise_reduce(&handle, enabled).await;
    }
    Ok(())
}

#[tauri::command]
async fn toggle_noise_reduce(state: State<'_, SipAppState>) -> Result<bool, String> {
    let handle = {
        let handle_guard = state.handle.lock().await;
        handle_guard
            .as_ref()
            .ok_or_else(|| "Not registered".to_string())?
            .clone()
    };

    sip::handle_toggle_noise_reduce(&handle).await
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

// ── SIP Flow config commands (unified interface, works before and after registration) ──

/// Enable or disable SIP message flow logging
#[tauri::command]
async fn set_sip_flow_enabled(state: State<'_, SipAppState>, enabled: bool) -> Result<(), String> {
    // Update stored config
    state.sip_flow_config.lock().await.enabled = enabled;

    // If already registered, also update the running instance
    let handle_guard = state.handle.lock().await;
    if let Some(handle) = handle_guard.as_ref() {
        if enabled {
            sip::handle_enable_sip_flow(handle)?;
        } else {
            sip::handle_disable_sip_flow(handle)?;
        }
    }    Ok(())
}

/// Set the SIP message log directory
#[tauri::command]
async fn set_sip_flow_dir(state: State<'_, SipAppState>, dir: String) -> Result<(), String> {
    // Update stored config
    state.sip_flow_config.lock().await.log_dir = dir.clone();

    // If already registered, also update the running instance
    let handle_guard = state.handle.lock().await;
    if let Some(handle) = handle_guard.as_ref() {
        sip::handle_set_sip_flow_dir(handle, dir)?;
    }

    Ok(())
}

/// Get the current SIP message flow log configuration
#[tauri::command]
async fn get_sip_flow_config(
    state: State<'_, SipAppState>,
) -> Result<sip::state::SipFlowConfig, String> {
    // Prefer live state from the registered handle when available
    let handle_guard = state.handle.lock().await;
    if let Some(handle) = handle_guard.as_ref() {
        let enabled = sip::handle_is_sip_flow_enabled(handle)?;
        let log_dir = sip::handle_get_sip_flow_dir(handle)?;
        Ok(sip::state::SipFlowConfig { enabled, log_dir })
    } else {
        // Otherwise return the stored config
        Ok(state.sip_flow_config.lock().await.clone())
    }
}

/// Get the SRTP preference setting
#[tauri::command]
async fn get_prefer_srtp(state: State<'_, SipAppState>) -> Result<bool, String> {
    Ok(*state.prefer_srtp.lock().await)
}

/// Set the SRTP preference setting
#[tauri::command]
async fn set_prefer_srtp(state: State<'_, SipAppState>, enabled: bool) -> Result<(), String> {
    *state.prefer_srtp.lock().await = enabled;
    Ok(())
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
            prefer_srtp: tokio::sync::Mutex::new(true), // default: prefer SRTP
            noise_reduce: tokio::sync::Mutex::new(false), // default: noise reduction disabled
            speaker_noise_reduce: tokio::sync::Mutex::new(false), // default: speaker noise reduction disabled
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
            toggle_noise_reduce,
            get_noise_reduce,
            set_noise_reduce,
            get_speaker_noise_reduce,
            set_speaker_noise_reduce,
            send_dtmf,
            set_sip_flow_enabled,
            set_sip_flow_dir,
            get_sip_flow_config,
            get_prefer_srtp,
            set_prefer_srtp,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
