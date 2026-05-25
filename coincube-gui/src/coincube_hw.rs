//! CoinCube Hardware Wallet Integration
//!
//! Implements `async_hwi::HWI` for the CoinCube ESP32-S3 hardware wallet,
//! communicating over USB CDC serial using the line-based ASCII/base64
//! protocol defined in `serial_transport.h`.
//!
//! # Wire Protocol (Host ↔ Device)
//!
//! Host → Device commands (newline-terminated):
//!   `PSBT:<base64-psbt>`       — send PSBT for user review and signing
//!   `VERIFY:<address>`         — ask device to display and verify address
//!   `GET_XPUB:<derivation>`    — request xpub at a BIP32 path (e.g. m/84'/0'/0')
//!   `BALANCE:<conf>:<unconf>`  — push balance in sats (response to device request)
//!   `TX:<txid>:<dir>:<amount>:<confirms>` — push one TX entry
//!
//! Device → Host responses (newline-terminated):
//!   `READY:<fingerprint_hex>:<version>:<xpub_m84>`  — on boot / after reset (extended form)
//!   `READY`                                          — bare form (old firmware, fallback)
//!   `SIGNED:<base64-psbt>`    — signed PSBT returned after user confirmation
//!   `REJECTED`                — user declined on device
//!   `VERIFIED`                — user confirmed address matches
//!   `MISMATCH`                — user confirmed address does NOT match
//!   `XPUB:<base58check>`      — response to GET_XPUB
//!   `ERROR:<code>`            — numeric error code
//!   `BALANCE_REQUEST:<addr>`  — device asks host for balance
//!   `TX_HISTORY:<addr>`       — device asks host for TX history

use async_hwi::{AddressScript, DeviceKind, Error as HWIError, Version, HWI};
use async_trait::async_trait;
use coincube_core::miniscript::bitcoin::{
    bip32::{DerivationPath, Fingerprint, Xpub},
    psbt::Psbt,
};
use std::{str::FromStr, sync::Arc, time::Duration};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Mutex, oneshot, watch};
use tokio_serial::SerialStream;
use tracing::{debug, info, warn};

// ─── Re-exports for hw.rs integration ────────────────────────────────────────

pub use transport::{CoinCubeTransport, GenericCoinCubeTransport, TransportError};

// ─── Constants ────────────────────────────────────────────────────────────────

/// USB VID for Espressif CDC devices (ESP32-S3 default USB serial).
pub const COINCUBE_USB_VID: u16 = 0x303A;
/// USB PID for the CoinCube device.
/// TODO: obtain dedicated PID via pid.codes or ESP32 USB descriptor.
pub const COINCUBE_USB_PID: u16 = 0x4001;

/// Timeout for PSBT signing — user may need time to review on device.
const SIGN_TIMEOUT: Duration = Duration::from_secs(120);
/// Timeout for info queries (fingerprint, xpub, address verify).
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for initial handshake / READY detection.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
/// Timeout for device-initiated balance/history queries.
const DEVICE_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Read timeout for the session background reader — effectively infinite
/// (cancellation is driven by a watch channel).
const SESSION_READ_TIMEOUT: Duration = Duration::from_secs(3600);

// ─── Balance Provider trait ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TxHistoryEntry {
    pub txid: String,
    pub direction: String,
    pub amount: u64,
    pub confirms: u32,
}

#[async_trait]
pub trait BalanceProvider: Send + Sync {
    async fn get_balance(&self, address: &str) -> Result<(u64, u64), String>;
    async fn get_history(&self, address: &str) -> Result<Vec<TxHistoryEntry>, String>;
}

/// Balance provider backed by an Esplora HTTP API (e.g. mempool.space).
pub struct EsploraBalanceProvider {
    client: reqwest::Client,
    base_url: String,
}

impl EsploraBalanceProvider {
    pub fn new(base_url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
        }
    }
}

