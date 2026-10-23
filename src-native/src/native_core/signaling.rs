//! lowlat examples/client/signaling.cc account URL, using our existing Rust TLS stack.
//! The BUD adapter still owns offers, candidates, retries and media credentials.
use super::account::Session;
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use std::{sync::mpsc, time::Duration};
use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, Message};

pub struct Socket {
    pub writer: crate::bud_attempt::SignalingWrite,
    pub incoming: mpsc::Receiver<String>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Socket {
    pub fn open(session: &Session) -> Result<Self> {
        let mut url = reqwest::Url::parse("wss://kessel-ws.parsec.app")?;
        // Keep the exact no-slash-before-query form used by lowlat's demo.
        url.query_pairs_mut().extend_pairs([
            ("session_id", session.token()),
            ("role", "client"),
            ("version", "1"),
            ("build", "lowlat-client"),
            ("sdk_version", "0"),
        ]);
        let url = url.to_string().replacen(".app/?", ".app?", 1);
        let (out, mut outgoing) = tokio::sync::mpsc::channel::<String>(64);
        let (incoming_tx, incoming) = mpsc::sync_channel(64);
        let (stop, mut stopping) = tokio::sync::oneshot::channel();
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let consumer = std::thread::current();
        let worker = std::thread::Builder::new().name("native-account-signaling".into()).spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(r)=>r, Err(_)=>{let _=ready_tx.send(false);return;}
            };
            runtime.block_on(async move {
                let config = WebSocketConfig::default().max_message_size(Some(16384)).max_frame_size(Some(16384));
                let connected = tokio::time::timeout(Duration::from_secs(12),
                    tokio_tungstenite::connect_async_with_config(url.as_str(),Some(config),false)).await;
                let mut socket = match connected {
                    Ok(Ok((socket,_)))=>socket,
                    _=>{let _=ready_tx.send(false);return;}
                };
                if ready_tx.send(true).is_err() {return;}
                loop {
                    tokio::select! {
                        _ = &mut stopping => { let _=tokio::time::timeout(Duration::from_secs(1),socket.close(None)).await; break; }
                        next = outgoing.recv() => {
                            let Some(text)=next else {break;};
                            if !matches!(tokio::time::timeout(Duration::from_secs(5),socket.send(Message::Text(text.into()))).await,Ok(Ok(()))) {break;}
                        }
                        next = socket.next() => match next {
                            Some(Ok(Message::Text(text))) => {
                                if incoming_tx.try_send(text.to_string()).is_err() {break;}
                                consumer.unpark();
                            }
                            Some(Ok(Message::Ping(bytes))) => {
                                if !matches!(tokio::time::timeout(Duration::from_secs(5),socket.send(Message::Pong(bytes))).await,Ok(Ok(()))) {break;}
                            }
                            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                            _ => {}
                        }
                    }
                }
                consumer.unpark();
            });
        })?;
        // Handshake has its own deadline; never expose an error containing the URL/token.
        if ready.recv_timeout(Duration::from_secs(15)) != Ok(true) {
            let _ = stop.send(());
            let _ = worker.join();
            anyhow::bail!("Could not open authenticated Parsec signaling.");
        }
        Ok(Self {
            writer: std::sync::Arc::new(move |text| out.try_send(text).is_ok()),
            incoming,
            stop: Some(stop),
            worker: Some(worker),
        })
    }
    pub fn next(&self) -> Result<Option<String>> {
        match self.incoming.try_recv() {
            Ok(text) => Ok(Some(text)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(anyhow::anyhow!("Account signaling closed"))
            }
        }
    }
}
impl Drop for Socket {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
