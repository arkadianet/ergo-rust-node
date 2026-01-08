//! Connection handling.

use crate::{NetworkError, NetworkResult, Message, PeerId};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, instrument};

/// Connection configuration.
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    /// Connection timeout.
    pub connect_timeout: Duration,
    /// Read timeout.
    pub read_timeout: Duration,
    /// Write timeout.
    pub write_timeout: Duration,
    /// Maximum message size.
    pub max_message_size: usize,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(10),
            max_message_size: 10 * 1024 * 1024, // 10 MB
        }
    }
}

/// A P2P connection.
pub struct Connection {
    /// Peer ID.
    pub peer_id: PeerId,
    /// Remote address.
    pub addr: SocketAddr,
    /// TCP stream.
    stream: TcpStream,
    /// Configuration.
    config: ConnectionConfig,
}

impl Connection {
    /// Create a new connection from an accepted socket.
    pub fn new(stream: TcpStream, addr: SocketAddr, config: ConnectionConfig) -> Self {
        Self {
            peer_id: PeerId::from_addr(&addr),
            addr,
            stream,
            config,
        }
    }

    /// Connect to a remote peer.
    #[instrument(skip(config))]
    pub async fn connect(addr: SocketAddr, config: ConnectionConfig) -> NetworkResult<Self> {
        let stream = tokio::time::timeout(
            config.connect_timeout,
            TcpStream::connect(addr),
        )
        .await
        .map_err(|_| NetworkError::Timeout("Connection timeout".to_string()))?
        .map_err(NetworkError::Io)?;

        debug!("Connected to {}", addr);

        Ok(Self::new(stream, addr, config))
    }

    /// Send a message.
    #[instrument(skip(self, message))]
    pub async fn send(&mut self, message: Message) -> NetworkResult<()> {
        let bytes = message.encode()?;

        // Send length prefix
        let len = bytes.len() as u32;
        self.stream.write_all(&len.to_be_bytes()).await?;

        // Send message
        self.stream.write_all(&bytes).await?;
        self.stream.flush().await?;

        debug!(msg_type = ?message.message_type(), len = bytes.len(), "Sent message");
        Ok(())
    }

    /// Receive a message.
    #[instrument(skip(self))]
    pub async fn receive(&mut self) -> NetworkResult<Message> {
        // Read length prefix
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;

        if len > self.config.max_message_size {
            return Err(NetworkError::MessageTooLarge {
                size: len,
                max: self.config.max_message_size,
            });
        }

        // Read message
        let mut buf = vec![0u8; len];
        self.stream.read_exact(&mut buf).await?;

        let message = Message::decode(buf.into())?;
        debug!(msg_type = ?message.message_type(), "Received message");

        Ok(message)
    }

    /// Close the connection.
    pub async fn close(mut self) -> NetworkResult<()> {
        self.stream.shutdown().await?;
        debug!("Connection closed");
        Ok(())
    }

    /// Get the peer address.
    pub fn peer_addr(&self) -> SocketAddr {
        self.addr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connection_config_defaults() {
        let config = ConnectionConfig::default();
        assert_eq!(config.connect_timeout, Duration::from_secs(10));
        assert_eq!(config.max_message_size, 10 * 1024 * 1024);
    }
}
