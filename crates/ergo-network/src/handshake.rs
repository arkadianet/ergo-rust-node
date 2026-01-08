//! Ergo P2P handshake protocol.
//!
//! The handshake is sent as raw bytes (no framing) at connection start.
//! Format:
//! - Timestamp (VLQ encoded)
//! - Agent name length (1 byte) + Agent name (UTF-8)
//! - Version (3 bytes: major, minor, patch)
//! - Peer name length (1 byte) + Peer name (UTF-8)
//! - Declared address (Option: 0=None, 1=Some with address data)
//! - Features (count as 1 byte, then each feature: id + VLQ length + data)

use crate::{NetworkError, NetworkResult};
use bytes::BytesMut;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{SystemTime, UNIX_EPOCH};

/// Maximum handshake size.
const MAX_HANDSHAKE_SIZE: usize = 8096;

/// Declared network address.
#[derive(Debug, Clone)]
pub struct DeclaredAddress {
    pub ip: IpAddr,
    pub port: u16,
}

impl DeclaredAddress {
    pub fn to_socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.ip, self.port)
    }
}

/// Handshake data.
#[derive(Debug, Clone)]
pub struct HandshakeData {
    /// Timestamp (milliseconds since epoch).
    pub time: u64,
    /// Agent name (e.g., "ergoref").
    pub agent_name: String,
    /// Protocol version.
    pub version: (u8, u8, u8),
    /// Peer name.
    pub peer_name: String,
    /// Declared address (optional).
    pub declared_addr: Option<DeclaredAddress>,
    /// Features bytes (raw, for forwarding).
    pub features: Vec<PeerFeature>,
}

/// Peer feature.
#[derive(Debug, Clone)]
pub enum PeerFeature {
    /// Mode feature (ID 16): state type, verifying, nipopow suffix, blocks to keep
    Mode {
        state_type: u8,
        verifying: bool,
        nipopow_suffix: Option<i32>,
        blocks_to_keep: i32,
    },
    /// Session feature (ID 3): magic bytes and session ID
    Session { magic: [u8; 4], session_id: u64 },
    /// Local address feature (ID 2)
    LocalAddress { ip: IpAddr, port: u16 },
    /// REST API URL feature (ID 4)
    RestApiUrl(String),
    /// Unknown feature
    Unknown { id: u8, data: Vec<u8> },
}

impl HandshakeData {
    /// Create a new handshake.
    pub fn new(agent_name: String, version: (u8, u8, u8), peer_name: String) -> Self {
        Self {
            time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            agent_name,
            version,
            peer_name,
            declared_addr: None,
            features: Vec::new(),
        }
    }

    /// Create handshake with standard features for mainnet.
    pub fn with_mainnet_features(mut self, session_id: u64) -> Self {
        // Mode feature: UTXO state, verifying transactions, no nipopow, keep all blocks
        self.features.push(PeerFeature::Mode {
            state_type: 0, // UTXO
            verifying: true,
            nipopow_suffix: None,
            blocks_to_keep: -1, // All blocks
        });

        // Session feature with mainnet magic
        self.features.push(PeerFeature::Session {
            magic: [1, 0, 2, 4],
            session_id,
        });

        self
    }

    /// Serialize handshake to bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(256);

        // Timestamp (VLQ encoding)
        vlq_encode(&mut buf, self.time);

        // Agent name
        let agent_bytes = self.agent_name.as_bytes();
        buf.push(agent_bytes.len() as u8);
        buf.extend_from_slice(agent_bytes);

        // Version (3 bytes)
        buf.push(self.version.0);
        buf.push(self.version.1);
        buf.push(self.version.2);

        // Peer name
        let peer_bytes = self.peer_name.as_bytes();
        buf.push(peer_bytes.len() as u8);
        buf.extend_from_slice(peer_bytes);

        // Declared address (Option)
        if let Some(ref addr) = self.declared_addr {
            buf.push(1); // Some
            match addr.ip {
                IpAddr::V4(ipv4) => {
                    buf.push(8); // Size indicator for IPv4 (4 + nominal 4 for port)
                    buf.extend_from_slice(&ipv4.octets());
                }
                IpAddr::V6(ipv6) => {
                    buf.push(20); // Size indicator for IPv6 (16 + nominal 4 for port)
                    buf.extend_from_slice(&ipv6.octets());
                }
            }
            vlq_encode(&mut buf, addr.port as u64);
        } else {
            buf.push(0); // None
        }