#[async_trait]
impl BalanceProvider for EsploraBalanceProvider {
    async fn get_balance(&self, address: &str) -> Result<(u64, u64), String> {
        let url = format!("{}/address/{}", self.base_url, address);
        let resp: serde_json::Value = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?
            .json()
            .await
            .map_err(|e| format!("JSON error: {}", e))?;

        let chain = &resp["chain_stats"];
        let mempool = &resp["mempool_stats"];
        let confirmed = chain["funded_txo_sum"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(chain["spent_txo_sum"].as_u64().unwrap_or(0));
        let unconfirmed = mempool["funded_txo_sum"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(mempool["spent_txo_sum"].as_u64().unwrap_or(0));
        Ok((confirmed, unconfirmed))
    }

    async fn get_history(&self, address: &str) -> Result<Vec<TxHistoryEntry>, String> {
        let url = format!("{}/address/{}/txs", self.base_url, address);
        let txs: Vec<serde_json::Value> = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?
            .json()
            .await
            .map_err(|e| format!("JSON error: {}", e))?;

        let mut entries = Vec::new();
        for tx in txs {
            let txid = tx["txid"].as_str().unwrap_or("").to_string();
            let status = &tx["status"];
            let height = status["block_height"].as_u64();
            let confirmed = status["confirmed"].as_bool().unwrap_or(false);

            let mut is_receiving = false;
            let mut amount: u64 = 0;

            if let Some(vout) = tx["vout"].as_array() {
                for out in vout {
                    if out["scriptpubkey_address"].as_str() == Some(address) {
                        is_receiving = true;
                        amount = out["value"].as_u64().unwrap_or(0);
                        break;
                    }
                }
            }

            let direction = if is_receiving { "in" } else { "out" };
            let confirms = if confirmed {
                height.map(|h| h as u32).unwrap_or(1)
            } else {
                0
            };

            entries.push(TxHistoryEntry {
                txid,
                direction: direction.to_string(),
                amount,
                confirms,
            });
        }
        Ok(entries)
    }
}

// ─── Transport layer ──────────────────────────────────────────────────────────

pub mod transport {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
    use tokio_serial::{SerialPortBuilderExt, SerialStream};

    #[derive(Debug)]
    pub enum TransportError {
        Io(std::io::Error),
        Serial(tokio_serial::Error),
        Timeout,
        Utf8(std::string::FromUtf8Error),
    }

    impl std::fmt::Display for TransportError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Io(e) => write!(f, "IO error: {}", e),
                Self::Serial(e) => write!(f, "Serial error: {}", e),
                Self::Timeout => write!(f, "Timeout waiting for device response"),
                Self::Utf8(e) => write!(f, "UTF-8 decode error: {}", e),
            }
        }
    }

    impl From<TransportError> for HWIError {
        fn from(e: TransportError) -> Self {
            HWIError::Device(e.to_string())
        }
    }

    /// Concrete transport for serial port I/O.
    pub type CoinCubeTransport = GenericCoinCubeTransport<SerialStream>;

    /// Bidirectional line-oriented transport over an async read/write stream.
    ///
    /// Wraps a `BufReader<T>` behind a `tokio::sync::Mutex` so the containing
    /// `CoinCubeDevice` can be `Sync`.  The concrete alias `CoinCubeTransport`
    /// uses `SerialStream` (tokio-serial 5.x); tests inject `DuplexStream`.
    pub struct GenericCoinCubeTransport<T> {
        port_path: String,
        inner: Mutex<TransportInner<T>>,
    }

    struct TransportInner<T> {
        reader: BufReader<T>,
    }

    impl CoinCubeTransport {
        /// Open the serial port at 115200 8N1.
        pub fn open(port_path: &str) -> Result<Self, TransportError> {
            let port = tokio_serial::new(port_path, 115_200)
                .data_bits(tokio_serial::DataBits::Eight)
                .stop_bits(tokio_serial::StopBits::One)
                .parity(tokio_serial::Parity::None)
                .open_native_async()
                .map_err(TransportError::Serial)?;
            Ok(Self::with_stream(port_path, port))
        }
    }

    impl<T: AsyncRead + AsyncWrite + Unpin + Send> GenericCoinCubeTransport<T> {
        /// Create a transport wrapping an arbitrary bidirectional stream.
        pub fn with_stream(port_path: &str, stream: T) -> Self {
            Self {
                port_path: port_path.to_string(),
                inner: Mutex::new(TransportInner {
                    reader: BufReader::new(stream),
                }),
            }
        }

        pub fn port_path(&self) -> &str {
            &self.port_path
        }

        /// Send a newline-terminated command.
        pub async fn send(&self, cmd: &str) -> Result<(), TransportError> {
            let line = format!("{}\n", cmd);
            let mut guard = self.inner.lock().await;
            guard
                .reader
                .get_mut()
                .write_all(line.as_bytes())
                .await
                .map_err(TransportError::Io)
        }

        /// Read one newline-terminated line, stripping CR/LF.
        pub async fn recv_line(&self, timeout: Duration) -> Result<String, TransportError> {
            let mut buf = String::new();
            let fut = async {
                let mut guard = self.inner.lock().await;
                guard
                    .reader
                    .read_line(&mut buf)
                    .await
                    .map_err(TransportError::Io)?;
                Ok::<(), TransportError>(())
            };
            tokio::time::timeout(timeout, fut)
                .await
                .map_err(|_| TransportError::Timeout)??;
            Ok(buf.trim_end_matches(['\n', '\r']).to_string())
        }
    }

    impl<T> std::fmt::Debug for GenericCoinCubeTransport<T> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "CoinCubeTransport({})", self.port_path)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        #[tokio::test]
        async fn test_send_writes_newline_terminated_command() {
            let (local, mut remote) = tokio::io::duplex(256);
            let transport = GenericCoinCubeTransport::with_stream("mock", local);

            transport.send("GET_INFO").await.unwrap();

            let mut buf = [0u8; 64];
            let n = remote.read(&mut buf).await.unwrap();
            assert_eq!(&buf[..n], b"GET_INFO\n");
        }

        #[tokio::test]
        async fn test_recv_line_reads_response() {
            let (mut local, remote) = tokio::io::duplex(256);
            let transport = GenericCoinCubeTransport::with_stream("mock", remote);

            local.write_all(b"READY:aabbccdd:1.0.0:xpub6ERApfZwUNrhLCkDtcHTcxd75RbzS1ed54G1LkBUHQVHQKqhMkhgbmJbZRkrgZw4koxb5JaHWkY4ALHY2grBGRjaDMzQLcgJvLJuZZvRcEL\n").await.unwrap();

            let resp = transport
                .recv_line(Duration::from_secs(1))
                .await
                .unwrap();
            assert!(resp.starts_with("READY:"));
        }

        #[tokio::test]
        async fn test_recv_line_strips_trailing_crlf() {
            let (mut local, remote) = tokio::io::duplex(256);
            let transport = GenericCoinCubeTransport::with_stream("mock", remote);

            local.write_all(b"VERIFIED\r\n").await.unwrap();

            let resp = transport
                .recv_line(Duration::from_secs(1))
                .await
                .unwrap();
            assert_eq!(resp, "VERIFIED");
        }

        #[tokio::test]
        async fn test_recv_line_timeout_returns_timeout_error() {
            let (local, _remote) = tokio::io::duplex(256);
            let transport = GenericCoinCubeTransport::with_stream("mock", local);

            let result = transport.recv_line(Duration::from_millis(10)).await;

            assert!(matches!(result, Err(TransportError::Timeout)));
        }

        #[tokio::test]
        async fn test_send_recv_roundtrip() {
            let (mut device_side, transport_side) = tokio::io::duplex(256);
            let transport = GenericCoinCubeTransport::with_stream("mock", transport_side);

            transport.send("GET_XPUB:m/84'/0'/0'").await.unwrap();

            let mut buf = [0u8; 128];
            let n = device_side.read(&mut buf).await.unwrap();
            assert_eq!(&buf[..n], b"GET_XPUB:m/84'/0'/0'\n");

            let xpub_str = "xpub6ERApfZwUNrhLCkDtcHTcxd75RbzS1ed54G1LkBUHQVHQKqhMkhgbmJbZRkrgZw4koxb5JaHWkY4ALHY2grBGRjaDMzQLcgJvLJuZZvRcEL";
            device_side
                .write_all(format!("XPUB:{}\n", xpub_str).as_bytes())
                .await
                .unwrap();

            let resp = transport
                .recv_line(Duration::from_secs(1))
                .await
                .unwrap();
            assert!(resp.starts_with("XPUB:"));
            assert!(resp.contains(xpub_str));
        }
    }
}

