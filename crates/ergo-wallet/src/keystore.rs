//! Key storage and HD derivation.
//!
//! This module provides secure key storage with encryption and
//! BIP32/BIP39 HD key derivation using ergo-lib primitives.
//!
//! ## Security
//!
//! - Seeds are encrypted using AES-256-GCM
//! - Encryption keys are derived using Argon2id
//! - Random salt and nonce for each encryption
//! - Keys are cleared from memory when locked

use crate::{WalletError, WalletResult};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{password_hash::SaltString, Argon2};
use ergo_lib::{
    ergotree_ir::chain::address::{Address, NetworkAddress, NetworkPrefix},
    wallet::{
        derivation_path::DerivationPath,
        ext_secret_key::ExtSecretKey,
        mnemonic::{Mnemonic, MnemonicSeed},
        secret_key::SecretKey,
    },
};
use parking_lot::RwLock;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::debug;

/// Version of the encryption format.
const ENCRYPTION_VERSION: u8 = 1;

/// Encrypted key data stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedKey {
    /// Version of the encryption format.
    #[serde(default = "default_version")]
    pub version: u8,
    /// Encrypted seed (AES-256-GCM).
    pub encrypted_seed: Vec<u8>,
    /// Salt for Argon2id key derivation (22 bytes base64).
    pub salt: String,
    /// Nonce for AES-GCM (12 bytes).
    pub nonce: Vec<u8>,
    /// Argon2id memory cost in KiB.
    #[serde(default = "default_memory_cost")]
    pub memory_cost: u32,
    /// Argon2id time cost (iterations).
    #[serde(default = "default_time_cost")]
    pub time_cost: u32,
    /// Argon2id parallelism.
    #[serde(default = "default_parallelism")]
    pub parallelism: u32,
}

fn default_version() -> u8 {
    1
}
fn default_memory_cost() -> u32 {
    65536
}
fn default_time_cost() -> u32 {
    3
}
fn default_parallelism() -> u32 {
    4
}

/// A derived key with its address.
#[derive(Debug, Clone)]
pub struct DerivedKey {
    /// The secret key.
    pub secret_key: SecretKey,
    /// The derivation path.
    pub path: DerivationPath,
    /// The P2PK address.
    pub address: Address,
    /// The network-prefixed address string.
    pub address_str: String,
}

/// Key store for HD wallet keys.
///
/// Provides:
/// - Secure encrypted storage of mnemonic seed using AES-256-GCM
/// - Argon2id key derivation from password
/// - BIP32/BIP39 HD key derivation
/// - Address generation following EIP-3 (Ergo's BIP44 path)
pub struct KeyStore {
    /// Path to key file.
    path: Option<std::path::PathBuf>,
    /// Encrypted key data.
    encrypted: RwLock<Option<EncryptedKey>>,
    /// Decrypted mnemonic seed (when unlocked).
    seed: RwLock<Option<MnemonicSeed>>,
    /// Master extended secret key (when unlocked).
    master_key: RwLock<Option<ExtSecretKey>>,
    /// Network prefix for address encoding.
    network: NetworkPrefix,
    /// Derived keys cache.
    derived_keys: RwLock<Vec<DerivedKey>>,
}

impl KeyStore {
    /// Create a new in-memory key store for mainnet.
    pub fn new() -> Self {
        Self::with_network(NetworkPrefix::Mainnet)
    }

    /// Create a new in-memory key store for the specified network.
    pub fn with_network(network: NetworkPrefix) -> Self {
        Self {
            path: None,
            encrypted: RwLock::new(None),
            seed: RwLock::new(None),
            master_key: RwLock::new(None),
            network,
            derived_keys: RwLock::new(Vec::new()),
        }
    }

    /// Create a key store backed by a file.
    pub fn with_path<P: AsRef<Path>>(path: P) -> Self {
        Self {
            path: Some(path.as_ref().to_path_buf()),
            encrypted: RwLock::new(None),
            seed: RwLock::new(None),
            master_key: RwLock::new(None),
            network: NetworkPrefix::Mainnet,
            derived_keys: RwLock::new(Vec::new()),
        }
    }

