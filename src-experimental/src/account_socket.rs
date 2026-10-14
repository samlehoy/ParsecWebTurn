//! Native account signaling socket, independent of the WASM/WebRTC socket.
use anyhow::{Result, anyhow, ensure};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, protocol::WebSocketConfig},
};
use url::Url;

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
const MESSAGE_LIMIT: usize = 16 * 1024;

pub struct AccountSocket {
    socket: Socket,
}

fn authenticated_url(endpoint: &str, token: &str) -> Result<Url> {
    let mut url = Url::parse(endpoint).map_err(|_| anyhow!("invalid native signaling endpoint"))?;
    ensure!(
        url.scheme() == "wss" && url.host_str().is_some(),
        "native signaling requires WSS"
    );
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "native signaling endpoint must not embed credentials or query data"
    );
    ensure!(
        !token.is_empty() && token.len() <= 4096,
        "invalid native account session token"
    );
    url.query_pairs_mut()
        .append_pair("session_id", token)
        .append_pair("role", "client")
        .append_pair("version", "1")
        .append_pair("build", "ParsecWebTurn-experimental-bud")
        .append_pair("sdk_version", "0");
    Ok(url)
}

impl AccountSocket {
    /// Token comes from an authenticated account session, not from the native
    /// BUD media keys. Never log the URL, offer JSON, or websocket error text
    /// that could embed the authenticated request.
    pub async fn connect(endpoint: &str, token: &str) -> Result<Self> {
        let url = authenticated_url(endpoint, token)?;
        let config = WebSocketConfig::default()
            .write_buffer_size(0)
            .max_write_buffer_size(MESSAGE_LIMIT * 2)
            .max_message_size(Some(MESSAGE_LIMIT))
            .max_frame_size(Some(MESSAGE_LIMIT));
        // Select this socket's provider explicitly. Never change the process's
        // global TLS provider or the existing WebRTC account socket.
        let roots =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| anyhow!("native signaling TLS configuration failed"))?
        .with_root_certificates(roots)
        .with_no_client_auth();
        let (socket, _) = tokio::time::timeout(
            Duration::from_secs(10),
            tokio_tungstenite::connect_async_tls_with_config(
                url.as_str(),
                Some(config),
                true,
                Some(tokio_tungstenite::Connector::Rustls(Arc::new(tls))),
            ),
        )
        .await
        .map_err(|_| anyhow!("native account signaling connection timed out"))?
        .map_err(|_| anyhow!("native account signaling connection failed"))?;
        Ok(Self { socket })
    }

    pub async fn send(&mut self, message: &Value) -> Result<()> {
        let text = serde_json::to_string(message)?;
        ensure!(
            text.len() <= MESSAGE_LIMIT,
            "native signaling output too large"
        );
        tokio::time::timeout(
            Duration::from_secs(5),
            self.socket.send(Message::Text(text.into())),
        )
        .await
        .map_err(|_| anyhow!("native signaling send timed out"))?
        .map_err(|_| anyhow!("native signaling send failed"))
    }

    pub async fn receive(&mut self) -> Result<String> {
        loop {
            let message = self
                .socket
                .next()
                .await
                .ok_or_else(|| anyhow!("native signaling closed"))?
                .map_err(|_| anyhow!("native signaling receive failed"))?;
            match message {
                Message::Text(text) => return Ok(text.to_string()),
                Message::Close(_) => anyhow::bail!("native signaling closed"),
                Message::Binary(_) => {
                    anyhow::bail!("native signaling received a non-JSON binary message")
                }
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    }

    pub async fn close(mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(2), self.socket.close(None)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_token_is_encoded_only_into_the_native_request() {
        let url = authenticated_url(
            "wss://signaling.example.test/",
            "dummy+token&not-a-parameter",
        )
        .unwrap();
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(pairs["session_id"], "dummy+token&not-a-parameter");
        assert_eq!(pairs["role"], "client");
        assert_eq!(pairs.len(), 5);
    }
    #[test]
    fn no_plaintext_or_embedded_secret_endpoint() {
        for endpoint in [
            "ws://example.test/",
            "https://example.test/",
            "wss://user:secret@example.test/",
            "wss://example.test/?session_id=secret",
            "wss://example.test/#secret",
        ] {
            assert!(authenticated_url(endpoint, "dummy").is_err());
        }
        assert!(authenticated_url("wss://example.test/", "").is_err());
    }
}