// ─── CoinCubeSession — background reader + message routing ─────────────────────

struct CoinCubeSession<T: AsyncRead + AsyncWrite + Unpin + Send + 'static = SerialStream> {
    transport: Arc<GenericCoinCubeTransport<T>>,
    hwi_response: Mutex<Option<oneshot::Sender<String>>>,
    balance_provider: Option<Arc<dyn BalanceProvider + Send + Sync>>,
    cancel_tx: watch::Sender<bool>,
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> CoinCubeSession<T> {
    fn spawn(
        transport: Arc<GenericCoinCubeTransport<T>>,
        balance_provider: Option<Arc<dyn BalanceProvider + Send + Sync>>,
    ) -> Arc<Self> {
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let session = Arc::new(Self {
            transport,
            hwi_response: Mutex::new(None),
            balance_provider,
            cancel_tx,
        });

        let session_clone = session.clone();
        tokio::spawn(async move {
            reader_loop(session_clone, cancel_rx).await;
        });

        session
    }

    fn cancel(&self) {
        let _ = self.cancel_tx.send(true);
    }

    async fn send_and_wait(
        &self,
        command: &str,
        expected_prefixes: &[&str],
        timeout: Duration,
    ) -> Result<String, HWIError> {
        let deadline = tokio::time::Instant::now() + timeout;

        // Set the oneshot BEFORE sending the command.  If the background
        // reader is currently holding the transport Mutex in recv_line,
        // our send will block on that Mutex.  By the time we acquire it
        // and the write completes, the reader may have already read
        // a line — with the oneshot already set, that line gets routed
        // to us instead of being lost.
        let mut rx = {
            let (tx, rx) = oneshot::channel();
            let mut guard = self.hwi_response.lock().await;
            *guard = Some(tx);
            rx
        };

        self.transport
            .send(command)
            .await
            .map_err(|e| HWIError::Device(e.to_string()))?;

        loop {
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .unwrap_or(Duration::ZERO);
            if remaining.is_zero() {
                let mut guard = self.hwi_response.lock().await;
                *guard = None;
                return Err(HWIError::Device(
                    "timed out waiting for device response".to_string(),
                ));
            }

            let line = tokio::time::timeout(remaining, rx)
                .await
                .map_err(|_| {
                    HWIError::Device("timed out waiting for device response".to_string())
                })?
                .map_err(|_| HWIError::Device("transport session closed".to_string()))?;

            for prefix in expected_prefixes {
                if line.starts_with(prefix) {
                    return Ok(line);
                }
            }

            // An unexpected line arrived — could be a device-initiated
            // message that was buffered.  Handle it and set a fresh
            // oneshot for the next expected response.
            if let Some(bp) = &self.balance_provider {
                if let Some(addr) = line.strip_prefix("BALANCE_REQUEST:") {
                    handle_balance_request(&self.transport, bp.as_ref(), addr).await;
                } else if let Some(addr) = line.strip_prefix("TX_HISTORY:") {
                    handle_tx_history_request(&self.transport, bp.as_ref(), addr).await;
                }
            } else {
                debug!(
                    "CoinCube: unexpected response in send_and_wait: {:?}",
                    line
                );
            }

            // Re-arm the oneshot for the next line.
            let (tx, next_rx) = oneshot::channel();
            {
                let mut guard = self.hwi_response.lock().await;
                *guard = Some(tx);
            }
            rx = next_rx;
        }
    }
}

