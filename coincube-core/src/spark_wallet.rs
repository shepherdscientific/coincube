use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use bip39::{Language, Mnemonic};
use miniscript::bitcoin::{
    hashes::{sha256, Hash},
    Network,
};
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub enum SparkWalletError {
    Io(io::Error),
    Serde(serde_json::Error),
    WalletNotInitialized,
    WalletAlreadyExists,
    InvalidMnemonic(String),
    InvalidRecipient,
    InvalidAmount,
    InsufficientFunds { available: u64, requested: u64 },
}

impl fmt::Display for SparkWalletError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "Spark wallet I/O error: {}", e),
            Self::Serde(e) => write!(f, "Spark wallet serialization error: {}", e),
            Self::WalletNotInitialized => write!(f, "Spark wallet has not been created yet."),
            Self::WalletAlreadyExists => write!(f, "Spark wallet already exists."),
            Self::InvalidMnemonic(e) => write!(f, "Invalid mnemonic: {}", e),
            Self::InvalidRecipient => write!(
                f,
                "Recipient must be a non-empty Spark address or Lightning invoice."
            ),
            Self::InvalidAmount => write!(f, "Amount must be greater than zero."),
            Self::InsufficientFunds {
                available,
                requested,
            } => write!(
                f,
                "Insufficient Spark balance: available {} sats, requested {} sats.",
                available, requested
            ),
        }
    }
}

impl std::error::Error for SparkWalletError {}

