//! Experimental HomeKit Accessory Protocol (HAP) pairing over `/pair-setup`.
//!
//! **Not used for mirroring today.** `PairingManager` uses the legacy
//! `pair-setup-pin` flow in `legacy_pin` instead. This module is incomplete
//! (M5/M6 encryption is unfinished) and kept for future work.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{Client, RequestBuilder};
use rotten_core::config::DeviceCredentials;
use rotten_core::debug_log::agent_log;
use rotten_core::device::AirPlayDevice;
use rotten_core::error::{Result, RottenError};
use rotten_crypto::{Ed25519KeyPair, SrpClient, generate_ed25519_keypair};
use tracing::{debug, info};

use crate::tlv::{TlvType, decode, encode};

const PAIR_SETUP: &str = "pair-setup";
const PAIR_PIN_START: &str = "pair-pin-start";
const AIRPLAY_USER_AGENT: &str = "AirPlay/320.20";
const USERNAME: &str = "Pair-Setup";

fn apply_pair_headers(builder: RequestBuilder) -> RequestBuilder {
    builder
        .header("User-Agent", AIRPLAY_USER_AGENT)
        .header("Connection", "keep-alive")
        .header("X-Apple-HKP", "3")
        .header("Content-Type", "application/octet-stream")
}

/// In-progress pairing state between M2 (TV shows PIN) and M3.
pub struct PairingSession {
    client: Client,
    url: String,
    device_id: String,
    salt: Vec<u8>,
    server_pub: Vec<u8>,
    srp: SrpClient,
    keypair: Ed25519KeyPair,
}

/// Send M1 and parse M2. Apple TV should display a PIN after this returns.
pub async fn start_pairing(device: &AirPlayDevice) -> Result<PairingSession> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| RottenError::Pairing(e.to_string()))?;

    let url = format!("{}/{}", device.base_url(), PAIR_SETUP);

    // #region agent log
    agent_log(
        "homekit.rs:start_pairing",
        "pair-setup M1 starting",
        "A",
        serde_json::json!({
            "deviceHost": device.host,
            "devicePort": device.port,
            "pairUrl": url,
        }),
    );
    // #endregion

    let keypair = generate_ed25519_keypair();
    let srp = SrpClient::new();

    let pin_start_url = format!("{}/{}", device.base_url(), PAIR_PIN_START);
    let pin_start_status = match apply_pair_headers(client.post(&pin_start_url)).send().await {
        Ok(r) => r.status().as_u16(),
        Err(e) => {
            // #region agent log
            agent_log(
                "homekit.rs:start_pairing",
                "pair-pin-start failed",
                "E",
                serde_json::json!({ "error": e.to_string() }),
            );
            // #endregion
            return Err(RottenError::Pairing(format!("pair-pin-start: {e:?}")));
        }
    };
    // #region agent log
    agent_log(
        "homekit.rs:start_pairing",
        "pair-pin-start response",
        "E",
        serde_json::json!({ "httpStatus": pin_start_status }),
    );
    // #endregion

    let m1 = encode(&[
        (TlvType::Method, &[0x00][..]),
        (TlvType::State, &[0x01][..]),
    ]);

    debug!(url = %url, "pair-setup M1");
    let resp = match apply_pair_headers(client.post(&url)).body(m1).send().await {
        Ok(r) => r,
        Err(e) => {
            // #region agent log
            agent_log(
                "homekit.rs:start_pairing",
                "pair-setup M1 failed",
                "A",
                serde_json::json!({
                    "error": e.to_string(),
                    "errorDebug": format!("{e:?}"),
                    "isBuilder": e.is_builder(),
                    "pairUrl": url,
                }),
            );
            // #endregion
            return Err(RottenError::Pairing(format!("M1 request: {e:?}")));
        }
    };

    let status = resp.status();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp
        .bytes()
        .await
        .map_err(|e| RottenError::Pairing(format!("M1 response: {e}")))?;
    let m2 = decode(&body);

    // #region agent log
    let mut tlv_keys: Vec<u8> = m2.keys().copied().collect();
    tlv_keys.sort_unstable();
    let hex_prefix: String = body.iter().take(128).map(|b| format!("{b:02x}")).collect();
    agent_log(
        "homekit.rs:start_pairing",
        "pair-setup M2 response",
        "B",
        serde_json::json!({
            "httpStatus": status.as_u16(),
            "contentType": content_type,
            "bodyLen": body.len(),
            "bodyHexPrefix": hex_prefix,
            "tlvKeys": tlv_keys,
            "errorTlv": m2.get(&(TlvType::Error as u8)).and_then(|v| v.first().copied()),
            "stateTlv": m2.get(&(TlvType::State as u8)).cloned(),
            "hasSalt": m2.contains_key(&(TlvType::Salt as u8)),
            "hasPublicKey": m2.contains_key(&(TlvType::PublicKey as u8)),
            "looksOpackFrame": body.first().copied() == Some(0x03) || body.first().copied() == Some(0x04),
            "sentHkpHeader": true,
            "pairPinStartStatus": pin_start_status,
        }),
    );
    // #endregion

    if !status.is_success() {
        return Err(RottenError::Pairing(format!("M2 HTTP {}", status.as_u16())));
    }

    if let Some(err) = m2.get(&(TlvType::Error as u8)) {
        let code = err.first().copied().unwrap_or(0xff);
        return Err(RottenError::Pairing(format!(
            "M2 pairing error TLV: {code}"
        )));
    }

    let salt = m2
        .get(&(TlvType::Salt as u8))
        .ok_or_else(|| RottenError::Pairing("M2 missing salt".into()))?
        .clone();
    let server_pub = m2
        .get(&(TlvType::PublicKey as u8))
        .ok_or_else(|| RottenError::Pairing("M2 missing server public key".into()))?
        .clone();

    Ok(PairingSession {
        client,
        url,
        device_id: device.device_id.clone(),
        salt,
        server_pub,
        srp,
        keypair,
    })
}