        // Features
        buf.push(self.features.len() as u8);
        for feature in &self.features {
            match feature {
                PeerFeature::Mode {
                    state_type,
                    verifying,
                    nipopow_suffix,
                    blocks_to_keep,
                } => {
                    buf.push(16); // Mode feature ID
                    let mut mode_data = Vec::new();
                    mode_data.push(*state_type);
                    mode_data.push(if *verifying { 1 } else { 0 });
                    if let Some(suffix) = nipopow_suffix {
                        mode_data.push(1); // Some
                        mode_data.extend_from_slice(&suffix.to_be_bytes());
                    } else {
                        mode_data.push(0); // None
                    }
                    vlq_encode_signed(&mut mode_data, *blocks_to_keep as i64);
                    vlq_encode(&mut buf, mode_data.len() as u64);
                    buf.extend_from_slice(&mode_data);
                }
                PeerFeature::Session { magic, session_id } => {
                    buf.push(3); // Session feature ID
                    let mut session_data = Vec::new();
                    session_data.extend_from_slice(magic);
                    vlq_encode(&mut session_data, *session_id); // Session ID is VLQ encoded
                    vlq_encode(&mut buf, session_data.len() as u64);
                    buf.extend_from_slice(&session_data);
                }
                PeerFeature::LocalAddress { ip, port } => {
                    buf.push(2); // Local address feature ID
                    let mut addr_data = Vec::new();
                    match ip {
                        IpAddr::V4(ipv4) => addr_data.extend_from_slice(&ipv4.octets()),
                        IpAddr::V6(ipv6) => addr_data.extend_from_slice(&ipv6.octets()),
                    }
                    addr_data.extend_from_slice(&(*port as u32).to_be_bytes());
                    vlq_encode(&mut buf, addr_data.len() as u64);
                    buf.extend_from_slice(&addr_data);
                }
                PeerFeature::RestApiUrl(url) => {
                    buf.push(4); // REST API URL feature ID
                    let url_bytes = url.as_bytes();
                    vlq_encode(&mut buf, url_bytes.len() as u64 + 1);
                    buf.push(url_bytes.len() as u8);
                    buf.extend_from_slice(url_bytes);
                }
                PeerFeature::Unknown { id, data } => {
                    buf.push(*id);
                    vlq_encode(&mut buf, data.len() as u64);
                    buf.extend_from_slice(data);
                }
            }
        }