async fn reader_loop<T: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    session: Arc<CoinCubeSession<T>>,
    mut cancel_rx: watch::Receiver<bool>,
) {
    loop {
        let line = tokio::select! {
            _ = cancel_rx.changed() => break,
            result = session.transport.recv_line(SESSION_READ_TIMEOUT) => {
                match result {
                    Ok(line) => line,
                    Err(_) => continue,
                }
            }
        };

        let response_tx = {
            let mut guard = session.hwi_response.lock().await;
            guard.take()
        };

        if let Some(sender) = response_tx {
            let _ = sender.send(line);
            continue;
        }

        if let Some(bp) = &session.balance_provider {
            if let Some(addr) = line.strip_prefix("BALANCE_REQUEST:") {
                handle_balance_request(&session.transport, bp.as_ref(), addr).await;
                continue;
            }
            if let Some(addr) = line.strip_prefix("TX_HISTORY:") {
                handle_tx_history_request(&session.transport, bp.as_ref(), addr).await;
                continue;
            }
        }

        debug!("CoinCube session: unhandled line: {:?}", line);
    }
}

async fn handle_balance_request<T: AsyncRead + AsyncWrite + Unpin + Send>(
    transport: &GenericCoinCubeTransport<T>,
    bp: &(dyn BalanceProvider + Send + Sync),
    address: &str,
) {
    match tokio::time::timeout(DEVICE_REQUEST_TIMEOUT, bp.get_balance(address)).await {
        Ok(Ok((confirmed, unconfirmed))) => {
            let resp = format!("BALANCE:{}:{}", confirmed, unconfirmed);
            if let Err(e) = transport.send(&resp).await {
                warn!("CoinCube: failed to send balance response: {}", e);
            }
        }
        Ok(Err(e)) => warn!("CoinCube: balance query failed for {}: {}", address, e),
        Err(_) => warn!("CoinCube: balance query timed out for {}", address),
    }
}

async fn handle_tx_history_request<T: AsyncRead + AsyncWrite + Unpin + Send>(
    transport: &GenericCoinCubeTransport<T>,
    bp: &(dyn BalanceProvider + Send + Sync),
    address: &str,
) {
    match tokio::time::timeout(DEVICE_REQUEST_TIMEOUT, bp.get_history(address)).await {
        Ok(Ok(entries)) => {
            for entry in entries {
                let resp = format!(
                    "TX:{}:{}:{}:{}",
                    entry.txid, entry.direction, entry.amount, entry.confirms
                );
                if let Err(e) = transport.send(&resp).await {
                    warn!("CoinCube: failed to send TX entry: {}", e);
                    break;
                }
            }
        }
        Ok(Err(e)) => warn!("CoinCube: TX history query failed for {}: {}", address, e),
        Err(_) => warn!("CoinCube: TX history query timed out for {}", address),
    }
}

// ─── Device info parsed from READY message ───────────────────────────────────

#[derive(Debug, Clone)]
pub struct CoinCubeInfo {
    pub fingerprint: Fingerprint,
    pub version: Version,
    /// xpub at m/84'/0'/0' — the default native-segwit account.
    pub account_xpub: Xpub,
}