/// Complete pairing with the PIN shown on Apple TV (M3 through M5).
pub async fn finish_pairing(session: PairingSession, pin: &str) -> Result<DeviceCredentials> {
    let PairingSession {
        client,
        url,
        device_id,
        salt,
        server_pub,
        srp,
        keypair,
    } = session;

    let (proof, server_proof_expected) = srp.process_challenge(&salt, &server_pub, USERNAME, pin);

    let m3 = encode(&[
        (TlvType::State, &[0x03][..]),
        (TlvType::PublicKey, &srp.client_public()),
        (TlvType::Proof, &proof),
    ]);

    debug!("pair-setup M3");
    let resp = apply_pair_headers(client.post(&url))
        .body(m3)
        .send()
        .await
        .map_err(|e| RottenError::Pairing(format!("M3 request: {e}")))?;

    let body = resp
        .bytes()
        .await
        .map_err(|e| RottenError::Pairing(format!("M3 response: {e}")))?;
    let m4 = decode(&body);

    let server_proof = m4
        .get(&(TlvType::Proof as u8))
        .ok_or_else(|| RottenError::Pairing("M4 missing server proof".into()))?;

    if server_proof != &server_proof_expected {
        return Err(RottenError::Pairing("SRP server proof mismatch".into()));
    }

    let identifier = uuid::Uuid::new_v4().to_string();
    let m5_payload = encode(&[
        (TlvType::Identifier, identifier.as_bytes()),
        (TlvType::PublicKey, &keypair.public_key),
    ]);

    let m5 = encode(&[
        (TlvType::State, &[0x05][..]),
        (TlvType::EncryptedData, &m5_payload),
    ]);

    debug!("pair-setup M5");
    let resp = apply_pair_headers(client.post(&url))
        .body(m5)
        .send()
        .await
        .map_err(|e| RottenError::Pairing(format!("M5 request: {e}")))?;

    if !resp.status().is_success() {
        return Err(RottenError::Pairing(format!(
            "M5 failed: {}",
            resp.status()
        )));
    }

    info!(device_id = %device_id, "pairing complete");

    Ok(DeviceCredentials {
        device_id,
        identifier,
        public_key: keypair.public_key.to_vec(),
        private_key: keypair.private_key.to_vec(),
        server_public_key: vec![],
    })
}

/// One-shot pairing when the PIN is already known (scripts, tests).
pub async fn pair_device(device: &AirPlayDevice, pin: &str) -> Result<DeviceCredentials> {
    let session = start_pairing(device).await?;
    finish_pairing(session, pin).await
}

pub fn device_auth_header(creds: &DeviceCredentials) -> String {
    let id_b64 = STANDARD.encode(&creds.identifier);
    let pk_b64 = STANDARD.encode(&creds.public_key);
    format!(
        "X-Apple-Device-ID: {}\r\nX-Apple-Device-Public-Key: {}",
        id_b64, pk_b64
    )
}