        buf
    }

    /// Parse handshake from bytes.
    pub fn parse(data: &[u8]) -> NetworkResult<Self> {
        if data.len() > MAX_HANDSHAKE_SIZE {
            return Err(NetworkError::InvalidMessage(format!(
                "Handshake too large: {} > {}",
                data.len(),
                MAX_HANDSHAKE_SIZE
            )));
        }

        let mut pos = 0;

        // Timestamp (VLQ)
        let (time, new_pos) = vlq_decode(data, pos)?;
        pos = new_pos;

        // Agent name
        if pos >= data.len() {
            return Err(NetworkError::InvalidMessage("Truncated handshake".into()));
        }
        let agent_len = data[pos] as usize;
        pos += 1;
        if pos + agent_len > data.len() {
            return Err(NetworkError::InvalidMessage("Truncated agent name".into()));
        }
        let agent_name = String::from_utf8_lossy(&data[pos..pos + agent_len]).to_string();
        pos += agent_len;

        // Version (3 bytes)
        if pos + 3 > data.len() {
            return Err(NetworkError::InvalidMessage("Truncated version".into()));
        }
        let version = (data[pos], data[pos + 1], data[pos + 2]);
        pos += 3;

        // Peer name
        if pos >= data.len() {
            return Err(NetworkError::InvalidMessage("Truncated handshake".into()));
        }
        let peer_len = data[pos] as usize;
        pos += 1;
        if pos + peer_len > data.len() {
            return Err(NetworkError::InvalidMessage("Truncated peer name".into()));
        }
        let peer_name = String::from_utf8_lossy(&data[pos..pos + peer_len]).to_string();
        pos += peer_len;

        // Declared address (Option)
        let declared_addr = if pos < data.len() && data[pos] == 1 {
            pos += 1; // Skip Some marker

            if pos >= data.len() {
                return Err(NetworkError::InvalidMessage(
                    "Truncated declared address".into(),
                ));
            }

            let addr_type = data[pos];
            pos += 1;

            let ip = if addr_type == 8 {
                // IPv4 (4 bytes) + port
                if pos + 4 > data.len() {
                    return Err(NetworkError::InvalidMessage(
                        "Truncated IPv4 address".into(),
                    ));
                }
                let ip = IpAddr::V4(Ipv4Addr::new(
                    data[pos],
                    data[pos + 1],
                    data[pos + 2],
                    data[pos + 3],
                ));
                pos += 4;
                ip
            } else if addr_type == 20 {
                // IPv6 (16 bytes) + port
                if pos + 16 > data.len() {
                    return Err(NetworkError::InvalidMessage(
                        "Truncated IPv6 address".into(),
                    ));
                }
                let mut octets = [0u8; 16];
                octets.copy_from_slice(&data[pos..pos + 16]);
                let ip = IpAddr::V6(octets.into());
                pos += 16;
                ip
            } else {
                return Err(NetworkError::InvalidMessage(format!(
                    "Unknown address type: {}",
                    addr_type
                )));
            };

            // Port is VLQ encoded
            let (port, new_pos) = vlq_decode(data, pos)?;
            pos = new_pos;

            Some(DeclaredAddress {
                ip,
                port: port as u16,
            })
        } else if pos < data.len() {
            pos += 1; // Skip None marker
            None
        } else {
            None
        };

        // Features
        let mut features = Vec::new();
        if pos < data.len() {
            let feature_count = data[pos];
            pos += 1;

            for _ in 0..feature_count {
                if pos >= data.len() {
                    break;
                }

                let feature_id = data[pos];
                pos += 1;

                let (feature_len, new_pos) = vlq_decode(data, pos)?;
                pos = new_pos;

                if pos + feature_len as usize > data.len() {
                    return Err(NetworkError::InvalidMessage(
                        "Truncated feature data".into(),
                    ));
                }

                let feature_data = &data[pos..pos + feature_len as usize];
                pos += feature_len as usize;

                let feature = match feature_id {
                    16 => {
                        // Mode feature
                        if feature_data.len() < 3 {
                            PeerFeature::Unknown {
                                id: feature_id,
                                data: feature_data.to_vec(),
                            }
                        } else {
                            let state_type = feature_data[0];
                            let verifying = feature_data[1] != 0;
                            let nipopow_opt = feature_data[2];
                            let (nipopow_suffix, blocks_pos) = if nipopow_opt == 1 {
                                if feature_data.len() >= 7 {
                                    let suffix = i32::from_be_bytes([
                                        feature_data[3],
                                        feature_data[4],
                                        feature_data[5],
                                        feature_data[6],
                                    ]);
                                    (Some(suffix), 7)
                                } else {
                                    (None, 3)
                                }
                            } else {
                                (None, 3)
                            };

                            let (blocks_to_keep, _) =
                                vlq_decode_signed(feature_data, blocks_pos).unwrap_or((1, 0));

                            PeerFeature::Mode {
                                state_type,
                                verifying,
                                nipopow_suffix,
                                blocks_to_keep: blocks_to_keep as i32,
                            }
                        }
                    }
                    3 => {
                        // Session feature: magic (4 bytes) + session_id (VLQ)
                        if feature_data.len() >= 5 {
                            let mut magic = [0u8; 4];
                            magic.copy_from_slice(&feature_data[0..4]);
                            // Session ID is VLQ encoded
                            let (session_id, _) = vlq_decode(feature_data, 4).unwrap_or((0, 4));
                            PeerFeature::Session { magic, session_id }
                        } else {
                            PeerFeature::Unknown {
                                id: feature_id,
                                data: feature_data.to_vec(),
                            }
                        }
                    }
                    2 => {
                        // Local address feature
                        if feature_data.len() == 8 {
                            let ip = IpAddr::V4(Ipv4Addr::new(
                                feature_data[0],
                                feature_data[1],
                                feature_data[2],
                                feature_data[3],
                            ));
                            let port = u32::from_be_bytes([
                                feature_data[4],
                                feature_data[5],
                                feature_data[6],
                                feature_data[7],
                            ]) as u16;
                            PeerFeature::LocalAddress { ip, port }
                        } else {
                            PeerFeature::Unknown {
                                id: feature_id,
                                data: feature_data.to_vec(),
                            }
                        }
                    }
                    4 => {
                        // REST API URL feature
                        if !feature_data.is_empty() {
                            let url_len = feature_data[0] as usize;
                            if feature_data.len() >= 1 + url_len {
                                let url = String::from_utf8_lossy(&feature_data[1..1 + url_len])
                                    .to_string();
                                PeerFeature::RestApiUrl(url)
                            } else {
                                PeerFeature::Unknown {
                                    id: feature_id,
                                    data: feature_data.to_vec(),
                                }
                            }
                        } else {
                            PeerFeature::Unknown {
                                id: feature_id,
                                data: feature_data.to_vec(),
                            }
                        }
                    }
                    _ => PeerFeature::Unknown {
                        id: feature_id,
                        data: feature_data.to_vec(),
                    },
                };

                features.push(feature);
            }
        }

        Ok(Self {
            time,
            agent_name,
            version,
            peer_name,
            declared_addr,
            features,
        })
    }

    /// Get the session magic bytes from features.
    pub fn get_magic(&self) -> Option<[u8; 4]> {
        for feature in &self.features {
            if let PeerFeature::Session { magic, .. } = feature {
                return Some(*magic);
            }
        }
        None
    }

    /// Get the session ID from features.
    pub fn get_session_id(&self) -> Option<u64> {
        for feature in &self.features {
            if let PeerFeature::Session { session_id, .. } = feature {
                return Some(*session_id);
            }
        }
        None
    }
}

