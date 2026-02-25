mod logging;
mod sip;
mod webrtc;

use rustls;
use sip::state::SipAppState;
use tauri::{Manager, State};
use tracing::error;

// ── Audio device enumeration via cpal ──

/// Temporarily redirect stderr to /dev/null while `f` runs, then restore it.
///
/// cpal probes ALSA, JACK and other Linux audio backends by trying to open every
/// PCM device it finds. This causes libasound and libjack to print dozens of
/// harmless-but-noisy messages directly to stderr (bypassing the Rust logging
/// framework). Redirecting stderr around the enumeration call suppresses that
/// noise without losing any real application log output.
#[cfg(target_os = "linux")]
fn with_suppressed_stderr<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    use libc::{close, dup, dup2, open, O_WRONLY};
    unsafe {
        let saved = dup(2);
        let devnull = open(b"/dev/null\0".as_ptr() as *const libc::c_char, O_WRONLY);
        if devnull >= 0 {
            dup2(devnull, 2);
            close(devnull);
        }
        let result = f();
        if saved >= 0 {
            dup2(saved, 2);
            close(saved);
        }
        result
    }
}

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

/// Linux-specific: Enumerate audio devices using PulseAudio/PipeWire as the primary
/// source of truth, so that displayed names match GNOME Settings → Sound exactly.
///
/// Strategy:
/// 1. Query PulseAudio/PipeWire sinks (outputs) and sources (inputs) for their
///    human-readable `description` and ALSA hardware properties (`alsa.card`,
///    `alsa.device`).
/// 2. Convert each PA device → candidate cpal device IDs:
///      alsa.card (index) → /proc/asound/cards → short name → "alsa:plughw:CARD=<short>,DEV=<dev>"
///    Also try the legacy format without CARD=/DEV= keywords.
/// 3. Check the candidate IDs against cpal's actual device list; keep the first
///    match (guarantees the ID is openable by AudioBridge).
/// 4. If PulseAudio is unavailable or yields no matches, fall back to raw cpal
///    ALSA enumeration with cpal-supplied descriptions.
#[cfg(target_os = "linux")]
fn enumerate_audio_devices_linux() -> Result<AudioDevices, String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    use pulsectl::controllers::{DeviceControl, SinkController, SourceController};
    use tracing::{debug, warn};

    let card_short_names = get_alsa_card_short_names();
    debug!(cards = ?card_short_names, "ALSA card short names from /proc/asound/cards");

    let host = cpal::default_host();

    // Collect all valid cpal device IDs upfront, split by capability.
    // Wrapped in with_suppressed_stderr to silence ALSA/JACK probe noise.
    let (cpal_input_ids, cpal_output_ids) = with_suppressed_stderr(|| {
        let mut ins = std::collections::HashSet::<String>::new();
        let mut outs = std::collections::HashSet::<String>::new();
        if let Ok(devs) = host.devices() {
            for d in devs {
                if let Ok(id) = d.id() {
                    let s = id.to_string();
                    if d.default_input_config().is_ok() { ins.insert(s.clone()); }
                    if d.default_output_config().is_ok() { outs.insert(s); }
                }
            }
        }
        (ins, outs)
    });  // with_suppressed_stderr
    debug!(inputs = ?cpal_input_ids, outputs = ?cpal_output_ids, "cpal device IDs");

    // Build cpal ID candidates from a PulseAudio proplist.
    // Tries both "alsa:plughw:CARD=<short>,DEV=<dev>" (modern) and
    // "alsa:plughw:<short>,<dev>" (legacy) so either ALSA naming convention works.
    macro_rules! pa_cpal_id_candidates {
        ($proplist:expr) => {{
            $proplist
                .get_str("alsa.card")
                .and_then(|s| s.parse::<u32>().ok())
                .and_then(|n| card_short_names.get(&n).cloned())
                .map(|short| {
                    let dev = $proplist
                        .get_str("alsa.device")
                        .unwrap_or_else(|| "0".to_string());
                    vec![
                        format!("alsa:plughw:CARD={},DEV={}", short, dev),
                        format!("alsa:plughw:{},{}", short, dev),
                    ]
                })
                .unwrap_or_default()
        }};
    }

    let mut inputs: Vec<AudioDevice> = Vec::new();
    let mut outputs: Vec<AudioDevice> = Vec::new();
    let mut pa_produced_results = false;

    // ── PulseAudio sources → input devices ──────────────────────────────────
    match SourceController::create() {
        Ok(mut ctrl) => {
            if let Ok(sources) = ctrl.list_devices() {
                for src in sources {
                    if src.monitor.is_some() { continue; } // skip monitor sources
                    let candidates = pa_cpal_id_candidates!(src.proplist);
                    let description = src
                        .description
                        .unwrap_or_else(|| src.name.unwrap_or_default());
                    debug!(description = %description, ?candidates, "PA source");
                    if let Some(cpal_id) = candidates.iter().find(|id| cpal_input_ids.contains(*id)) {
                        pa_produced_results = true;
                        inputs.push(AudioDevice { name: cpal_id.clone(), description });
                    }
                }
            }
        }
        Err(e) => warn!(error = %e, "PulseAudio SourceController unavailable"),
    }

    // ── PulseAudio sinks → output devices ───────────────────────────────────
    match SinkController::create() {
        Ok(mut ctrl) => {
            if let Ok(sinks) = ctrl.list_devices() {
                for sink in sinks {
                    let candidates = pa_cpal_id_candidates!(sink.proplist);
                    let description = sink
                        .description
                        .unwrap_or_else(|| sink.name.unwrap_or_default());
                    debug!(description = %description, ?candidates, "PA sink");
                    if let Some(cpal_id) = candidates.iter().find(|id| cpal_output_ids.contains(*id)) {
                        pa_produced_results = true;
                        outputs.push(AudioDevice { name: cpal_id.clone(), description });
                    }
                }
            }
        }
        Err(e) => warn!(error = %e, "PulseAudio SinkController unavailable"),
    }

    // ── Fallback: use raw cpal ALSA descriptions ─────────────────────────────
    if !pa_produced_results {
        warn!("PulseAudio produced no matching devices; falling back to cpal ALSA names");
        return enumerate_audio_devices_cpal_fallback(&host);
    }

    Ok(AudioDevices { inputs, outputs })
}

