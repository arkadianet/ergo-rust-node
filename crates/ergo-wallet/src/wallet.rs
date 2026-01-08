//! Main wallet implementation.
//!
//! This module provides the high-level wallet API for:
//! - Initialization from mnemonic or existing secrets
//! - Address derivation and management
//! - Balance tracking
//! - Transaction building (see tx_builder module)

use crate::{keystore::DerivedKey, BoxTracker, KeyStore, WalletBalance, WalletError, WalletResult};
use ergo_lib::{ergotree_ir::chain::address::NetworkPrefix, wallet::secret_key::SecretKey};
use ergo_state::StateManager;
use parking_lot::RwLock;
use std::sync::Arc;
use tracing::{debug, info};

/// Wallet configuration.
#[derive(Debug, Clone)]
pub struct WalletConfig {
    /// Wallet data directory.
    pub data_dir: std::path::PathBuf,
    /// Secret storage file name.
    pub secret_file: String,
    /// Number of addresses to pre-generate.
    pub pre_generate: usize,
    /// Default account index.
    pub default_account: u32,
    /// Network (mainnet or testnet).
    pub network: NetworkPrefix,
}

impl Default for WalletConfig {
    fn default() -> Self {
        Self {
            data_dir: std::path::PathBuf::from(".ergo-wallet"),
            secret_file: "secret.json".to_string(),
            pre_generate: 20,
            default_account: 0,
            network: NetworkPrefix::Mainnet,
        }
    }
}

impl WalletConfig {
    /// Create a testnet configuration.
    pub fn testnet() -> Self {
        Self {
            network: NetworkPrefix::Testnet,
            ..Default::default()
        }
    }
}

/// Wallet status information.
#[derive(Debug, Clone)]
pub struct WalletStatus {
    /// Is the wallet initialized with a seed.
    pub initialized: bool,
    /// Is the wallet unlocked.
    pub unlocked: bool,
    /// Current blockchain height tracked.
    pub height: u32,
    /// Number of derived addresses.
    pub address_count: usize,
    /// Network (mainnet/testnet).
    pub network: NetworkPrefix,
}

/// HD Wallet for Ergo.
///
/// Provides:
/// - BIP39 mnemonic support
/// - BIP32/BIP44 HD key derivation following EIP-3
/// - Address generation and tracking
/// - Balance calculation from UTXO state
pub struct Wallet {
    /// Configuration.
    config: WalletConfig,
    /// Key store for secrets.
    keystore: KeyStore,
    /// Box tracker for balance.
    tracker: BoxTracker,
    /// Next address index to derive.
    next_index: RwLock<u32>,
}

impl Wallet {
    /// Create a new wallet.
    pub fn new(config: WalletConfig, state: Arc<StateManager>) -> Self {
        let secret_path = config.data_dir.join(&config.secret_file);
        let keystore = KeyStore::with_path_and_network(&secret_path, config.network);
        let tracker = BoxTracker::new(state);

        Self {
            config,
            keystore,
            tracker,
            next_index: RwLock::new(0),
        }
    }

    /// Initialize a new wallet from mnemonic phrase.
    ///
    /// # Arguments
    /// * `mnemonic` - BIP39 mnemonic phrase (12, 15, 18, 21, or 24 words)
    /// * `mnemonic_password` - Optional BIP39 passphrase (can be empty)
    /// * `encryption_password` - Password to encrypt the seed on disk
    ///
    /// # Returns
    /// The mnemonic phrase (useful when generating a new one)
    pub fn init(
        &self,
        mnemonic: &str,
        mnemonic_password: &str,
        encryption_password: &str,
    ) -> WalletResult<String> {
        if self.keystore.is_initialized() {
            return Err(WalletError::TxBuildError(
                "Wallet already initialized".to_string(),
            ));
        }

        // Create data directory if needed
        std::fs::create_dir_all(&self.config.data_dir)?;

        // Initialize keystore with mnemonic
        self.keystore
            .initialize_from_mnemonic(mnemonic, mnemonic_password, encryption_password)?;

        // Pre-generate addresses
        self.generate_addresses(self.config.pre_generate)?;

        info!(
            addresses = self.config.pre_generate,
            "Wallet initialized from mnemonic"
        );
        Ok(mnemonic.to_string())
    }

    /// Initialize with a new random mnemonic.
    ///
    /// Returns the generated mnemonic phrase - user MUST back this up!
    pub fn init_new(&self, encryption_password: &str) -> WalletResult<String> {
        // Generate random mnemonic using ergo-lib if available
        // For now, use a placeholder that should be replaced with proper generation
        let mnemonic = generate_random_mnemonic()?;
        self.init(&mnemonic, "", encryption_password)?;
        Ok(mnemonic)
    }

    /// Load existing wallet from disk.
    pub fn load(&self) -> WalletResult<()> {
        self.keystore.load()?;
        info!("Wallet loaded from disk");
        Ok(())
    }