/// VLQ encode an unsigned integer.
fn vlq_encode(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// VLQ encode a signed integer using ZigZag encoding.
fn vlq_encode_signed(buf: &mut Vec<u8>, value: i64) {
    // ZigZag encoding: (value << 1) ^ (value >> 63)
    let encoded = ((value << 1) ^ (value >> 63)) as u64;
    vlq_encode(buf, encoded);
}

/// VLQ decode an unsigned integer.
fn vlq_decode(data: &[u8], mut pos: usize) -> NetworkResult<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0;

    loop {
        if pos >= data.len() {
            return Err(NetworkError::InvalidMessage("Truncated VLQ".into()));
        }
        let byte = data[pos];
        pos += 1;

        result |= ((byte & 0x7F) as u64) << shift;

        if (byte & 0x80) == 0 {
            break;
        }
        shift += 7;

        if shift > 63 {
            return Err(NetworkError::InvalidMessage("VLQ overflow".into()));
        }
    }

    Ok((result, pos))
}

/// VLQ decode a signed integer using ZigZag encoding.
fn vlq_decode_signed(data: &[u8], pos: usize) -> NetworkResult<(i64, usize)> {
    let (encoded, new_pos) = vlq_decode(data, pos)?;
    // ZigZag decoding: (encoded >> 1) ^ (-(encoded & 1))
    let value = ((encoded >> 1) as i64) ^ (-((encoded & 1) as i64));
    Ok((value, new_pos))
}

/// Handshake codec for raw (unframed) handshake messages.
pub struct HandshakeCodec {
    /// Whether we've completed the handshake.
    handshake_complete: bool,
    /// Buffer for accumulating handshake data.
    buffer: BytesMut,
}

impl HandshakeCodec {
    pub fn new() -> Self {
        Self {
            handshake_complete: false,
            buffer: BytesMut::with_capacity(1024),
        }
    }

    /// Read a handshake from the stream.
    pub async fn read_handshake<R: tokio::io::AsyncReadExt + Unpin>(
        &mut self,
        reader: &mut R,
    ) -> NetworkResult<HandshakeData> {
        let mut buf = [0u8; 1024];

        loop {
            // Try to parse what we have
            if !self.buffer.is_empty() {
                match HandshakeData::parse(&self.buffer) {
                    Ok(handshake) => {
                        self.handshake_complete = true;
                        return Ok(handshake);
                    }
                    Err(_) => {
                        // Need more data, continue reading
                    }
                }
            }

            // Check size limit
            if self.buffer.len() > MAX_HANDSHAKE_SIZE {
                return Err(NetworkError::InvalidMessage("Handshake too large".into()));
            }

            // Read more data
            let n = reader
                .read(&mut buf)
                .await
                .map_err(|e| NetworkError::Io(std::io::Error::new(e.kind(), e.to_string())))?;

            if n == 0 {
                return Err(NetworkError::ConnectionClosed);
            }

            self.buffer.extend_from_slice(&buf[..n]);

            // Try parsing after each read
            if let Ok(handshake) = HandshakeData::parse(&self.buffer) {
                self.handshake_complete = true;
                return Ok(handshake);
            }
        }
    }

