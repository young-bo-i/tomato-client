//! Persistent CDP (Chrome DevTools Protocol) client.
//!
//! `wayfern_manager::send_cdp_command` opens a fresh WebSocket per call,
//! which is fine for one-shot fingerprint injection but cannot subscribe
//! to events. This client keeps one WebSocket open per Wayfern target,
//! routes responses to the awaiting caller by id, and pumps unsolicited
//! events (e.g. `Runtime.bindingCalled`) into a channel the caller drains.
//!
//! Design:
//! - Two background tasks per connection: a writer (consumes commands) and
//!   a reader (dispatches to pending replies or the events channel).
//! - When the reader sees the WS close, it drops the events sender — the
//!   gather worker observes this and tears down its loop.
//! - `call()` has a 15s timeout so a stuck browser cannot wedge a worker.
//!   Setup commands like `Network.setBlockedURLs` complete in <100ms, the
//!   timeout exists purely as a liveness backstop.
//! - The writer task holds the `WriteHalf`; the `Cdp` handle just owns the
//!   command-side of an mpsc and is `Send + Sync`, so multiple workers
//!   could share it (we don't, one Cdp per worker).

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const CALL_TIMEOUT: Duration = Duration::from_secs(15);
const EVENT_CHANNEL_CAP: usize = 512;
const CMD_CHANNEL_CAP: usize = 64;

#[derive(Debug, Clone)]
pub struct CdpEvent {
  pub method: String,
  pub params: Value,
}

enum WriterCmd {
  Send {
    id: i64,
    payload: String,
    reply: oneshot::Sender<Result<Value, String>>,
  },
  Close,
}

pub struct Cdp {
  next_id: AtomicI64,
  cmd_tx: mpsc::Sender<WriterCmd>,
}

impl Cdp {
  /// Connect to a CDP target (browser-level or page-level WebSocket URL),
  /// returning the handle plus the events receiver. Caller is expected to
  /// drain the events stream — the channel is capped so a stalled consumer
  /// causes events to drop with a tracing warning rather than memory bloat.
  pub async fn connect(ws_url: &str) -> Result<(Self, mpsc::Receiver<CdpEvent>), String> {
    let (ws, _) = connect_async(ws_url)
      .await
      .map_err(|e| format!("cdp ws connect {ws_url}: {e}"))?;
    let (mut sink, mut stream) = ws.split();

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriterCmd>(CMD_CHANNEL_CAP);
    let (event_tx, event_rx) = mpsc::channel::<CdpEvent>(EVENT_CHANNEL_CAP);
    let pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>> =
      Arc::new(Mutex::new(HashMap::new()));

    // Writer: pulls commands off cmd_rx and pushes onto the WS.
    let pending_w = pending.clone();
    tokio::spawn(async move {
      while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
          WriterCmd::Send { id, payload, reply } => {
            pending_w.lock().await.insert(id, reply);
            if let Err(e) = sink.send(Message::Text(payload.into())).await {
              if let Some(r) = pending_w.lock().await.remove(&id) {
                let _ = r.send(Err(format!("cdp write: {e}")));
              }
              break;
            }
          }
          WriterCmd::Close => {
            let _ = sink.close().await;
            break;
          }
        }
      }
      // On exit, fail every still-pending caller.
      let mut p = pending_w.lock().await;
      for (_, r) in p.drain() {
        let _ = r.send(Err("cdp connection closed".into()));
      }
    });

    // Reader: routes frames to either pending replies or the event stream.
    let pending_r = pending.clone();
    tokio::spawn(async move {
      while let Some(msg) = stream.next().await {
        let text = match msg {
          Ok(Message::Text(t)) => t,
          Ok(Message::Close(_)) => break,
          Ok(_) => continue,
          Err(_) => break,
        };
        let parsed: Value = match serde_json::from_str(text.as_str()) {
          Ok(v) => v,
          Err(_) => continue,
        };
        if let Some(id) = parsed.get("id").and_then(|v| v.as_i64()) {
          if let Some(reply) = pending_r.lock().await.remove(&id) {
            if let Some(err) = parsed.get("error") {
              let _ = reply.send(Err(format!("cdp error: {err}")));
            } else {
              let _ = reply.send(Ok(parsed.get("result").cloned().unwrap_or(json!({}))));
            }
          }
        } else if let Some(method) = parsed.get("method").and_then(|v| v.as_str()) {
          let ev = CdpEvent {
            method: method.to_string(),
            params: parsed.get("params").cloned().unwrap_or(json!({})),
          };
          // try_send: drop on full rather than wedge the reader.
          if event_tx.try_send(ev).is_err() {
            log::warn!("cdp events channel full — dropping {method}");
          }
        }
      }
      // Reader exit = remote disconnect. event_tx drop signals consumer.
    });

    Ok((
      Self {
        next_id: AtomicI64::new(1),
        cmd_tx,
      },
      event_rx,
    ))
  }

  /// Send a CDP command and await its reply. Times out after 15s.
  pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
    let id = self.next_id.fetch_add(1, Ordering::Relaxed);
    let payload = json!({
      "id": id,
      "method": method,
      "params": params,
    })
    .to_string();
    let (tx, rx) = oneshot::channel();
    self
      .cmd_tx
      .send(WriterCmd::Send {
        id,
        payload,
        reply: tx,
      })
      .await
      .map_err(|_| "cdp writer task gone".to_string())?;

    match tokio::time::timeout(CALL_TIMEOUT, rx).await {
      Ok(Ok(res)) => res,
      Ok(Err(_)) => Err("cdp reply dropped".into()),
      Err(_) => Err(format!("cdp.{method} timed out after {CALL_TIMEOUT:?}")),
    }
  }

  pub async fn close(&self) {
    let _ = self.cmd_tx.send(WriterCmd::Close).await;
  }
}

