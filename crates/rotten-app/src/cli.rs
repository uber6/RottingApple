use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use rotten_core::config::{MirrorCipherMode, MirrorConfig, StreamConfig, resolve_credentials_path};
use rotten_core::debug_log::{DEBUG_BUILD_ID, agent_log};
use rotten_discovery::{discover_devices, discover_for, resolve_device};
use rotten_pairing::{PairingManager, format_pin};
use tracing::info;

use crate::mirror::run_mirror;

#[derive(Parser)]
#[command(name = "rottingapple")]
#[command(about = "Mirror or extend your PC display to Apple TV via AirPlay")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Write a boot marker and exit (diagnose startup hangs)
    Probe,
    /// Scan the LAN for AirPlay receivers (Apple TVs)
    Discover {
        /// Discovery timeout in seconds
        #[arg(long, default_value = "5")]
        timeout: u64,
    },
    /// Pair with an Apple TV (stores credentials for future sessions)
    Pair {
        /// Apple TV hostname or IP address
        #[arg(short, long)]
        target: String,
        /// PIN shown on the Apple TV screen (prompted interactively if omitted)
        #[arg(short, long)]
        pin: Option<String>,
        /// AirPlay port
        #[arg(long, default_value = "7000")]
        port: u16,
        /// Force re-pairing even if credentials exist
        #[arg(long)]
        force: bool,
        /// Path to credentials file
        #[arg(long)]
        creds: Option<PathBuf>,
    },
    /// Mirror the PC screen to an Apple TV
    Mirror {
        /// Apple TV hostname or IP (skips mDNS discovery)
        #[arg(short, long)]
        target: Option<String>,
        /// 4-digit PIN for first-time pairing
        #[arg(short, long)]
        pin: Option<String>,
        /// AirPlay port
        #[arg(long, default_value = "7000")]
        port: u16,
        /// Stream width
        #[arg(long, default_value = "1920")]
        width: u32,
        /// Stream height
        #[arg(long, default_value = "1080")]
        height: u32,
        /// Frames per second
        #[arg(long, default_value = "30")]
        fps: u32,
        /// Video bitrate in kbps (0 = auto)
        #[arg(long, default_value = "0")]
        bitrate: u32,
        /// Hardware encoder: auto, nvenc, vaapi, none
        #[arg(long, default_value = "auto")]
        hwaccel: String,
        /// Use synthetic test pattern instead of screen capture
        #[arg(long)]
        test: bool,
        /// Enable audio streaming (experimental)
        #[arg(long)]
        audio: bool,
        /// Force new pairing
        #[arg(long)]
        pair: bool,
        /// Capture only virtual displays (extend mode)
        #[arg(long)]
        virtual_display: bool,
        /// Display index to capture
        #[arg(long)]
        display: Option<u32>,
        /// Path to credentials file
        #[arg(long)]
        creds: Option<PathBuf>,
        /// Verbose debug logging
        #[arg(long)]
        debug: bool,
        /// Send video frames without encryption (debug cipher issues)
        #[arg(long)]
        no_encrypt: bool,
        /// Video cipher: cha-cha (Apple TV default) or aes (legacy UxPlay-style)
        #[arg(long, value_enum, default_value = "cha-cha")]
        cipher: CipherArg,
    },
}

#[derive(Clone, Copy, ValueEnum, Default)]
enum CipherArg {
    Aes,
    #[default]
    #[value(alias = "chacha")]
    ChaCha,
}

impl From<CipherArg> for MirrorCipherMode {
    fn from(v: CipherArg) -> Self {
        match v {
            CipherArg::Aes => MirrorCipherMode::AesCtr,
            CipherArg::ChaCha => MirrorCipherMode::ChaCha,
        }
    }
}

impl Cli {
    pub async fn run(self) -> anyhow::Result<()> {
        match self.command {
            Commands::Probe => {}
            Commands::Discover { timeout } => {
                let devices = discover_for(Duration::from_secs(timeout)).await?;
                if devices.is_empty() {
                    println!("No AirPlay devices found.");
                } else {
                    println!("Found {} AirPlay device(s):\n", devices.len());
                    for d in &devices {
                        println!("  {} — {}:{} ({})", d.name, d.host, d.port, d.device_id);
                        if let Some(model) = &d.model {
                            println!("    model: {model}");
                        }
                    }
                }
            }
            Commands::Pair {
                target,
                pin,
                port,
                force,
                creds,
            } => {
                let device = resolve_device(&target, port).await?;
                let pin = pin.map(|p| format_pin(&p)).transpose()?;
                let creds_path = resolve_credentials_path(creds);
                let mut manager = PairingManager::load(creds_path)?;
                let stored = manager.pair(&device, pin.as_deref(), force).await?;
                println!("Paired with {} ({})", device.name, stored.device_id);
            }
            Commands::Mirror {
                target,
                pin,
                port,
                width,
                height,
                fps,
                bitrate,
                hwaccel,
                test,
                audio,
                pair,
                virtual_display,
                display,
                creds,
                debug,
                no_encrypt,
                cipher,
            } => {
                // #region agent log
                agent_log(
                    "cli.rs:mirror",
                    "mirror command starting",
                    "H17",
                    serde_json::json!({
                        "buildId": DEBUG_BUILD_ID,
                        "target": target.as_deref(),
                        "test": test,
                        "noEncrypt": no_encrypt,
                        "cipher": match cipher {
                            CipherArg::Aes => "aes",
                            CipherArg::ChaCha => "chacha",
                        },
                    }),
                );
                // #endregion

                if debug {
                    tracing::subscriber::set_global_default(
                        tracing_subscriber::fmt().with_env_filter("debug").finish(),
                    )
                    .ok();
                }

                let device = if let Some(t) = target {
                    // #region agent log
                    agent_log(
                        "cli.rs:mirror",
                        "resolving target",
                        "H17",
                        serde_json::json!({ "target": &t, "port": port }),
                    );
                    // #endregion
                    let device = resolve_device(&t, port).await?;
                    // #region agent log
                    agent_log(
                        "cli.rs:mirror",
                        "target resolved",
                        "H17",
                        serde_json::json!({
                            "name": device.name,
                            "host": device.host,
                        }),
                    );
                    // #endregion
                    device
                } else {
                    let devices = discover_devices().await?;
                    devices
                        .into_iter()
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("no AirPlay devices found; use --target"))?
                };

                info!(name = %device.name, host = %device.host, "selected device");

                let config = MirrorConfig {
                    stream: StreamConfig {
                        width,
                        height,
                        fps,
                        bitrate_kbps: bitrate,
                    },
                    pin: pin.map(|p| format_pin(&p)).transpose()?,
                    force_pair: pair,
                    test_mode: test,
                    audio,
                    hw_accel: rotten_core::config::HwAccel::from_str(&hwaccel),
                    credentials_path: resolve_credentials_path(creds),
                    display_index: display,
                    virtual_display_only: virtual_display,
                    no_encrypt,
                    cipher: cipher.into(),
                };

                run_mirror(device, config).await?;
            }
        }
        Ok(())
    }
}