/// Linux-only cpal fallback: enumerate ALSA devices and use cpal-supplied descriptions.
/// Used when PulseAudio is unavailable (e.g. bare ALSA / headless systems).
#[cfg(target_os = "linux")]
fn enumerate_audio_devices_cpal_fallback(host: &cpal::Host) -> Result<AudioDevices, String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    use tracing::warn;

    let (inputs, outputs) = with_suppressed_stderr(|| {
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();

        let devices = match host.devices() {
            Ok(d) => d,
            Err(_) => return (inputs, outputs),
        };

        for device in devices {
            let id = match device.id() {
                Ok(id) => id.to_string(),
                Err(e) => { warn!(error = ?e, "Failed to get device ID"); continue; }
            };
            let local_id = id.split_once(':').map(|(_, r)| r).unwrap_or(&id);
            if !is_useful_device(local_id) { continue; }

            let desc = device
                .description()
                .map(|d| d.to_string())
                .unwrap_or_else(|_| id.clone());

            if device.default_input_config().is_ok() {
                inputs.push(AudioDevice { name: id.clone(), description: desc.clone() });
            }
            if device.default_output_config().is_ok() {
                outputs.push(AudioDevice { name: id, description: desc });
            }
        }
        (inputs, outputs)
    });

    Ok(AudioDevices { inputs, outputs })
}

#[tauri::command]
fn enumerate_audio_devices() -> Result<AudioDevices, String> {
    // On Linux, use PulseAudio/PipeWire as primary source so device names match
    // GNOME Settings → Sound. Falls back to raw cpal ALSA if PA is unavailable.
    #[cfg(target_os = "linux")]
    return enumerate_audio_devices_linux();

    // On macOS / Windows, cpal descriptions are already the correct system names.
    #[cfg(not(target_os = "linux"))]
    {
        use cpal::traits::{DeviceTrait, HostTrait};
        use tracing::warn;

        let host = cpal::default_host();
        let devices = host
            .devices()
            .map_err(|e| format!("Failed to enumerate devices: {}", e))?;

        let mut inputs = Vec::new();
        let mut outputs = Vec::new();

        for device in devices {
            let id = match device.id() {
                Ok(id) => id.to_string(),
                Err(e) => { warn!(error = ?e, "Failed to get device ID"); continue; }
            };
            let desc = device
                .description()
                .map(|d| d.to_string())
                .unwrap_or_else(|_| id.clone());
            if device.default_input_config().is_ok() {
                inputs.push(AudioDevice { name: id.clone(), description: desc.clone() });
            }
            if device.default_output_config().is_ok() {
                outputs.push(AudioDevice { name: id, description: desc });
            }
        }

        Ok(AudioDevices { inputs, outputs })
    }
}

/// Filter out ALSA virtual plugins and duplicates for the cpal fallback path.
#[cfg(target_os = "linux")]
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
            // Modern format: plughw:CARD=PCH,DEV=0
            if let Some(card_val) = rest.strip_prefix("CARD=") {
                let card_name = card_val.split(&[',', ':']).next().unwrap_or("");
                return !card_name.chars().all(|c| c.is_ascii_digit());
            }
            // Legacy format: plughw:PCH,0
            let card_name = rest.split(&[',', ':']).next().unwrap_or("");
            return !card_name.is_empty() && !card_name.chars().all(|c| c.is_ascii_digit());
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
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Prevent the default close so we can send SIP UNREGISTER first.
                // registration_refresh_loop sends REGISTER expires=0 when the
                // cancel_token is cancelled, then the window is closed explicitly.
                api.prevent_close();
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<SipAppState>();
                    if let Some(token) = state.cancel_token.lock().await.take() {
                        token.cancel();
                        // Give registration_refresh_loop time to send UNREGISTER.
                        // 500 ms is sufficient for LAN/fast WAN; the server's
                        // expires timer handles cleanup if the network is slower.
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    }
                    state.handle.lock().await.take();
                    app.exit(0);
                });
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
