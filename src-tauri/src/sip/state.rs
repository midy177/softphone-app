use rsip::Uri;
use rsipstack::dialog::authenticate::Credential;
use rsipstack::dialog::dialog::{Dialog, DialogStateSender};
use rsipstack::dialog::dialog_layer::DialogLayer;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::webrtc::WebRtcSession;

pub struct SipAppState {
    pub handle: tokio::sync::Mutex<Option<SipClientHandle>>,
    pub cancel_token: tokio::sync::Mutex<Option<CancellationToken>>,
    pub input_device: tokio::sync::Mutex<Option<String>>,
    pub output_device: tokio::sync::Mutex<Option<String>>,
}

pub struct SipClientHandle {
    pub app_handle: tauri::AppHandle,
    pub dialog_layer: Arc<DialogLayer>,
    pub state_sender: DialogStateSender,
    pub contact: Uri,
    pub credential: Credential,
    pub server: Uri,
    pub active_call: tokio::sync::Mutex<Option<ActiveCall>>,
    pub pending_incoming: Arc<tokio::sync::Mutex<HashMap<String, PendingCall>>>,
    pub _tasks: Vec<tokio::task::JoinHandle<()>>,
}

pub struct ActiveCall {
    pub call_id: String,
    pub dialog: Dialog,
    pub webrtc_session: Option<WebRtcSession>,
}

pub struct PendingCall {
    pub call_id: String,
    pub caller: String,
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