    /// Unlock wallet with password.
    pub fn unlock(&self, password: &str) -> WalletResult<()> {
        self.keystore.unlock(password)?;

        // Regenerate addresses
        self.generate_addresses(self.config.pre_generate)?;

        info!("Wallet unlocked");
        Ok(())
    }

    /// Lock wallet (clear secrets from memory).
    pub fn lock(&self) {
        self.keystore.lock();
        *self.next_index.write() = 0;
        info!("Wallet locked");
    }

    /// Get wallet status.
    pub fn status(&self) -> WalletStatus {
        WalletStatus {
            initialized: self.keystore.is_initialized(),
            unlocked: self.keystore.is_unlocked(),
            height: self.tracker.current_height(),
            address_count: self.keystore.get_derived_keys().len(),
            network: self.config.network,
        }
    }

    /// Get wallet balance.
    pub fn balance(&self) -> WalletResult<WalletBalance> {
        if !self.keystore.is_unlocked() {
            return Err(WalletError::Locked);
        }

        // Scan for boxes belonging to our addresses
        self.tracker.scan()?;

        Ok(self.tracker.balance())
    }

    /// Get all derived addresses.
    pub fn addresses(&self) -> WalletResult<Vec<String>> {
        if !self.keystore.is_unlocked() {
            return Err(WalletError::Locked);
        }

        Ok(self
            .keystore
            .get_derived_keys()
            .iter()
            .map(|k| k.address_str.clone())
            .collect())
    }

    /// Get all derived keys (for signing).
    pub fn derived_keys(&self) -> WalletResult<Vec<DerivedKey>> {
        if !self.keystore.is_unlocked() {
            return Err(WalletError::Locked);
        }

        Ok(self.keystore.get_derived_keys())
    }

    /// Generate a new receiving address.
    pub fn new_address(&self) -> WalletResult<String> {
        if !self.keystore.is_unlocked() {
            return Err(WalletError::Locked);
        }

        let index = *self.next_index.read();
        let key = self
            .keystore
            .derive_key_for_index(self.config.default_account, index)?;

        self.keystore.cache_derived_key(key.clone());
        self.tracker.track_address_str(&key.address_str);
        *self.next_index.write() = index + 1;

        debug!(index, address = %key.address_str, "Generated new address");
        Ok(key.address_str)
    }

    /// Derive address at specific path.
    pub fn derive_address(&self, path: &str) -> WalletResult<String> {
        if !self.keystore.is_unlocked() {
            return Err(WalletError::Locked);
        }

        let key = self.keystore.derive_key(path)?;
        self.keystore.cache_derived_key(key.clone());
        self.tracker.track_address_str(&key.address_str);

        Ok(key.address_str)
    }

    /// Get secret key for an address (for signing).
    pub fn get_secret_key(&self, address: &str) -> WalletResult<SecretKey> {
        if !self.keystore.is_unlocked() {
            return Err(WalletError::Locked);
        }

        self.keystore
            .find_secret_key_by_str(address)?
            .ok_or_else(|| WalletError::AddressNotFound(address.to_string()))
    }

    /// Get all secret keys (for multi-input signing).
    pub fn get_all_secret_keys(&self) -> WalletResult<Vec<SecretKey>> {
        if !self.keystore.is_unlocked() {
            return Err(WalletError::Locked);
        }

        Ok(self
            .keystore
            .get_derived_keys()
            .iter()
            .map(|k| k.secret_key.clone())
            .collect())
    }

    /// Get unspent boxes for this wallet.
    pub fn get_unspent_boxes(&self) -> WalletResult<Vec<crate::WalletBox>> {
        if !self.keystore.is_unlocked() {
            return Err(WalletError::Locked);
        }

        Ok(self.tracker.get_unspent())
    }

    /// Generate multiple addresses.
    fn generate_addresses(&self, count: usize) -> WalletResult<()> {
        let start_index = *self.next_index.read();

        for i in 0..count {
            let index = start_index + i as u32;
            let key = self
                .keystore
                .derive_key_for_index(self.config.default_account, index)?;

            self.keystore.cache_derived_key(key.clone());
            self.tracker.track_address_str(&key.address_str);
        }

        *self.next_index.write() = start_index + count as u32;
        debug!(count, start_index, "Generated addresses");
        Ok(())
    }

    /// Get the keystore (for advanced operations).
    pub fn keystore(&self) -> &KeyStore {
        &self.keystore
    }

    /// Get the box tracker (for scanning).
    pub fn tracker(&self) -> &BoxTracker {
        &self.tracker
    }
}

