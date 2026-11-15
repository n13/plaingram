//! Bindings to TDLib's JSON interface, with types generated from the schema of the linked TDLib build.

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_double, c_int};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use futures_channel::oneshot;
use serde_json::{Value, json};

mod generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub use generated::{enums, functions, types};

unsafe extern "C" {
    fn td_create_client_id() -> c_int;
    fn td_send(client_id: c_int, request: *const c_char);
    fn td_receive(timeout: c_double) -> *const c_char;
    fn td_execute(request: *const c_char) -> *const c_char;
}

static PENDING: LazyLock<Mutex<HashMap<u64, oneshot::Sender<Value>>>> = LazyLock::new(Default::default);
static NEXT_EXTRA: AtomicU64 = AtomicU64::new(0);

pub fn create_client() -> i32 {
    unsafe { td_create_client_id() }
}

pub fn set_log_verbosity(level: i32) -> Result<(), types::Error> {
    let request = to_c_string(&json!({ "@type": "setLogVerbosityLevel", "new_verbosity_level": level }));
    let response = unsafe { parse(td_execute(request.as_ptr())) }.expect("TDLib cannot execute setLogVerbosityLevel");
    match response["@type"] == "error" {
        true => Err(serde_json::from_value(response).expect("malformed TDLib error")),
        false => Ok(()),
    }
}

/// Waits up to `timeout` seconds. Responses are routed to their pending requests; updates are returned.
pub fn receive(timeout: f64) -> Option<enums::Update> {
    let response = unsafe { parse(td_receive(timeout)) }?;
    if let Some(extra) = response.get("@extra").and_then(Value::as_u64) {
        match PENDING.lock().expect("pending requests lock poisoned").remove(&extra) {
            Some(sender) => {
                if sender.send(response).is_err() {
                    tracing::warn!("TDLib response {extra} was not awaited");
                }
            }
            None => tracing::error!("TDLib response {extra} matches no request"),
        }
        return None;
    }
    serde_json::from_value(response).inspect_err(|e| tracing::error!("unparseable TDLib update: {e}")).ok()
}

async fn send_request(client_id: i32, mut request: Value) -> Value {
    let extra = NEXT_EXTRA.fetch_add(1, Ordering::Relaxed);
    request["@extra"] = extra.into();
    let (sender, receiver) = oneshot::channel();
    PENDING.lock().expect("pending requests lock poisoned").insert(extra, sender);
    unsafe { td_send(client_id, to_c_string(&request).as_ptr()) };
    receiver.await.expect("TDLib response channel closed")
}

fn to_c_string(request: &Value) -> CString {
    CString::new(request.to_string()).expect("serialized JSON never contains NUL")
}

unsafe fn parse(raw: *const c_char) -> Option<Value> {
    let raw = unsafe { raw.as_ref() }?;
    let text = unsafe { CStr::from_ptr(raw) }.to_str().expect("TDLib returned invalid UTF-8");
    Some(serde_json::from_str(text).expect("TDLib returned invalid JSON"))
}
