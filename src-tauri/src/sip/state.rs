use dashmap::DashMap;
use rsip::Uri;
use rsipstack::dialog::authenticate::Credential;
use rsipstack::dialog::dialog::{Dialog, DialogStateSender};
use rsipstack::dialog::dialog_layer::DialogLayer;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::sip::message_inspector::SipFlow;
use crate::webrtc::WebRtcSession;

/// SIP 日志配置
#[derive(Clone, Serialize)]
pub struct SipFlowConfig {
    pub enabled: bool,
    pub log_dir: String,
}

impl Default for SipFlowConfig {
    fn default() -> Self {
        // 默认使用 $HOME/softphone/
        let log_dir = if let Some(home) = std::env::var_os("HOME") {
            let mut path = std::path::PathBuf::from(home);
            path.push("softphone");
            path.to_string_lossy().to_string()
        } else {
            // 如果无法获取 HOME，回退到临时目录
            let mut temp = std::env::temp_dir();
            temp.push("softphone");
            temp.to_string_lossy().to_string()
        };

        Self {
            enabled: false,
            log_dir,
        }
    }
}

pub struct SipAppState {
    pub handle: tokio::sync::Mutex<Option<Arc<SipClientHandle>>>,
    pub cancel_token: tokio::sync::Mutex<Option<CancellationToken>>,
    pub input_device: tokio::sync::Mutex<Option<String>>,
    pub output_device: tokio::sync::Mutex<Option<String>>,
    pub sip_flow_config: tokio::sync::Mutex<SipFlowConfig>,
    pub prefer_srtp: tokio::sync::Mutex<bool>,
    pub noise_reduce: tokio::sync::Mutex<bool>,
    pub speaker_noise_reduce: tokio::sync::Mutex<bool>,
}

pub struct SipClientHandle {
    pub app_handle: tauri::AppHandle,
    pub dialog_layer: Arc<DialogLayer>,
    pub state_sender: DialogStateSender,
    pub contact: Uri,
    pub credential: Credential,
    pub server: Uri,
    pub active_call: Arc<tokio::sync::Mutex<Option<ActiveCall>>>,
    pub pending_incoming: Arc<tokio::sync::Mutex<HashMap<String, PendingCall>>>,
    pub active_call_tokens: Arc<DashMap<String, CancellationToken>>,
    pub sip_flow: Option<Arc<SipFlow>>,
    pub _tasks: Vec<tokio::task::JoinHandle<()>>,
}

pub struct ActiveCall {
    pub call_id: String,
    pub dialog: Dialog,
    pub webrtc_session: Option<WebRtcSession>,
    pub cancel_token: CancellationToken,
}

pub struct PendingCall {
    pub dialog: Dialog,
    pub sdp_offer: String,
}

#[derive(Clone, Serialize)]
pub struct IncomingCallPayload {
    pub call_id: String,
    pub caller: String,
    pub callee: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct CallStatePayload {
    pub state: String,
    pub call_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct RegistrationStatusPayload {
    pub status: String,
    pub message: Option<String>,
}