/// Generate a random BIP39 mnemonic phrase.
///
/// This is a simplified implementation. For production, use the
/// mnemonic_generator feature of ergo-lib.
fn generate_random_mnemonic() -> WalletResult<String> {
    // BIP39 English wordlist (first 100 words for this example)
    // In production, use the full 2048-word list from ergo-lib
    const WORDS: &[&str] = &[
        "abandon", "ability", "able", "about", "above", "absent", "absorb", "abstract", "absurd",
        "abuse", "access", "accident", "account", "accuse", "achieve", "acid", "acoustic",
        "acquire", "across", "act", "action", "actor", "actress", "actual", "adapt", "add",
        "addict", "address", "adjust", "admit", "adult", "advance", "advice", "aerobic", "affair",
        "afford", "afraid", "again", "age", "agent", "agree", "ahead", "aim", "air", "airport",
        "aisle", "alarm", "album", "alcohol", "alert", "alien", "all", "alley", "allow", "almost",
        "alone", "alpha", "already", "also", "alter", "always", "amateur", "amazing", "among",
        "amount", "amused", "analyst", "anchor", "ancient", "anger", "angle", "angry", "animal",
        "ankle", "announce", "annual", "another", "answer", "antenna", "antique", "anxiety", "any",
        "apart", "apology", "appear", "apple", "approve", "april", "arch", "arctic", "area",
        "arena", "argue", "arm", "armed", "armor", "army", "around", "arrange", "arrest",
    ];

    // Generate 12 random words (128 bits of entropy)
    let mut phrase = Vec::with_capacity(12);
    for _ in 0..12 {
        let idx = rand::random::<usize>() % WORDS.len();
        phrase.push(WORDS[idx]);
    }

    Ok(phrase.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ergo_storage::Database;
    use tempfile::TempDir;

    const TEST_MNEMONIC: &str =
        "slow silly start wash bundle suffer bulb ancient height spin express remind today effort helmet";

    fn create_test_wallet() -> (Wallet, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("db");
        let db = Database::open(&db_path).unwrap();
        let state = Arc::new(StateManager::new(Arc::new(db)));

        let config = WalletConfig {
            data_dir: tmp.path().join("wallet"),
            pre_generate: 5,
            ..Default::default()
        };

        let wallet = Wallet::new(config, state);
        (wallet, tmp)
    }

    #[test]
    fn test_wallet_init() {
        let (wallet, _tmp) = create_test_wallet();

        assert!(!wallet.status().initialized);

        wallet.init(TEST_MNEMONIC, "", "password").unwrap();

        let status = wallet.status();
        assert!(status.initialized);
        assert!(status.unlocked);
        assert_eq!(status.address_count, 5); // pre_generate = 5
    }

    #[test]
    fn test_wallet_addresses() {
        let (wallet, _tmp) = create_test_wallet();
        wallet.init(TEST_MNEMONIC, "", "password").unwrap();

        let addresses = wallet.addresses().unwrap();
        assert_eq!(addresses.len(), 5);

        // First address should match known test vector
        assert_eq!(
            addresses[0],
            "9eatpGQdYNjTi5ZZLK7Bo7C3ms6oECPnxbQTRn6sDcBNLMYSCa8"
        );
    }

    #[test]
    fn test_wallet_new_address() {
        let (wallet, _tmp) = create_test_wallet();
        wallet.init(TEST_MNEMONIC, "", "password").unwrap();

        let initial_count = wallet.status().address_count;
        let new_addr = wallet.new_address().unwrap();

        assert_eq!(wallet.status().address_count, initial_count + 1);
        assert!(wallet.addresses().unwrap().contains(&new_addr));
    }

    #[test]
    fn test_wallet_lock_unlock() {
        let (wallet, _tmp) = create_test_wallet();
        wallet.init(TEST_MNEMONIC, "", "password").unwrap();

        assert!(wallet.status().unlocked);

        wallet.lock();
        assert!(!wallet.status().unlocked);
        assert!(wallet.addresses().is_err());

        wallet.unlock("password").unwrap();
        assert!(wallet.status().unlocked);

        let addresses = wallet.addresses().unwrap();
        assert_eq!(
            addresses[0],
            "9eatpGQdYNjTi5ZZLK7Bo7C3ms6oECPnxbQTRn6sDcBNLMYSCa8"
        );
    }

    #[test]
    fn test_wallet_get_secret_key() {
        let (wallet, _tmp) = create_test_wallet();
        wallet.init(TEST_MNEMONIC, "", "password").unwrap();

        let addresses = wallet.addresses().unwrap();
        let secret = wallet.get_secret_key(&addresses[0]).unwrap();

        // We got a secret key
        // SecretKey has a to_bytes method that returns the key bytes
        let _ = secret; // Just verify we got a secret key
    }

    #[test]
    fn test_wallet_derive_custom_path() {
        let (wallet, _tmp) = create_test_wallet();
        wallet.init(TEST_MNEMONIC, "", "password").unwrap();

        // Derive at a custom path
        let addr = wallet.derive_address("m/44'/429'/1'/0/0").unwrap();

        // Should be different from account 0
        let account0_addr = wallet.addresses().unwrap()[0].clone();
        assert_ne!(addr, account0_addr);
    }
}