    /// Create a key store with path and network.
    pub fn with_path_and_network<P: AsRef<Path>>(path: P, network: NetworkPrefix) -> Self {
        Self {
            path: Some(path.as_ref().to_path_buf()),
            encrypted: RwLock::new(None),
            seed: RwLock::new(None),
            master_key: RwLock::new(None),
            network,
            derived_keys: RwLock::new(Vec::new()),
        }
    }

    /// Check if key store is initialized.
    pub fn is_initialized(&self) -> bool {
        self.encrypted.read().is_some()
    }

    /// Check if key store is unlocked.
    pub fn is_unlocked(&self) -> bool {
        self.seed.read().is_some()
    }

    /// Get the network prefix.
    pub fn network(&self) -> NetworkPrefix {
        self.network
    }

    /// Initialize with a mnemonic phrase.
    ///
    /// This converts the mnemonic to a seed using BIP39 and stores it encrypted.
    pub fn initialize_from_mnemonic(
        &self,
        mnemonic: &str,
        mnemonic_password: &str,
        encryption_password: &str,
    ) -> WalletResult<()> {
        // Validate mnemonic (basic check - should have 12, 15, 18, 21, or 24 words)
        let word_count = mnemonic.split_whitespace().count();
        if ![12, 15, 18, 21, 24].contains(&word_count) {
            return Err(WalletError::InvalidMnemonic(format!(
                "Invalid word count: {}. Expected 12, 15, 18, 21, or 24 words.",
                word_count
            )));
        }

        // Convert mnemonic to seed using BIP39
        let seed = Mnemonic::to_seed(mnemonic, mnemonic_password);

        // Derive master key to validate the seed works
        let master_key = ExtSecretKey::derive_master(seed).map_err(|e| {
            WalletError::KeyDerivation(format!("Failed to derive master key: {}", e))
        })?;

        // Encrypt and store the seed
        self.encrypt_and_store_seed(&seed, encryption_password)?;

        // Store decrypted seed and master key
        *self.seed.write() = Some(seed);
        *self.master_key.write() = Some(master_key);

        debug!("Key store initialized from mnemonic");
        Ok(())
    }

    /// Initialize with a raw seed (64 bytes).
    pub fn initialize_from_seed(
        &self,
        seed: MnemonicSeed,
        encryption_password: &str,
    ) -> WalletResult<()> {
        // Derive master key to validate
        let master_key = ExtSecretKey::derive_master(seed).map_err(|e| {
            WalletError::KeyDerivation(format!("Failed to derive master key: {}", e))
        })?;

        // Encrypt and store
        self.encrypt_and_store_seed(&seed, encryption_password)?;

        *self.seed.write() = Some(seed);
        *self.master_key.write() = Some(master_key);

        debug!("Key store initialized from seed");
        Ok(())
    }

