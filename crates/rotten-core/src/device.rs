use serde::{Deserialize, Serialize};

/// Parsed feature flags from AirPlay TXT record `features` field.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceFeatures {
    pub raw: u64,
}

impl DeviceFeatures {
    /// Parse AirPlay `features` TXT (`0xLOW,0xHIGH` or single hex value).
    pub fn from_hex(hex: &str) -> Self {
        let s = hex.trim();
        if let Some((lo, hi)) = s.split_once(',') {
            let lo = parse_hex_u64(lo);
            let hi = parse_hex_u64(hi);
            return Self {
                raw: (hi << 32) | lo,
            };
        }
        Self {
            raw: parse_hex_u64(s),
        }
    }

    pub fn supports_screen_mirroring(&self) -> bool {
        const FEATURE_SCREEN: u64 = 1 << 8;
        self.raw & FEATURE_SCREEN != 0 || self.raw & 0x80 != 0
    }

    /// FairPlay SAP 2.5 — modern Apple receivers buffer at very low latency.
    pub fn supports_fairplay_sap(&self) -> bool {
        const FEATURE_FP_SAP_25: u64 = 1 << 14;
        self.raw & FEATURE_FP_SAP_25 != 0
    }

    /// Minimum playout lead for receivers without robust jitter buffers.
    pub fn playout_latency_floor_ms(&self) -> u64 {
        if self.supports_fairplay_sap() { 0 } else { 500 }
    }
}

fn parse_hex_u64(s: &str) -> u64 {
    let s = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    u64::from_str_radix(s, 16).unwrap_or(0)
}

/// A discovered or manually specified AirPlay receiver (Apple TV).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirPlayDevice {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub device_id: String,
    pub model: Option<String>,
    pub features: DeviceFeatures,
    /// Primary display width from `/info` `displays[0]` (presentation size in codec header).
    pub display_width: Option<u32>,
    /// Primary display height from `/info` `displays[0]`.
    pub display_height: Option<u32>,
    pub pi: Option<String>,
    pub pk: Option<String>,
}

impl AirPlayDevice {
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn base_url(&self) -> String {
        format!(
            "http://{}:{}",
            crate::debug_log::format_host_for_url(&self.host),
            self.port
        )
    }
}

#[cfg(test)]
mod tests {
    use super::DeviceFeatures;

    #[test]
    fn parses_comma_separated_features() {
        let f = DeviceFeatures::from_hex("0x527feec,0x0");
        assert_ne!(f.raw, 0);
    }
}