/// Parse `READY:<fingerprint_hex>:<version>:<xpub_base58>` or bare `READY`.
fn parse_ready(line: &str) -> Option<CoinCubeInfo> {
    let rest = line.strip_prefix("READY:")?;
    let parts: Vec<&str> = rest.splitn(3, ':').collect();
    if parts.len() != 3 {
        return None;
    }

    // fingerprint: 8 hex chars = 4 bytes
    let fg_bytes = hex::decode(parts[0]).ok()?;
    if fg_bytes.len() != 4 {
        return None;
    }
    let fingerprint = Fingerprint::from([fg_bytes[0], fg_bytes[1], fg_bytes[2], fg_bytes[3]]);

    // version: e.g. "1.0.0"
    let ver_parts: Vec<&str> = parts[1].splitn(3, '.').collect();
    let major = ver_parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = ver_parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch = ver_parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    // xpub
    let account_xpub = Xpub::from_str(parts[2]).ok()?;

    Some(CoinCubeInfo {
        fingerprint,
        version: Version {
            major,
            minor,
            patch,
            prerelease: None,
        },
        account_xpub,
    })
}

// ─── CoinCubeDevice ───────────────────────────────────────────────────────────

/// A connected CoinCube hardware wallet, ready for HWI operations.
pub struct CoinCubeDevice {
    port: String,
    transport: Arc<CoinCubeTransport>,
    session: Arc<CoinCubeSession>,
    fingerprint: Fingerprint,
    version: Version,
    account_xpub: Xpub,
}

impl std::fmt::Debug for CoinCubeDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CoinCubeDevice({}, fg={})",
            self.port,
            self.fingerprint
        )
    }
}

impl Drop for CoinCubeDevice {
    fn drop(&mut self) {
        self.session.cancel();
    }
}

impl CoinCubeDevice {
    /// Open a CoinCube device on the given serial port.
    ///
    /// Sends a `GET_INFO` nudge and waits for a `READY:…` response.
    /// If the device responds with bare `READY` (legacy firmware), falls back
    /// by querying `GET_XPUB:m/84'/0'/0'` to derive fingerprint and xpub info.
    /// Returns `Err(HWIError::DeviceNotFound)` if the port times out.
    ///
    /// After the handshake, spawns a background session task that handles
    /// device-initiated messages (BALANCE_REQUEST, TX_HISTORY) between
    /// HWI operations.
    pub async fn new(port_path: &str) -> Result<Self, HWIError> {
        let transport =
            CoinCubeTransport::open(port_path).map_err(|e| HWIError::Device(e.to_string()))?;

        let _ = transport.send("GET_INFO").await;

        let info = loop {
            let line = transport
                .recv_line(HANDSHAKE_TIMEOUT)
                .await
                .map_err(|_| HWIError::DeviceNotFound)?;

            debug!("CoinCube handshake line: {:?}", line);

            if line.starts_with("READY:") {
                match parse_ready(&line) {
                    Some(i) => break i,
                    None => {
                        warn!("CoinCube: malformed READY line: {:?}", line);
                        return Err(HWIError::Device("malformed READY response".to_string()));
                    }
                }
            } else if line == "READY" {
                info!("CoinCube: legacy firmware detected (bare READY), falling back to GET_XPUB");
                break legacy_handshake(&transport).await?;
            }
        };

        let transport_arc = Arc::new(transport);
        let session = CoinCubeSession::spawn(transport_arc.clone(), None);

        Ok(Self {
            port: port_path.to_string(),
            transport: transport_arc,
            session,
            fingerprint: info.fingerprint,
            version: info.version,
            account_xpub: info.account_xpub,
        })
    }

    /// Open a CoinCube device and immediately start the session with a
    /// balance provider for handling device-initiated queries.
    pub async fn new_with_balance_provider(
        port_path: &str,
        balance_provider: Arc<dyn BalanceProvider + Send + Sync>,
    ) -> Result<Self, HWIError> {
        let transport =
            CoinCubeTransport::open(port_path).map_err(|e| HWIError::Device(e.to_string()))?;

        let _ = transport.send("GET_INFO").await;

        let info = loop {
            let line = transport
                .recv_line(HANDSHAKE_TIMEOUT)
                .await
                .map_err(|_| HWIError::DeviceNotFound)?;

            debug!("CoinCube handshake line: {:?}", line);

            if line.starts_with("READY:") {
                match parse_ready(&line) {
                    Some(i) => break i,
                    None => {
                        warn!("CoinCube: malformed READY line: {:?}", line);
                        return Err(HWIError::Device("malformed READY response".to_string()));
                    }
                }
            } else if line == "READY" {
                info!("CoinCube: legacy firmware detected (bare READY), falling back to GET_XPUB");
                break legacy_handshake(&transport).await?;
            }
        };

        let transport_arc = Arc::new(transport);
        let session =
            CoinCubeSession::spawn(transport_arc.clone(), Some(balance_provider));

        Ok(Self {
            port: port_path.to_string(),
            transport: transport_arc,
            session,
            fingerprint: info.fingerprint,
            version: info.version,
            account_xpub: info.account_xpub,
        })
    }

