//! HTTP/2 tunnel client for connecting to shadow databases.
//!
//! ## Latency Note
//!
//! The tunnel adds 2-10ms RTT per protocol message compared to direct connections.
//! This is acceptable for migration workloads but interactive psql sessions will
//! feel slightly sluggish. For latency-sensitive use, consider SSH port forwarding.

use bytes::Bytes;
use futures::StreamExt;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use hyper::Request;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::convert::Infallible;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

/// Client for establishing HTTP/2 tunnels to shadow databases.
pub struct TunnelClient {
    api_url: String,
    token: String,
    shadow_id: String,
}

impl TunnelClient {
    /// Create a new tunnel client.
    pub fn new(api_url: String, token: String, shadow_id: String) -> Self {
        Self {
            api_url,
            token,
            shadow_id,
        }
    }

    /// Start a local TCP listener that tunnels connections to the shadow database.
    pub async fn listen(&self, port: u16) -> Result<TcpListener, TunnelError> {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
            .await
            .map_err(|e| TunnelError::BindFailed(e.to_string()))?;
        Ok(listener)
    }

    /// Handle a single incoming connection by establishing tunnel to shadow.
    pub async fn handle_connection(&self, local_stream: TcpStream) -> Result<(), TunnelError> {
        let (local_read, local_write) = local_stream.into_split();

        // Create channel for local -> remote data
        let (to_remote_tx, to_remote_rx) = mpsc::channel::<Bytes>(32);

        // Build HTTP/2 client with rustls
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_native_roots()
            .map_err(|e| TunnelError::TlsError(e.to_string()))?
            .https_or_http()
            .enable_http2()
            .build();

        let client = Client::builder(TokioExecutor::new())
            .http2_only(true)
            .build(https);

        // Create request body stream from channel
        let body_stream = tokio_stream::wrappers::ReceiverStream::new(to_remote_rx)
            .map(|bytes| Ok::<_, Infallible>(Frame::data(bytes)));
        let request_body = StreamBody::new(body_stream);

        // Build tunnel request
        let uri = format!(
            "{}/api/v1/shadows/{}/tunnel",
            self.api_url, self.shadow_id
        );
        let request = Request::builder()
            .method("POST")
            .uri(&uri)
            .header("authorization", format!("Bearer {}", self.token))
            .header("content-type", "application/octet-stream")
            .body(request_body)
            .map_err(|e| TunnelError::RequestFailed(e.to_string()))?;

        // Spawn task to read from local and send to channel
        let _local_to_remote = tokio::spawn(async move {
            let mut local_read = local_read;
            let mut buf = vec![0u8; 8192];
            loop {
                match local_read.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if to_remote_tx
                            .send(Bytes::copy_from_slice(&buf[..n]))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Send request and get response
        let response = client
            .request(request)
            .await
            .map_err(|e| TunnelError::ConnectionFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(TunnelError::ServerError(format!(
                "Server returned {}",
                response.status()
            )));
        }

        // Stream response body to local socket
        let mut body = response.into_body();
        let mut local_write = local_write;

        while let Some(frame_result) = body.frame().await {
            match frame_result {
                Ok(frame) => {
                    if let Some(data) = frame.data_ref() {
                        if local_write.write_all(data).await.is_err() {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }

        Ok(())
    }
}

/// Tunnel-specific errors.
#[derive(Debug, thiserror::Error)]
pub enum TunnelError {
    #[error("Failed to bind local port: {0}")]
    BindFailed(String),
    #[error("TLS error: {0}")]
    TlsError(String),
    #[error("Failed to build request: {0}")]
    RequestFailed(String),
    #[error("Failed to connect to server: {0}")]
    ConnectionFailed(String),
    #[error("Server error: {0}")]
    ServerError(String),
}
