use rotten_core::device::AirPlayDevice;
use rotten_core::error::Result;
use tracing::{info, warn};

/// Experimental system audio mirroring placeholder.
/// Full AAC-ELD audio streaming requires a separate RTP pipeline.
pub struct AudioMirror {
    device_name: String,
}

impl AudioMirror {
    pub async fn start(device: &AirPlayDevice) -> Result<Self> {
        warn!(
            device = %device.name,
            "audio mirroring is experimental and not yet fully implemented"
        );
        info!("audio capture pipeline reserved for future AAC-ELD stream");
        Ok(Self {
            device_name: device.name.clone(),
        })
    }

    pub async fn stop(self) -> Result<()> {
        info!(device = %self.device_name, "audio mirror stopped");
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub mod platform_audio {
    /// PulseAudio/PipeWire loopback capture hook (stub).
    pub fn list_audio_sources() -> Vec<String> {
        vec!["default".into()]
    }
}

#[cfg(target_os = "windows")]
pub mod platform_audio {
    /// WASAPI loopback capture hook (stub).
    pub fn list_audio_sources() -> Vec<String> {
        vec!["default".into()]
    }
}
