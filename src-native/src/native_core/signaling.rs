//! lowlat examples/client/signaling.cc account URL, using our existing Rust TLS stack.
//! The BUD adapter still owns offers, candidates, retries and media credentials.
use super::account::Session;
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use std::{sync::mpsc, time::Duration};
use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, Message};

fn connect_url(token: &str) -> Result<String> {
    let mut url = reqwest::Url::parse("wss://kessel-ws.parsec.app/")?;
    // Unlike the C++ demo's Matoya parser, tungstenite requires the root
    // path. Removing it produces `GET ?session_id=...`, an invalid target.
    // See lowlat crates/kessel/src/url.rs at e548d48.
    url.query_pairs_mut().extend_pairs([
        ("session_id", token),
        ("role", "client"),
        ("version", "1"),
        ("build", "lowlat-client"),
        ("sdk_version", "0"),
    ]);
    Ok(url.into())
}

#[derive(Clone, Copy, Debug)]
struct UpgradeFailure {
    stage: &'static str,
    status: Option<u16>,
}
impl UpgradeFailure {
    fn at(stage: &'static str) -> Self {
        Self {
            stage,
            status: None,
        }
    }
    fn from_error(error: &tokio_tungstenite::tungstenite::Error) -> Self {
        use tokio_tungstenite::tungstenite::Error;
        // Never retain/format raw errors, response bodies, headers or URLs:
        // each can include the credential-bearing request.
        match error {
            Error::Http(response) => Self {
                stage: "http_upgrade",
                status: Some(response.status().as_u16()),
            },
            Error::Io(_) => Self::at("network_or_tls"),
            Error::Tls(_) => Self::at("tls"),
            Error::Url(_) | Error::HttpFormat(_) => Self::at("request_format"),
            _ => Self::at("websocket_handshake"),
        }
    }
}
impl std::fmt::Display for UpgradeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.stage)?;
        if let Some(status) = self.status {
            write!(f, "; HTTP {status}")?;
        }
        Ok(())
    }
}

pub struct Socket {
    pub writer: crate::bud_attempt::SignalingWrite,
    pub incoming: mpsc::Receiver<String>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Socket {
    pub fn open(session: &Session) -> Result<Self> {
        let url = connect_url(session.token())?;
        let (out, mut outgoing) = tokio::sync::mpsc::channel::<String>(64);
        let (incoming_tx, incoming) = mpsc::sync_channel(64);
        let (stop, mut stopping) = tokio::sync::oneshot::channel();
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let consumer = std::thread::current();
        let worker = std::thread::Builder::new().name("native-account-signaling".into()).spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(r)=>r, Err(_)=>{let _=ready_tx.send(Err(UpgradeFailure::at("runtime")));return;}
            };
            runtime.block_on(async move {
                let config = WebSocketConfig::default().max_message_size(Some(16384)).max_frame_size(Some(16384));
                let connected = tokio::time::timeout(Duration::from_secs(12),
                    tokio_tungstenite::connect_async_with_config(url.as_str(),Some(config),false)).await;
                let mut socket = match connected {
                    Ok(Ok((socket,_)))=>socket,
                    Ok(Err(error))=>{let _=ready_tx.send(Err(UpgradeFailure::from_error(&error)));return;}
                    Err(_)=>{let _=ready_tx.send(Err(UpgradeFailure::at("handshake_timeout")));return;}
                };
                if ready_tx.send(Ok(())).is_err() {return;}
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
        let opened = ready
            .recv_timeout(Duration::from_secs(15))
            .unwrap_or_else(|error| {
                Err(UpgradeFailure::at(match error {
                    mpsc::RecvTimeoutError::Timeout => "worker_timeout",
                    mpsc::RecvTimeoutError::Disconnected => "worker_stopped",
                }))
            });
        if let Err(failure) = opened {
            let _ = stop.send(());
            let _ = worker.join();
            crate::event_log::record(
                "ERROR",
                "native_signaling_upgrade_failed",
                serde_json::json!({"stage":failure.stage,"http_status":failure.status}),
            );
            anyhow::bail!("Could not open authenticated Parsec signaling ({failure}).");
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::{
        client::IntoClientRequest, handshake::client::generate_request,
    };

    #[test]
    fn upgrade_request_has_root_target_and_an_unmodified_host_header() {
        let url = connect_url("a b&c=d+/").unwrap();
        let parsed = reqwest::Url::parse(&url).unwrap();
        let fields: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(fields["session_id"], "a b&c=d+/");
        assert_eq!(fields["role"], "client");
        assert_eq!(fields["version"], "1");
        let request = url.into_client_request().unwrap();
        assert_eq!(request.headers()["host"], "kessel-ws.parsec.app");
        let (wire, _) = generate_request(request).unwrap();
        let wire = String::from_utf8(wire).unwrap();
        assert!(wire.starts_with("GET /?session_id="));
        assert!(!wire.starts_with("GET ?"));
    }

    #[test]
    fn handshake_diagnostics_preserve_status_without_server_secrets() {
        use tokio_tungstenite::tungstenite::{http::Response, Error};
        let error = Error::Http(Box::new(
            Response::builder()
                .status(400)
                .header("location", "https://example.invalid/?token=secret-sentinel")
                .body(Some(b"secret-sentinel".to_vec()))
                .unwrap(),
        ));
        let failure = UpgradeFailure::from_error(&error);
        assert_eq!(failure.status, Some(400));
        assert_eq!(failure.to_string(), "http_upgrade; HTTP 400");
        assert!(!format!("{failure:?}").contains("secret-sentinel"));
        let io = Error::Io(std::io::Error::other("secret-sentinel"));
        assert_eq!(
            UpgradeFailure::from_error(&io).to_string(),
            "network_or_tls"
        );
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