    /// Enumerate candidate serial port paths for CoinCube devices.
    ///
    /// Filters by USB VID/PID where the OS provides it, then falls back
    /// to port description substring matching.
    pub fn enumerate_ports() -> Result<Vec<String>, HWIError> {
        let ports = tokio_serial::available_ports().map_err(|e| HWIError::Device(e.to_string()))?;

        let candidates: Vec<String> = ports
            .into_iter()
            .filter(|p| match &p.port_type {
                tokio_serial::SerialPortType::UsbPort(info) => {
                    (info.vid == COINCUBE_USB_VID && info.pid == COINCUBE_USB_PID)
                        || info
                            .product
                            .as_deref()
                            .map(|s| s.to_ascii_lowercase().contains("coincube"))
                            .unwrap_or(false)
                }
                _ => false,
            })
            .map(|p| p.port_name)
            .collect();

        debug!("CoinCube: candidate ports: {:?}", candidates);
        Ok(candidates)
    }

    /// Unique device ID string for use as the `HardwareWallet` id.
    pub fn device_id(&self) -> String {
        format!("coincube-{}", self.port)
    }

    pub fn transport(&self) -> &Arc<CoinCubeTransport> {
        &self.transport
    }
}

/// Handshake with a legacy CoinCube that only sends bare `READY`.
///
/// Falls back to sending `GET_XPUB:m/84'/0'/0'` to obtain the account xpub
/// and derive the master fingerprint. Version is set to an unknown placeholder.
async fn legacy_handshake(transport: &CoinCubeTransport) -> Result<CoinCubeInfo, HWIError> {
    transport
        .send("GET_XPUB:m/84'/0'/0'")
        .await
        .map_err(|e| HWIError::Device(e.to_string()))?;

    let line = transport
        .recv_line(QUERY_TIMEOUT)
        .await
        .map_err(|e| HWIError::Device(e.to_string()))?;

    let xpub_str = line
        .strip_prefix("XPUB:")
        .ok_or_else(|| HWIError::Device("no XPUB response from legacy device".to_string()))?;

    let account_xpub = Xpub::from_str(xpub_str).map_err(|e| HWIError::Device(e.to_string()))?;

    let fingerprint = account_xpub.fingerprint();

    Ok(CoinCubeInfo {
        fingerprint,
        version: Version {
            major: 0,
            minor: 0,
            patch: 0,
            prerelease: Some("legacy".to_string()),
        },
        account_xpub,
    })
}

// ─── HWI trait implementation ─────────────────────────────────────────────────

#[async_trait]
impl HWI for CoinCubeDevice {
    fn device_kind(&self) -> DeviceKind {
        DeviceKind::Specter
    }

    async fn get_version(&self) -> Result<Version, HWIError> {
        Ok(self.version.clone())
    }

    async fn get_master_fingerprint(&self) -> Result<Fingerprint, HWIError> {
        Ok(self.fingerprint)
    }

    async fn get_extended_pubkey(&self, path: &DerivationPath) -> Result<Xpub, HWIError> {
        let path_str = path.to_string();
        let line = self
            .session
            .send_and_wait(
                &format!("GET_XPUB:{}", path_str),
                &["XPUB:", "ERROR:"],
                QUERY_TIMEOUT,
            )
            .await?;

        if let Some(xpub_str) = line.strip_prefix("XPUB:") {
            Xpub::from_str(xpub_str).map_err(|e| HWIError::Device(e.to_string()))
        } else {
            Err(HWIError::Device(format!(
                "device returned error for GET_XPUB: {}",
                line
            )))
        }
    }

    async fn register_wallet(
        &self,
        _name: &str,
        _policy: &str,
    ) -> Result<Option<[u8; 32]>, HWIError> {
        Err(HWIError::UnimplementedMethod)
    }

    async fn is_wallet_registered(&self, _name: &str, _policy: &str) -> Result<bool, HWIError> {
        Ok(false)
    }