    /// Encrypt seed using AES-256-GCM with Argon2id key derivation.
    fn encrypt_and_store_seed(&self, seed: &MnemonicSeed, password: &str) -> WalletResult<()> {
        // Generate random salt for Argon2id
        let salt = SaltString::generate(&mut OsRng);

        // Generate random nonce for AES-GCM (12 bytes)
        let nonce_bytes: [u8; 12] = rand::random();
        let nonce = Nonce::from_slice(&nonce_bytes);

        // Argon2id parameters (OWASP recommended)
        let memory_cost = 65536; // 64 MiB
        let time_cost = 3; // 3 iterations
        let parallelism = 4; // 4 lanes

        // Derive encryption key using Argon2id
        let encryption_key =
            derive_key_argon2id(password, salt.as_str(), memory_cost, time_cost, parallelism)?;

        // Create AES-256-GCM cipher
        let cipher = Aes256Gcm::new_from_slice(&encryption_key)
            .map_err(|e| WalletError::KeyDerivation(format!("Failed to create cipher: {}", e)))?;

        // Encrypt the seed
        let encrypted_seed = cipher
            .encrypt(nonce, seed.as_ref())
            .map_err(|e| WalletError::KeyDerivation(format!("Encryption failed: {}", e)))?;

        let encrypted = EncryptedKey {
            version: ENCRYPTION_VERSION,
            encrypted_seed,
            salt: salt.as_str().to_string(),
            nonce: nonce_bytes.to_vec(),
            memory_cost,
            time_cost,
            parallelism,
        };

        *self.encrypted.write() = Some(encrypted.clone());

        // Save to file if path is set
        if let Some(ref path) = self.path {
            let json = serde_json::to_string_pretty(&encrypted).map_err(|e| {
                WalletError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                ))
            })?;
            std::fs::write(path, json)?;
        }

        Ok(())
    }

    /// Load encrypted key from file.
    pub fn load(&self) -> WalletResult<()> {
        let path = self.path.as_ref().ok_or_else(|| {
            WalletError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "No path set",
            ))
        })?;

        if !path.exists() {
            return Err(WalletError::NotInitialized);
        }

        let json = std::fs::read_to_string(path)?;
        let encrypted: EncryptedKey = serde_json::from_str(&json).map_err(|e| {
            WalletError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                e.to_string(),
            ))
        })?;

        *self.encrypted.write() = Some(encrypted);
        debug!("Key store loaded from file");
        Ok(())
    }

    /// Unlock with password.
    pub fn unlock(&self, password: &str) -> WalletResult<()> {
        let encrypted = self
            .encrypted
            .read()
            .clone()
            .ok_or(WalletError::NotInitialized)?;

        // Derive decryption key using Argon2id
        let decryption_key = derive_key_argon2id(
            password,
            &encrypted.salt,
            encrypted.memory_cost,
            encrypted.time_cost,
            encrypted.parallelism,
        )?;

        // Create AES-256-GCM cipher
        let cipher =
            Aes256Gcm::new_from_slice(&decryption_key).map_err(|_| WalletError::InvalidPassword)?;

        // Decrypt seed
        let nonce = Nonce::from_slice(&encrypted.nonce);
        let decrypted = cipher
            .decrypt(nonce, encrypted.encrypted_seed.as_ref())
            .map_err(|_| WalletError::InvalidPassword)?;

        // Convert to MnemonicSeed
        if decrypted.len() != 64 {
            return Err(WalletError::InvalidPassword);
        }
        let mut seed: MnemonicSeed = [0u8; 64];
        seed.copy_from_slice(&decrypted);

        // Try to derive master key to validate
        let master_key =
            ExtSecretKey::derive_master(seed).map_err(|_| WalletError::InvalidPassword)?;

        *self.seed.write() = Some(seed);
        *self.master_key.write() = Some(master_key);

        debug!("Key store unlocked");
        Ok(())
    }

    /// Lock the key store (clear decrypted keys from memory).
    pub fn lock(&self) {
        *self.seed.write() = None;
        *self.master_key.write() = None;
        self.derived_keys.write().clear();
        debug!("Key store locked");
    }

    /// Get the raw seed (must be unlocked).
    pub fn seed(&self) -> WalletResult<MnemonicSeed> {
        self.seed.read().ok_or(WalletError::Locked)
    }

    /// Get the master extended secret key (must be unlocked).
    pub fn master_key(&self) -> WalletResult<ExtSecretKey> {
        self.master_key.read().clone().ok_or(WalletError::Locked)
    }

    /// Derive a key at the given path.
    ///
    /// Path should be in format like "m/44'/429'/0'/0/0" following EIP-3.
    pub fn derive_key(&self, path: &str) -> WalletResult<DerivedKey> {
        let master = self.master_key()?;

        let derivation_path: DerivationPath = path.parse().map_err(|e| {
            WalletError::KeyDerivation(format!("Invalid derivation path '{}': {:?}", path, e))
        })?;

        let derived = master.derive(derivation_path.clone()).map_err(|e| {
            WalletError::KeyDerivation(format!("Failed to derive key at {}: {}", path, e))
        })?;

        let secret_key = derived.secret_key();
        let address: Address = derived
            .public_key()
            .map_err(|e| WalletError::KeyDerivation(format!("Failed to get public key: {}", e)))?
            .into();

        let network_address = NetworkAddress::new(self.network, &address);
        let address_str = network_address.to_base58();

        Ok(DerivedKey {
            secret_key,
            path: derivation_path,
            address,
            address_str,
        })
    }

    /// Derive a key for the given account and address index.
    ///
    /// Uses the standard Ergo derivation path: m/44'/429'/{account}'/0/{index}
    pub fn derive_key_for_index(&self, account: u32, index: u32) -> WalletResult<DerivedKey> {
        let path = format!("m/44'/429'/{}'/0/{}", account, index);
        self.derive_key(&path)
    }

    /// Derive multiple keys starting from the given index.
    pub fn derive_keys(
        &self,
        account: u32,
        start_index: u32,
        count: u32,
    ) -> WalletResult<Vec<DerivedKey>> {
        let mut keys = Vec::with_capacity(count as usize);
        for i in 0..count {
            keys.push(self.derive_key_for_index(account, start_index + i)?);
        }
        Ok(keys)
    }

    /// Get all derived keys (cached).
    pub fn get_derived_keys(&self) -> Vec<DerivedKey> {
        self.derived_keys.read().clone()
    }

    /// Add a derived key to the cache.
    pub fn cache_derived_key(&self, key: DerivedKey) {
        self.derived_keys.write().push(key);
    }

    /// Find a secret key for the given address.
    pub fn find_secret_key(&self, address: &Address) -> WalletResult<Option<SecretKey>> {
        let keys = self.derived_keys.read();
        for key in keys.iter() {
            if &key.address == address {
                return Ok(Some(key.secret_key.clone()));
            }
        }
        Ok(None)
    }

    /// Find a secret key by address string.
    pub fn find_secret_key_by_str(&self, address_str: &str) -> WalletResult<Option<SecretKey>> {
        let keys = self.derived_keys.read();
        for key in keys.iter() {
            if key.address_str == address_str {
                return Ok(Some(key.secret_key.clone()));
            }
        }
        Ok(None)
    }
}