impl From<io::Error> for SparkWalletError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for SparkWalletError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serde(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SparkTransactionDirection {
    Sent,
    Received,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SparkTransactionStatus {
    Pending,
    Confirmed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SparkTransaction {
    pub txid: String,
    pub direction: SparkTransactionDirection,
    pub amount_sat: u64,
    pub counterparty: String,
    pub timestamp: u64,
    pub status: SparkTransactionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SparkWalletInfo {
    pub mnemonic: String,
    pub wallet_id: String,
    pub network: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SparkBalance {
    pub sats: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SparkWalletState {
    wallet_id: String,
    network: String,
    mnemonic: String,
    balance_sat: u64,
    next_address_index: u64,
    next_tx_index: u64,
    transactions: Vec<SparkTransaction>,
}

pub struct SparkWallet {
    state_path: PathBuf,
    state: Option<SparkWalletState>,
}

impl SparkWallet {
    pub fn load(state_path: impl Into<PathBuf>) -> Result<Self, SparkWalletError> {
        let state_path = state_path.into();
        let state = if state_path.exists() {
            Some(serde_json::from_slice(&fs::read(&state_path)?)?)
        } else {
            None
        };
        Ok(Self { state_path, state })
    }

    pub fn create_wallet(
        &mut self,
        network: Network,
        mnemonic: Option<&str>,
    ) -> Result<SparkWalletInfo, SparkWalletError> {
        if self.state.is_some() {
            return Err(SparkWalletError::WalletAlreadyExists);
        }

        let mnemonic = match mnemonic {
            Some(words) => Mnemonic::parse_in(Language::English, words)
                .map_err(|e| SparkWalletError::InvalidMnemonic(e.to_string()))?,
            None => {
                let mut entropy = [0u8; 16];
                getrandom::fill(&mut entropy)
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
                Mnemonic::from_entropy_in(Language::English, &entropy)
                    .map_err(|e| SparkWalletError::InvalidMnemonic(e.to_string()))?
            }
        };

        let mnemonic_string = mnemonic.to_string();
        let wallet_id = wallet_id(&mnemonic_string, network);
        let state = SparkWalletState {
            wallet_id: wallet_id.clone(),
            network: network.to_string(),
            mnemonic: mnemonic_string.clone(),
            balance_sat: 0,
            next_address_index: 0,
            next_tx_index: 0,
            transactions: Vec::new(),
        };

        self.state = Some(state);
        self.persist()?;

        Ok(SparkWalletInfo {
            mnemonic: mnemonic_string,
            wallet_id,
            network: network.to_string(),
        })
    }

    pub fn get_balance(&self) -> Result<SparkBalance, SparkWalletError> {
        let state = self
            .state
            .as_ref()
            .ok_or(SparkWalletError::WalletNotInitialized)?;
        Ok(SparkBalance {
            sats: state.balance_sat,
        })
    }

    pub fn receive_address(&mut self) -> Result<String, SparkWalletError> {
        let (address, should_persist) = {
            let state = self
                .state
                .as_mut()
                .ok_or(SparkWalletError::WalletNotInitialized)?;
            let address = format!(
                "spark:{}:{}:{}",
                state.network, state.wallet_id, state.next_address_index
            );
            state.next_address_index = state.next_address_index.saturating_add(1);
            (address, true)
        };

        if should_persist {
            self.persist()?;
        }

        Ok(address)
    }

    pub fn send(
        &mut self,
        recipient: &str,
        amount_sat: u64,
    ) -> Result<SparkTransaction, SparkWalletError> {
        if recipient.trim().is_empty() {
            return Err(SparkWalletError::InvalidRecipient);
        }
        if amount_sat == 0 {
            return Err(SparkWalletError::InvalidAmount);
        }

        let tx = {
            let state = self
                .state
                .as_mut()
                .ok_or(SparkWalletError::WalletNotInitialized)?;

            if amount_sat > state.balance_sat {
                return Err(SparkWalletError::InsufficientFunds {
                    available: state.balance_sat,
                    requested: amount_sat,
                });
            }

            state.balance_sat -= amount_sat;
            let tx = SparkTransaction {
                txid: transaction_id(&state.wallet_id, state.next_tx_index, recipient, amount_sat),
                direction: SparkTransactionDirection::Sent,
                amount_sat,
                counterparty: recipient.to_string(),
                timestamp: now_timestamp(),
                status: SparkTransactionStatus::Pending,
            };
            state.next_tx_index = state.next_tx_index.saturating_add(1);
            state.transactions.insert(0, tx.clone());
            tx
        };

        self.persist()?;
        Ok(tx)
    }

    pub fn get_transactions(&self) -> Result<Vec<SparkTransaction>, SparkWalletError> {
        let state = self
            .state
            .as_ref()
            .ok_or(SparkWalletError::WalletNotInitialized)?;
        Ok(state.transactions.clone())
    }

    pub fn wallet_info(&self) -> Result<SparkWalletInfo, SparkWalletError> {
        let state = self
            .state
            .as_ref()
            .ok_or(SparkWalletError::WalletNotInitialized)?;
        Ok(SparkWalletInfo {
            mnemonic: state.mnemonic.clone(),
            wallet_id: state.wallet_id.clone(),
            network: state.network.clone(),
        })
    }

    pub fn state_path(&self) -> &Path {
        &self.state_path
    }

    pub fn credit(
        &mut self,
        sender: &str,
        amount_sat: u64,
    ) -> Result<SparkTransaction, SparkWalletError> {
        if sender.trim().is_empty() {
            return Err(SparkWalletError::InvalidRecipient);
        }
        if amount_sat == 0 {
            return Err(SparkWalletError::InvalidAmount);
        }

        let tx = {
            let state = self
                .state
                .as_mut()
                .ok_or(SparkWalletError::WalletNotInitialized)?;

            state.balance_sat = state.balance_sat.saturating_add(amount_sat);
            let tx = SparkTransaction {
                txid: transaction_id(&state.wallet_id, state.next_tx_index, sender, amount_sat),
                direction: SparkTransactionDirection::Received,
                amount_sat,
                counterparty: sender.to_string(),
                timestamp: now_timestamp(),
                status: SparkTransactionStatus::Confirmed,
            };
            state.next_tx_index = state.next_tx_index.saturating_add(1);
            state.transactions.insert(0, tx.clone());
            tx
        };

        self.persist()?;
        Ok(tx)
    }

    fn persist(&self) -> Result<(), SparkWalletError> {
        if let Some(state) = &self.state {
            if let Some(parent) = self.state_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let bytes = serde_json::to_vec_pretty(state)?;
            fs::write(&self.state_path, bytes)?;
        }
        Ok(())
    }
}

fn wallet_id(mnemonic: &str, network: Network) -> String {
    sha256::Hash::hash(format!("{}:{}", network, mnemonic).as_bytes())
        .to_string()
        .chars()
        .take(16)
        .collect()
}

fn transaction_id(wallet_id: &str, tx_index: u64, counterparty: &str, amount_sat: u64) -> String {
    sha256::Hash::hash(
        format!("{}:{}:{}:{}", wallet_id, tx_index, counterparty, amount_sat).as_bytes(),
    )
    .to_string()
}

fn now_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("current time must be after unix epoch")
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spark_wallet_persists_state_to_disk() {
        let state_path =
            std::env::temp_dir().join(format!("spark-wallet-test-{}.json", now_timestamp()));
        if state_path.exists() {
            fs::remove_file(&state_path).unwrap();
        }

        let info = {
            let mut wallet = SparkWallet::load(&state_path).unwrap();
            let info = wallet.create_wallet(Network::Bitcoin, None).unwrap();
            let receive = wallet.receive_address().unwrap();
            assert!(receive.starts_with("spark:bitcoin:"));
            wallet.credit("spark:peer:demo", 42_000).unwrap();
            wallet.send("lnbc1recipient", 12_345).unwrap();
            info
        };

        let wallet = SparkWallet::load(&state_path).unwrap();
        let reloaded_info = wallet.wallet_info().unwrap();
        assert_eq!(reloaded_info.wallet_id, info.wallet_id);
        assert_eq!(wallet.get_balance().unwrap().sats, 29_655);
        assert_eq!(wallet.get_transactions().unwrap().len(), 2);

        fs::remove_file(state_path).unwrap();
    }
}