    async fn display_address(&self, script: &AddressScript) -> Result<(), HWIError> {
        let address = match script {
            AddressScript::Miniscript { index, change } => {
                format!("index={},change={}", index, change)
            }
            AddressScript::P2TR(path) => path.to_string(),
            #[allow(unreachable_patterns)]
            _ => {
                return Err(HWIError::UnimplementedMethod);
            }
        };

        let line = self
            .session
            .send_and_wait(
                &format!("VERIFY:{}", address),
                &["VERIFIED", "MISMATCH", "ERROR:"],
                QUERY_TIMEOUT,
            )
            .await?;

        if line == "VERIFIED" {
            Ok(())
        } else if line == "MISMATCH" {
            Err(HWIError::Device(
                "address mismatch confirmed by user".to_string(),
            ))
        } else {
            Err(HWIError::Device(format!("device error: {}", line)))
        }
    }

    async fn sign_tx(&self, tx: &mut Psbt) -> Result<(), HWIError> {
        let raw = tx.serialize();
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &raw);

        let line = self
            .session
            .send_and_wait(
                &format!("PSBT:{}", b64),
                &["SIGNED:", "REJECTED", "ERROR:"],
                SIGN_TIMEOUT,
            )
            .await?;

        if let Some(b64_signed) = line.strip_prefix("SIGNED:") {
            let signed_raw =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64_signed)
                    .map_err(|e| HWIError::Device(e.to_string()))?;
            let signed_psbt =
                Psbt::deserialize(&signed_raw).map_err(|e| HWIError::Device(e.to_string()))?;
            *tx = signed_psbt;
            Ok(())
        } else if line == "REJECTED" {
            Err(HWIError::UserRefused)
        } else {
            Err(HWIError::Device(format!("signing failed: {}", line)))
        }
    }
}