    /// Write a handshake to the stream.
    pub async fn write_handshake<W: tokio::io::AsyncWriteExt + Unpin>(
        &self,
        writer: &mut W,
        handshake: &HandshakeData,
    ) -> NetworkResult<()> {
        let data = handshake.serialize();
        writer
            .write_all(&data)
            .await
            .map_err(|e| NetworkError::Io(std::io::Error::new(e.kind(), e.to_string())))?;
        writer
            .flush()
            .await
            .map_err(|e| NetworkError::Io(std::io::Error::new(e.kind(), e.to_string())))?;
        Ok(())
    }
}

impl Default for HandshakeCodec {
    fn default() -> Self {
        Self::new()
    }
}

/// Peer specification for the Peers message.
/// This is similar to HandshakeData but without the timestamp.
#[derive(Debug, Clone)]
pub struct PeerSpec {
    /// Agent name (e.g., "ergoref").
    pub agent_name: String,
    /// Protocol version.
    pub version: (u8, u8, u8),
    /// Peer name.
    pub peer_name: String,
    /// Declared address (optional).
    pub declared_addr: Option<DeclaredAddress>,
    /// Features.
    pub features: Vec<PeerFeature>,
}

impl PeerSpec {
    /// Create a new PeerSpec.
    pub fn new(
        agent_name: String,
        version: (u8, u8, u8),
        peer_name: String,
        declared_addr: Option<DeclaredAddress>,
    ) -> Self {
        Self {
            agent_name,
            version,
            peer_name,
            declared_addr,
            features: Vec::new(),
        }
    }

    /// Create a PeerSpec from HandshakeData.
    pub fn from_handshake(handshake: &HandshakeData) -> Self {
        Self {
            agent_name: handshake.agent_name.clone(),
            version: handshake.version,
            peer_name: handshake.peer_name.clone(),
            declared_addr: handshake.declared_addr.clone(),
            features: handshake.features.clone(),
        }
    }

    /// Serialize PeerSpec to bytes (same as handshake but without timestamp).
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(256);

        // Agent name
        let agent_bytes = self.agent_name.as_bytes();
        buf.push(agent_bytes.len() as u8);
        buf.extend_from_slice(agent_bytes);

        // Version (3 bytes)
        buf.push(self.version.0);
        buf.push(self.version.1);
        buf.push(self.version.2);

        // Peer name
        let peer_bytes = self.peer_name.as_bytes();
        buf.push(peer_bytes.len() as u8);
        buf.extend_from_slice(peer_bytes);

        // Declared address (Option)
        if let Some(ref addr) = self.declared_addr {
            buf.push(1); // Some
            match addr.ip {
                IpAddr::V4(ipv4) => {
                    buf.push(8); // Size indicator for IPv4 (4 + nominal 4 for port)
                    buf.extend_from_slice(&ipv4.octets());
                }
                IpAddr::V6(ipv6) => {
                    buf.push(20); // Size indicator for IPv6 (16 + nominal 4 for port)
                    buf.extend_from_slice(&ipv6.octets());
                }
            }
            vlq_encode(&mut buf, addr.port as u64);
        } else {
            buf.push(0); // None
        }

