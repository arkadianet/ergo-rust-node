//! # ergo-network
//!
//! P2P networking layer for the Ergo blockchain.
//!
//! This crate provides:
//! - TCP connection management
//! - Ergo P2P protocol messages
//! - Peer discovery and management
//! - Connection pool and scoring

mod codec;
mod connection;
pub mod discovery;
mod error;
mod handshake;
mod message;
mod peer;
mod service;

pub use codec::{MessageCodec, PeerAddress, PeerFeature};
pub use connection::{Connection, ConnectionConfig};
pub use discovery::{NetworkType, PeerDiscovery, MAINNET_DNS_SEEDS, TESTNET_DNS_SEEDS};
pub use error::{NetworkError, NetworkResult};
pub use handshake::{DeclaredAddress, HandshakeCodec, HandshakeData, PeerSpec};
pub use message::{
    Handshake, InvData, ManifestData, ManifestRequest, Message, MessageType, ModifierData,
    ModifierItem, ModifierRequest, SnapshotsInfo, SyncInfo, UtxoChunkData, UtxoChunkRequest,
    MAX_CHUNK_SIZE, MAX_MANIFEST_SIZE, MAX_SNAPSHOTS_INFO_ENTRIES,
};
pub use peer::{PeerId, PeerInfo, PeerManager, PeerState};
pub use service::{NetworkCommand, NetworkConfig, NetworkEvent, NetworkService, PeerHandle};

/// Default P2P port.
pub const DEFAULT_PORT: u16 = 9030;

/// Protocol magic bytes for mainnet.
pub const MAINNET_MAGIC: [u8; 4] = [0x01, 0x00, 0x02, 0x04];

/// Protocol magic bytes for testnet.
pub const TESTNET_MAGIC: [u8; 4] = [0x02, 0x00, 0x02, 0x04];

/// Maximum message size.
pub const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024; // 10 MB

/// Protocol version.
pub const PROTOCOL_VERSION: (u8, u8, u8) = (6, 0, 1);