// ─── Unit Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Valid BIP32 test vector xpub (from bitcoin crate test suite)
    const VALID_XPUB: &str = "xpub6ERApfZwUNrhLCkDtcHTcxd75RbzS1ed54G1LkBUHQVHQKqhMkhgbmJbZRkrgZw4koxb5JaHWkY4ALHY2grBGRjaDMzQLcgJvLJuZZvRcEL";

    #[test]
    fn test_parse_ready_extended() {
        let line = format!("READY:aabbccdd:1.2.3:{}", VALID_XPUB);
        let info = parse_ready(&line);
        assert!(info.is_some(), "should parse extended READY");
        let info = info.unwrap();
        assert_eq!(info.fingerprint.as_bytes(), &[0xaa, 0xbb, 0xcc, 0xdd]);
        assert_eq!(info.version.major, 1);
        assert_eq!(info.version.minor, 2);
        assert_eq!(info.version.patch, 3);
    }

    #[test]
    fn test_parse_ready_bare_returns_none() {
        let line = "READY";
        assert!(parse_ready(line).is_none());
    }

    #[test]
    fn test_parse_ready_with_trailing_whitespace() {
        let line = format!("READY:aabbccdd:1.0.0:{}\r", VALID_XPUB);
        assert!(parse_ready(&line).is_none(), "trailing \\r not stripped");
    }

    #[test]
    fn test_parse_ready_bad_fingerprint() {
        let line = "READY:ZZZZZZZZ:1.0.0:xpubBAD";
        assert!(parse_ready(line).is_none());
    }

    #[test]
    fn test_parse_ready_insufficient_parts() {
        let line = "READY:aabbccdd:1.0.0";
        assert!(parse_ready(line).is_none());
    }

    #[test]
    fn test_parse_ready_bad_version_defaults_to_zero() {
        let line = format!("READY:aabbccdd:badver:{}", VALID_XPUB);
        let info = parse_ready(&line);
        assert!(
            info.is_some(),
            "bad version should still parse with defaults"
        );
        let info = info.unwrap();
        assert_eq!(info.version.major, 0);
        assert_eq!(info.version.minor, 0);
        assert_eq!(info.version.patch, 0);
    }

    #[test]
    fn test_is_ready_extended_vs_bare() {
        let extended = format!("READY:00000000:0.0.0:{}", VALID_XPUB);
        let bare = "READY";

        assert!(
            parse_ready(&extended).is_some(),
            "extended format should parse"
        );
        assert!(
            parse_ready(bare).is_none(),
            "bare format should return None for fallback"
        );
    }

    // ─── CoinCubeSession tests ────────────────────────────────────────────

    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

    struct MockBalanceProvider {
        balance: (u64, u64),
        history: Vec<TxHistoryEntry>,
    }

    #[async_trait]
    impl BalanceProvider for MockBalanceProvider {
        async fn get_balance(&self, _address: &str) -> Result<(u64, u64), String> {
            Ok(self.balance)
        }

        async fn get_history(&self, _address: &str) -> Result<Vec<TxHistoryEntry>, String> {
            Ok(self.history.clone())
        }
    }

    fn mock_session(
        bp: Option<MockBalanceProvider>,
    ) -> (
        Arc<CoinCubeSession<DuplexStream>>,
        DuplexStream,
    ) {
        let (local, remote) = tokio::io::duplex(4096);
        let transport =
            Arc::new(GenericCoinCubeTransport::<DuplexStream>::with_stream(
                "mock", local,
            ));
        let bp: Option<Arc<dyn BalanceProvider + Send + Sync>> =
            bp.map(|p| Arc::new(p) as Arc<dyn BalanceProvider + Send + Sync>);
        let session = CoinCubeSession::<DuplexStream>::spawn(transport, bp);
        (session, remote)
    }

    #[tokio::test]
    async fn test_session_balance_request_handled() {
        let (session, mut device_side) = mock_session(Some(MockBalanceProvider {
            balance: (100_000, 5_000),
            history: vec![],
        }));

        device_side
            .write_all(b"BALANCE_REQUEST:bc1qtest\n")
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(200)).await;

        let mut buf = [0u8; 256];
        let n = tokio::time::timeout(Duration::from_millis(500), device_side.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        let output = String::from_utf8_lossy(&buf[..n]);
        assert!(
            output.contains("BALANCE:100000:5000"),
            "should respond with balance, got: {}",
            output
        );

        // Spawn the response write on a short delay so send_and_wait
        // has time to set up its oneshot before the line arrives.
        let device_side = Arc::new(tokio::sync::Mutex::new(device_side));
        let ds = device_side.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            ds.lock()
                .await
                .write_all(b"XPUB:xpub6TestKey\n")
                .await
                .unwrap();
        });

        let result = session
            .send_and_wait("GET_XPUB:test", &["XPUB:"], Duration::from_secs(2))
            .await;
        assert!(result.is_ok(), "HWI routing should work after balance request");
    }

    #[tokio::test]
    async fn test_session_tx_history_request_handled() {
        let (session, mut device_side) = mock_session(Some(MockBalanceProvider {
            balance: (0, 0),
            history: vec![
                TxHistoryEntry {
                    txid: "abc123".to_string(),
                    direction: "in".to_string(),
                    amount: 50_000,
                    confirms: 3,
                },
                TxHistoryEntry {
                    txid: "def456".to_string(),
                    direction: "out".to_string(),
                    amount: 30_000,
                    confirms: 10,
                },
            ],
        }));

        device_side
            .write_all(b"TX_HISTORY:bc1qtest\n")
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(200)).await;

        let mut buf = [0u8; 512];
        let n = tokio::time::timeout(Duration::from_millis(500), device_side.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        let output = String::from_utf8_lossy(&buf[..n]);
        assert!(
            output.contains("TX:abc123:in:50000:3"),
            "should send TX entry for abc123, got: {}",
            output
        );
        assert!(
            output.contains("TX:def456:out:30000:10"),
            "should send TX entry for def456, got: {}",
            output
        );
        drop(session);
    }

    #[tokio::test]
    async fn test_session_shutdown_on_cancel() {
        let (session, _device_side) = mock_session(None);

        session.cancel();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let result = session
            .send_and_wait("TEST", &["RESPONSE:"], Duration::from_millis(200))
            .await;
        assert!(result.is_err(), "send_and_wait should fail after session cancelled");
    }

    #[tokio::test]
    async fn test_session_without_balance_provider_ignores_requests() {
        let (session, mut device_side) = mock_session(None);

        device_side
            .write_all(b"BALANCE_REQUEST:bc1qtest\n")
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Spawn the response on a delay so send_and_wait sets up its
        // oneshot before the background reader consumes the line.
        let device_side = Arc::new(tokio::sync::Mutex::new(device_side));
        let ds = device_side.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            ds.lock()
                .await
                .write_all(b"VERIFIED\n")
                .await
                .unwrap();
        });

        let result = session
            .send_and_wait("VERIFY:addr", &["VERIFIED"], Duration::from_secs(2))
            .await;
        assert!(result.is_ok(), "HWI routing should work without balance provider");
    }

    #[tokio::test]
    async fn test_session_send_and_wait_routing() {
        let (session, mut device_side) = mock_session(None);

        device_side
            .write_all(b"SIGNED:deadbeef\n")
            .await
            .unwrap();

        let result = session
            .send_and_wait("PSBT:test", &["SIGNED:"], Duration::from_secs(1))
            .await;
        assert!(result.is_ok(), "send_and_wait should receive SIGNED response");
        let line = result.unwrap();
        assert!(
            line.starts_with("SIGNED:"),
            "expected SIGNED: prefix, got: {}",
            line
        );
    }

    #[tokio::test]
    async fn test_session_send_and_wait_timeout() {
        let (session, _device_side) = mock_session(None);

        let result = session
            .send_and_wait("GET_XPUB:test", &["XPUB:"], Duration::from_millis(50))
            .await;
        assert!(
            result.is_err(),
            "send_and_wait should timeout when no response arrives"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, HWIError::Device(_)),
            "timeout should return HWIError::Device"
        );
    }
}