        // Features
        buf.push(self.features.len() as u8);
        for feature in &self.features {
            match feature {
                PeerFeature::Mode {
                    state_type,
                    verifying,
                    nipopow_suffix,
                    blocks_to_keep,
                } => {
                    buf.push(16); // Mode feature ID
                    let mut mode_data = Vec::new();
                    mode_data.push(*state_type);
                    mode_data.push(if *verifying { 1 } else { 0 });
                    if let Some(suffix) = nipopow_suffix {
                        mode_data.push(1); // Some
                        mode_data.extend_from_slice(&suffix.to_be_bytes());
                    } else {
                        mode_data.push(0); // None
                    }
                    vlq_encode_signed(&mut mode_data, *blocks_to_keep as i64);
                    vlq_encode(&mut buf, mode_data.len() as u64);
                    buf.extend_from_slice(&mode_data);
                }
                PeerFeature::Session { magic, session_id } => {
                    buf.push(3); // Session feature ID
                    let mut session_data = Vec::new();
                    session_data.extend_from_slice(magic);
                    vlq_encode(&mut session_data, *session_id);
                    vlq_encode(&mut buf, session_data.len() as u64);
                    buf.extend_from_slice(&session_data);
                }
                PeerFeature::LocalAddress { ip, port } => {
                    buf.push(2); // Local address feature ID
                    let mut addr_data = Vec::new();
                    match ip {
                        IpAddr::V4(ipv4) => addr_data.extend_from_slice(&ipv4.octets()),
                        IpAddr::V6(ipv6) => addr_data.extend_from_slice(&ipv6.octets()),
                    }
                    addr_data.extend_from_slice(&(*port as u32).to_be_bytes());
                    vlq_encode(&mut buf, addr_data.len() as u64);
                    buf.extend_from_slice(&addr_data);
                }
                PeerFeature::RestApiUrl(url) => {
                    buf.push(4); // REST API URL feature ID
                    let url_bytes = url.as_bytes();
                    vlq_encode(&mut buf, url_bytes.len() as u64 + 1);
                    buf.push(url_bytes.len() as u8);
                    buf.extend_from_slice(url_bytes);
                }
                PeerFeature::Unknown { id, data } => {
                    buf.push(*id);
                    vlq_encode(&mut buf, data.len() as u64);
                    buf.extend_from_slice(data);
                }
            }
        }

