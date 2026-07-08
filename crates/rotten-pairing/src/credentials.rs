use rotten_core::config::{CredentialsStore, DeviceCredentials};
use rotten_core::device::AirPlayDevice;
use rotten_core::error::{Result, RottenError};

use crate::legacy_pin::{finish_pairing, start_pairing};
use crate::prompt::prompt_pin_interactive_async;

/// Manages pairing state and credential persistence.
pub struct PairingManager {
    store: CredentialsStore,
    path: std::path::PathBuf,
}

impl PairingManager {
    pub fn load(path: std::path::PathBuf) -> Result<Self> {
        let store = CredentialsStore::load(&path)?;
        Ok(Self { store, path })
    }

    pub fn has_credentials(&self, device_id: &str) -> bool {
        self.store.get(device_id).is_some()
    }

    pub fn get_credentials(&self, device_id: &str) -> Option<&DeviceCredentials> {
        self.store.get(device_id)
    }

    pub async fn pair(
        &mut self,
        device: &AirPlayDevice,
        pin: Option<&str>,
        force: bool,
    ) -> Result<DeviceCredentials> {
        if !force {
            if let Some(creds) = self.store.get(&device.device_id) {
                return Ok(creds.clone());
            }
        }

        let creds = match pin {
            Some(pin) => {
                let session = start_pairing(device).await?;
                finish_pairing(session, pin).await?
            }
            None => {
                let session = start_pairing(device).await?;
                let pin = prompt_pin_interactive_async().await?;
                finish_pairing(session, &pin).await?
            }
        };

        self.store.upsert(creds.clone());
        self.store.save(&self.path)?;
        Ok(creds)
    }

    pub fn remove(&mut self, device_id: &str) -> Result<()> {
        self.store.remove(device_id);
        self.store.save(&self.path)
    }
}

pub fn format_pin(pin: &str) -> Result<String> {
    let digits: String = pin.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() != 4 && digits.len() != 6 {
        return Err(RottenError::Pairing(format!(
            "PIN must be 4 or 6 digits, got {}",
            digits.len()
        )));
    }
    Ok(digits)
}