impl Default for KeyStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Derive a 32-byte encryption key using Argon2id.
fn derive_key_argon2id(
    password: &str,
    salt: &str,
    memory_cost: u32,
    time_cost: u32,
    parallelism: u32,
) -> WalletResult<[u8; 32]> {
    use argon2::{Algorithm, Params, Version};

    let params = Params::new(memory_cost, time_cost, parallelism, Some(32))
        .map_err(|e| WalletError::KeyDerivation(format!("Invalid Argon2 params: {}", e)))?;

    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let salt_bytes = salt.as_bytes();
    let mut output = [0u8; 32];

    argon2
        .hash_password_into(password.as_bytes(), salt_bytes, &mut output)
        .map_err(|e| WalletError::KeyDerivation(format!("Argon2id failed: {}", e)))?;

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MNEMONIC: &str =
        "slow silly start wash bundle suffer bulb ancient height spin express remind today effort helmet";

    #[test]
    fn test_keystore_init_from_mnemonic() {
        let ks = KeyStore::new();

        assert!(!ks.is_initialized());
        assert!(!ks.is_unlocked());

        ks.initialize_from_mnemonic(TEST_MNEMONIC, "", "password123")
            .unwrap();

        assert!(ks.is_initialized());
        assert!(ks.is_unlocked());
    }

    #[test]
    fn test_keystore_lock_unlock() {
        let ks = KeyStore::new();
        ks.initialize_from_mnemonic(TEST_MNEMONIC, "", "password123")
            .unwrap();

        assert!(ks.is_unlocked());

        ks.lock();
        assert!(!ks.is_unlocked());
        assert!(ks.seed().is_err());
        assert!(ks.master_key().is_err());

        ks.unlock("password123").unwrap();
        assert!(ks.is_unlocked());
        assert!(ks.seed().is_ok());
    }

    #[test]
    fn test_wrong_password() {
        let ks = KeyStore::new();
        ks.initialize_from_mnemonic(TEST_MNEMONIC, "", "correct_password")
            .unwrap();

        ks.lock();

        let result = ks.unlock("wrong_password");
        assert!(result.is_err());
        assert!(matches!(result, Err(WalletError::InvalidPassword)));
    }

    #[test]
    fn test_derive_key() {
        let ks = KeyStore::new();
        ks.initialize_from_mnemonic(TEST_MNEMONIC, "", "password123")
            .unwrap();

        // This should match ergo-lib's test vector
        let key = ks.derive_key("m/44'/429'/0'/0/0").unwrap();

        // Expected address from ergo-lib test
        assert_eq!(
            key.address_str,
            "9eatpGQdYNjTi5ZZLK7Bo7C3ms6oECPnxbQTRn6sDcBNLMYSCa8"
        );
    }

    #[test]
    fn test_derive_key_for_index() {
        let ks = KeyStore::new();
        ks.initialize_from_mnemonic(TEST_MNEMONIC, "", "password123")
            .unwrap();

        let key0 = ks.derive_key_for_index(0, 0).unwrap();
        let key1 = ks.derive_key_for_index(0, 1).unwrap();

        // Expected addresses from ergo-lib appkit test vector
        assert_eq!(
            key0.address_str,
            "9eatpGQdYNjTi5ZZLK7Bo7C3ms6oECPnxbQTRn6sDcBNLMYSCa8"
        );
        assert_eq!(
            key1.address_str,
            "9iBhwkjzUAVBkdxWvKmk7ab7nFgZRFbGpXA9gP6TAoakFnLNomk"
        );
    }

    #[test]
    fn test_derive_multiple_keys() {
        let ks = KeyStore::new();
        ks.initialize_from_mnemonic(TEST_MNEMONIC, "", "password123")
            .unwrap();

        let keys = ks.derive_keys(0, 0, 5).unwrap();
        assert_eq!(keys.len(), 5);

        // All addresses should be unique
        let addresses: std::collections::HashSet<_> =
            keys.iter().map(|k| k.address_str.clone()).collect();
        assert_eq!(addresses.len(), 5);
    }

    #[test]
    fn test_invalid_mnemonic() {
        let ks = KeyStore::new();

        // Too few words
        let result = ks.initialize_from_mnemonic("word1 word2 word3", "", "password");
        assert!(result.is_err());
    }

    #[test]
    fn test_testnet_addresses() {
        let ks = KeyStore::with_network(NetworkPrefix::Testnet);
        ks.initialize_from_mnemonic(TEST_MNEMONIC, "", "password123")
            .unwrap();

        let key = ks.derive_key_for_index(0, 0).unwrap();

        // Testnet addresses start with '3'
        assert!(key.address_str.starts_with('3'));
    }

    #[test]
    fn test_encrypted_key_serialization() {
        let ks = KeyStore::new();
        ks.initialize_from_mnemonic(TEST_MNEMONIC, "", "password123")
            .unwrap();

        let encrypted = ks.encrypted.read().clone().unwrap();

        // Serialize and deserialize
        let json = serde_json::to_string(&encrypted).unwrap();
        let deserialized: EncryptedKey = serde_json::from_str(&json).unwrap();

        assert_eq!(encrypted.version, deserialized.version);
        assert_eq!(encrypted.salt, deserialized.salt);
        assert_eq!(encrypted.nonce, deserialized.nonce);
        assert_eq!(encrypted.encrypted_seed, deserialized.encrypted_seed);
    }
}