        buf
    }

    /// Parse PeerSpec from bytes.
    pub fn parse(data: &[u8], mut pos: usize) -> NetworkResult<(Self, usize)> {
        // Agent name
        if pos >= data.len() {
            return Err(NetworkError::InvalidMessage("Truncated PeerSpec".into()));
        }
        let agent_len = data[pos] as usize;
        pos += 1;
        if pos + agent_len > data.len() {
            return Err(NetworkError::InvalidMessage("Truncated agent name".into()));
        }
        let agent_name = String::from_utf8_lossy(&data[pos..pos + agent_len]).to_string();
        pos += agent_len;

        // Version (3 bytes)
        if pos + 3 > data.len() {
            return Err(NetworkError::InvalidMessage("Truncated version".into()));
        }
        let version = (data[pos], data[pos + 1], data[pos + 2]);
        pos += 3;

        // Peer name
        if pos >= data.len() {
            return Err(NetworkError::InvalidMessage("Truncated PeerSpec".into()));
        }
        let peer_len = data[pos] as usize;
        pos += 1;
        if pos + peer_len > data.len() {
            return Err(NetworkError::InvalidMessage("Truncated peer name".into()));
        }
        let peer_name = String::from_utf8_lossy(&data[pos..pos + peer_len]).to_string();
        pos += peer_len;

        // Declared address (Option)
        let declared_addr = if pos < data.len() && data[pos] == 1 {
            pos += 1; // Skip Some marker

            if pos >= data.len() {
                return Err(NetworkError::InvalidMessage(
                    "Truncated declared address".into(),
                ));
            }

            let addr_type = data[pos];
            pos += 1;

            let ip = if addr_type == 8 {
                // IPv4 (4 bytes) + port
                if pos + 4 > data.len() {
                    return Err(NetworkError::InvalidMessage(
                        "Truncated IPv4 address".into(),
                    ));
                }
                let ip = IpAddr::V4(std::net::Ipv4Addr::new(
                    data[pos],
                    data[pos + 1],
                    data[pos + 2],
                    data[pos + 3],
                ));
                pos += 4;
                ip
            } else if addr_type == 20 {
                // IPv6 (16 bytes) + port
                if pos + 16 > data.len() {
                    return Err(NetworkError::InvalidMessage(
                        "Truncated IPv6 address".into(),
                    ));
                }
                let mut octets = [0u8; 16];
                octets.copy_from_slice(&data[pos..pos + 16]);
                let ip = IpAddr::V6(octets.into());
                pos += 16;
                ip
            } else {
                return Err(NetworkError::InvalidMessage(format!(
                    "Unknown address type: {}",
                    addr_type
                )));
            };

            // Port is VLQ encoded
            let (port, new_pos) = vlq_decode(data, pos)?;
            pos = new_pos;

            Some(DeclaredAddress {
                ip,
                port: port as u16,
            })
        } else if pos < data.len() {
            pos += 1; // Skip None marker
            None
        } else {
            None
        };

        // Features
        let mut features = Vec::new();
        if pos < data.len() {
            let feature_count = data[pos];
            pos += 1;

            for _ in 0..feature_count {
                if pos >= data.len() {
                    break;
                }

                let feature_id = data[pos];
                pos += 1;

                let (feature_len, new_pos) = vlq_decode(data, pos)?;
                pos = new_pos;

                if pos + feature_len as usize > data.len() {
                    return Err(NetworkError::InvalidMessage(
                        "Truncated feature data".into(),
                    ));
                }

                let feature_data = &data[pos..pos + feature_len as usize];
                pos += feature_len as usize;

                let feature = match feature_id {
                    16 => {
                        // Mode feature
                        if feature_data.len() < 3 {
                            PeerFeature::Unknown {
                                id: feature_id,
                                data: feature_data.to_vec(),
                            }
                        } else {
                            let state_type = feature_data[0];
                            let verifying = feature_data[1] != 0;
                            let nipopow_opt = feature_data[2];
                            let (nipopow_suffix, blocks_pos) = if nipopow_opt == 1 {
                                if feature_data.len() >= 7 {
                                    let suffix = i32::from_be_bytes([
                                        feature_data[3],
                                        feature_data[4],
                                        feature_data[5],
                                        feature_data[6],
                                    ]);
                                    (Some(suffix), 7)
                                } else {
                                    (None, 3)
                                }
                            } else {
                                (None, 3)
                            };

                            let (blocks_to_keep, _) =
                                vlq_decode_signed(feature_data, blocks_pos).unwrap_or((1, 0));

                            PeerFeature::Mode {
                                state_type,
                                verifying,
                                nipopow_suffix,
                                blocks_to_keep: blocks_to_keep as i32,
                            }
                        }
                    }
                    3 => {
                        // Session feature: magic (4 bytes) + session_id (VLQ)
                        if feature_data.len() >= 5 {
                            let mut magic = [0u8; 4];
                            magic.copy_from_slice(&feature_data[0..4]);
                            let (session_id, _) = vlq_decode(feature_data, 4).unwrap_or((0, 4));
                            PeerFeature::Session { magic, session_id }
                        } else {
                            PeerFeature::Unknown {
                                id: feature_id,
                                data: feature_data.to_vec(),
                            }
                        }
                    }
                    2 => {
                        // Local address feature
                        if feature_data.len() == 8 {
                            let ip = IpAddr::V4(std::net::Ipv4Addr::new(
                                feature_data[0],
                                feature_data[1],
                                feature_data[2],
                                feature_data[3],
                            ));
                            let port = u32::from_be_bytes([
                                feature_data[4],
                                feature_data[5],
                                feature_data[6],
                                feature_data[7],
                            ]) as u16;
                            PeerFeature::LocalAddress { ip, port }
                        } else {
                            PeerFeature::Unknown {
                                id: feature_id,
                                data: feature_data.to_vec(),
                            }
                        }
                    }
                    4 => {
                        // REST API URL feature
                        if !feature_data.is_empty() {
                            let url_len = feature_data[0] as usize;
                            if feature_data.len() >= 1 + url_len {
                                let url = String::from_utf8_lossy(&feature_data[1..1 + url_len])
                                    .to_string();
                                PeerFeature::RestApiUrl(url)
                            } else {
                                PeerFeature::Unknown {
                                    id: feature_id,
                                    data: feature_data.to_vec(),
                                }
                            }
                        } else {
                            PeerFeature::Unknown {
                                id: feature_id,
                                data: feature_data.to_vec(),
                            }
                        }
                    }
                    _ => PeerFeature::Unknown {
                        id: feature_id,
                        data: feature_data.to_vec(),
                    },
                };

                features.push(feature);
            }
        }

        Ok((
            Self {
                agent_name,
                version,
                peer_name,
                declared_addr,
                features,
            },
            pos,
        ))
    }
}