#[derive(Deserialize)]
struct PageTargetEntry {
  #[serde(rename = "type")]
  target_type: String,
  #[serde(rename = "webSocketDebuggerUrl", default)]
  ws_url: Option<String>,
  #[serde(default)]
  url: Option<String>,
}

/// Resolve a Wayfern CDP debug port to a page target's WebSocket URL by
/// querying `http://127.0.0.1:{port}/json`. Prefers a douyin.com tab when
/// several are open. Used by the one-shot DOM dump path (`dump.rs`).
pub(super) async fn fetch_first_page_ws(port: u16) -> Result<String, String> {
  // Bounded by both connect-only and overall timeouts so a wedged CDP
  // port doesn't hang the caller indefinitely.
  static HTTP: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
  let client = HTTP.get_or_init(|| {
    reqwest::Client::builder()
      .timeout(Duration::from_secs(3))
      .connect_timeout(Duration::from_secs(1))
      .build()
      .expect("kol_automation::cdp reqwest client init")
  });
  let url = format!("http://127.0.0.1:{port}/json");
  let resp = client
    .get(&url)
    .send()
    .await
    .map_err(|e| format!("targets fetch: {e}"))?;
  let targets: Vec<PageTargetEntry> = resp
    .json()
    .await
    .map_err(|e| format!("targets parse: {e}"))?;

  // Prefer a target whose URL hints at douyin.com — covers the case where
  // an existing instance has multiple tabs open.
  if let Some(t) = targets
    .iter()
    .find(|t| {
      t.target_type == "page"
        && t.ws_url.is_some()
        && t.url.as_deref().map(|u| u.contains("douyin")).unwrap_or(false)
    })
    .or_else(|| {
      targets
        .iter()
        .find(|t| t.target_type == "page" && t.ws_url.is_some())
    })
  {
    return Ok(t.ws_url.clone().unwrap());
  }
  Err("no page target with ws url".into())
}