impl fmt::Display for PeerSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref addr) = self.declared_addr {
            write!(
                f,
                "{}@{}:{} (v{}.{}.{})",
                self.peer_name, addr.ip, addr.port, self.version.0, self.version.1, self.version.2
            )
        } else {
            write!(
                f,
                "{} (v{}.{}.{})",
                self.peer_name, self.version.0, self.version.1, self.version.2
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vlq_encode_decode() {
        let test_values = [0u64, 1, 127, 128, 255, 16383, 16384, 1767778037901];

        for &value in &test_values {
            let mut buf = Vec::new();
            vlq_encode(&mut buf, value);
            let (decoded, _) = vlq_decode(&buf, 0).unwrap();
            assert_eq!(value, decoded, "VLQ roundtrip failed for {}", value);
        }
    }

    #[test]
    fn test_vlq_signed_encode_decode() {
        let test_values = [0i64, 1, -1, 127, -128, 1000, -1000, i32::MIN as i64];

        for &value in &test_values {
            let mut buf = Vec::new();
            vlq_encode_signed(&mut buf, value);
            let (decoded, _) = vlq_decode_signed(&buf, 0).unwrap();
            assert_eq!(value, decoded, "VLQ signed roundtrip failed for {}", value);
        }
    }

    #[test]
    fn test_handshake_serialize_parse() {
        let handshake = HandshakeData::new(
            "ergo-rust-node".to_string(),
            (5, 0, 0),
            "test-node".to_string(),
        )
        .with_mainnet_features(12345);

        let serialized = handshake.serialize();
        let parsed = HandshakeData::parse(&serialized).unwrap();

        assert_eq!(parsed.agent_name, "ergo-rust-node");
        assert_eq!(parsed.version, (5, 0, 0));
        assert_eq!(parsed.peer_name, "test-node");
        assert_eq!(parsed.features.len(), 2);
    }

    #[test]
    fn test_parse_real_handshake() {
        // Real handshake response from ergo mainnet peer
        let data = hex::decode(
            "c3e7e5beb933076572676f726566060001196d61696e6e65742d736565642d6e6f64652d746f726f6e746f01089f59740fc64602100400010001030d01000204b191cba8ee929cc107",
        )
        .unwrap();

        let handshake = HandshakeData::parse(&data).unwrap();

        assert_eq!(handshake.agent_name, "ergoref");
        assert_eq!(handshake.version, (6, 0, 1));
        assert_eq!(handshake.peer_name, "mainnet-seed-node-toronto");

        // Check declared address
        let addr = handshake.declared_addr.as_ref().unwrap();
        assert_eq!(addr.ip.to_string(), "159.89.116.15");
        assert_eq!(addr.port, 9030);

        // Check features
        assert_eq!(handshake.features.len(), 2);

        // Check Mode feature
        if let PeerFeature::Mode {
            state_type,
            verifying,
            ..
        } = &handshake.features[0]
        {
            assert_eq!(*state_type, 0); // UTXO
            assert!(*verifying);
        } else {
            panic!("Expected Mode feature");
        }

        // Check Session feature
        if let PeerFeature::Session { magic, .. } = &handshake.features[1] {
            assert_eq!(*magic, [1, 0, 2, 4]);
        } else {
            panic!("Expected Session feature");
        }
    }

    #[test]
    fn test_peer_spec_roundtrip() {
        let peer_spec = PeerSpec::new(
            "ergo-rust-node".to_string(),
            (6, 0, 1),
            "test-node".to_string(),
            Some(DeclaredAddress {
                ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
                port: 9030,
            }),
        );

        let serialized = peer_spec.serialize();
        let (parsed, _) = PeerSpec::parse(&serialized, 0).unwrap();

        assert_eq!(parsed.agent_name, "ergo-rust-node");
        assert_eq!(parsed.version, (6, 0, 1));
        assert_eq!(parsed.peer_name, "test-node");
        assert!(parsed.declared_addr.is_some());
        let addr = parsed.declared_addr.unwrap();
        assert_eq!(addr.ip.to_string(), "192.168.1.1");
        assert_eq!(addr.port, 9030);
    }

    #[test]
    fn test_peer_spec_from_handshake() {
        let handshake = HandshakeData::new(
            "ergo-rust-node".to_string(),
            (6, 0, 1),
            "test-node".to_string(),
        );

        let peer_spec = PeerSpec::from_handshake(&handshake);

        assert_eq!(peer_spec.agent_name, handshake.agent_name);
        assert_eq!(peer_spec.version, handshake.version);
        assert_eq!(peer_spec.peer_name, handshake.peer_name);
    }

    #[test]
    fn test_peer_spec_display() {
        let peer_spec = PeerSpec::new(
            "ergo-rust-node".to_string(),
            (6, 0, 1),
            "test-node".to_string(),
            Some(DeclaredAddress {
                ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
                port: 9030,
            }),
        );

        let display = format!("{}", peer_spec);
        assert!(display.contains("test-node"));
        assert!(display.contains("192.168.1.1"));
        assert!(display.contains("9030"));
        assert!(display.contains("6.0.1"));
    }
}
