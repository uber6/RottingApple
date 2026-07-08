//! Minimal mirror audio RTP (ChaCha + ALAC silence) — Apple TV expects audio after frame 1.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use plist::Value;
use rotten_core::debug_log::agent_log;
use rotten_core::device::DeviceFeatures;
use rotten_core::error::{Result, RottenError};
use rotten_crypto::chacha64_seal;
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;
use tokio::time::{Duration as TokioDuration, MissedTickBehavior, interval};

use crate::ntp::ntp_boot_with_epoch;

const AUDIO_SPF: u16 = 352;
/// Legacy default kept for callers; prefer [`playout_latency_samples`].
pub const AUDIO_LATENCY_SAMPLES: u32 = 44;
const TARGET_LATENCY_MS: u64 = 1;
const AUDIO_CHACHA_NONCE_SIZE: usize = 8;

/// Playout latency in 44.1 kHz samples (doubletake `samplesFor44k1(TargetLatency())`).
pub fn playout_latency_samples(features: &DeviceFeatures) -> u32 {
    let floor_ms = if features.raw == 0 {
        // `/info` or mDNS features missing: mirror targets Apple TV; use low latency.
        0
    } else {
        features.playout_latency_floor_ms()
    };
    let target_ms = TARGET_LATENCY_MS.max(floor_ms);
    samples_for_44k1(Duration::from_millis(target_ms))
}

fn samples_for_44k1(d: Duration) -> u32 {
    let samples = (d.as_secs_f64() * 44_100.0).round() as i64;
    samples.clamp(1, i64::from(u32::MAX)) as u32
}

/// Parsed audio stream ports from RTSP SETUP response (stream type 96).
pub fn plist_audio_ports(body: &[u8]) -> Option<(u16, u16)> {
    let value: Value = plist::from_bytes(body).ok()?;
    let dict = value.as_dictionary()?;
    let streams = dict.get("streams")?.as_array()?;
    for stream in streams {
        let sd = stream.as_dictionary()?;
        if plist_int(sd.get("type")?) != 96 {
            continue;
        }
        let (data, control) = plist_stream_ports(sd);
        if data > 0 && control > 0 {
            return Some((data, control));
        }
    }
    None
}

/// Match doubletake `plistStreamPorts`: legacy dataPort/controlPort or streamConnections RTP/RTCP keys.
fn plist_stream_ports(stream: &plist::Dictionary) -> (u16, u16) {
    let mut data_port =
        plist_int(stream.get("dataPort").unwrap_or(&Value::Integer(0.into()))) as u16;
    let mut control_port = plist_int(
        stream
            .get("controlPort")
            .unwrap_or(&Value::Integer(0.into())),
    ) as u16;

    if let Some(sc) = stream
        .get("streamConnections")
        .and_then(|v| v.as_dictionary())
    {
        if let Some(rtp) = sc
            .get("streamConnectionTypeRTP")
            .and_then(|v| v.as_dictionary())
        {
            if let Some(port) = rtp.get("streamConnectionKeyPort") {
                let p = plist_int(port);
                if p > 0 {
                    data_port = p as u16;
                }
            }
        }
        if let Some(rtcp) = sc
            .get("streamConnectionTypeRTCP")
            .and_then(|v| v.as_dictionary())
        {
            if let Some(port) = rtcp.get("streamConnectionKeyPort") {
                let p = plist_int(port);
                if p > 0 {
                    control_port = p as u16;
                }
            }
        }
    }

    (data_port, control_port)
}

fn plist_int(value: &Value) -> i64 {
    match value {
        Value::Integer(i) => i.as_signed().unwrap_or(0),
        Value::Real(f) => *f as i64,
        _ => 0,
    }
}

/// UDP sockets + keys needed to stream silence audio after the first video frame.
pub struct MirrorAudioSetup {
    pub host: String,
    pub chacha_key: [u8; 32],
    pub remote_data_port: u16,
    pub remote_control_port: u16,
    pub ctrl_socket: UdpSocket,
    pub data_socket: UdpSocket,
    pub latency_samples: u32,
}

/// Spawn silence audio RTP after the first video frame broadcast fires.
pub fn spawn_mirror_audio_silence(
    setup: MirrorAudioSetup,
    mut first_frame: tokio::sync::broadcast::Receiver<()>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if first_frame.recv().await.is_err() {
            return;
        }
        if let Err(e) = run_audio_silence_loop(setup).await {
            agent_log(
                "audio_rtp.rs:run_audio_silence_loop",
                "mirror audio stream error",
                "H61",
                serde_json::json!({ "error": e.to_string() }),
            );
        }
    })
}

async fn run_audio_silence_loop(setup: MirrorAudioSetup) -> Result<()> {
    let data_addr = format!("{}:{}", setup.host, setup.remote_data_port);
    let ctrl_addr = format!("{}:{}", setup.host, setup.remote_control_port);

    let chacha_key = setup.chacha_key;
    let chacha_nonce = Arc::new(AtomicU64::new(0));
    let ssrc: u32 = 0;
    let alac_frame = encode_alac_verbatim_silence(AUDIO_SPF);

    // #region agent log
    agent_log(
        "audio_rtp.rs:run_audio_silence_loop",
        "mirror audio silence starting",
        "H61",
        serde_json::json!({
            "remoteDataPort": setup.remote_data_port,
            "remoteControlPort": setup.remote_control_port,
            "alacBytes": alac_frame.len(),
            "dataAddr": data_addr,
        }),
    );
    // #endregion

    let latency_samples = setup.latency_samples;
    let ntp_now = ntp_boot_with_epoch();
    for i in 0..7 {
        send_sync_packet(
            &setup.ctrl_socket,
            &ctrl_addr,
            ntp_now,
            0,
            latency_samples,
            true,
        )
        .await?;
        if i == 0 {
            // #region agent log
            agent_log(
                "audio_rtp.rs:run_audio_silence_loop",
                "audio sync burst at rtp=0",
                "H71",
                serde_json::json!({
                    "syncPackets": 7,
                    "latencySamples": latency_samples,
                }),
            );
            // #endregion
        }
    }

    let mut seq: u16 = 1;
    let mut rtp_time: u32 = latency_samples;
    let frame_samples = AUDIO_SPF as u32;

    let mut ticker = interval(TokioDuration::from_millis(
        (u64::from(frame_samples) * 1000 / 44100).max(1),
    ));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut sync_fast = interval(TokioDuration::from_millis(200));
    sync_fast.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let sync_deadline = tokio::time::Instant::now() + TokioDuration::from_secs(5);
    let mut packets_sent: u64 = 0;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                send_audio_packet(
                    &setup.data_socket,
                    &data_addr,
                    &chacha_key,
                    chacha_nonce.clone(),
                    &alac_frame,
                    rtp_time,
                    seq,
                    ssrc,
                ).await?;
                seq = seq.wrapping_add(1);
                rtp_time = rtp_time.wrapping_add(frame_samples);
                packets_sent += 1;
                if packets_sent == 1 || packets_sent % 100 == 0 {
                    agent_log(
                        "audio_rtp.rs:run_audio_silence_loop",
                        "audio RTP packet sent",
                        "H61",
                        serde_json::json!({
                            "packetsSent": packets_sent,
                            "seq": seq,
                            "rtpTime": rtp_time,
                        }),
                    );
                }
            }
            _ = sync_fast.tick(), if tokio::time::Instant::now() < sync_deadline => {
                let sync_rtp = sync_rtp_for(rtp_time, latency_samples);
                // #region agent log
                if sync_rtp <= AUDIO_SPF as u32 {
                    agent_log(
                        "audio_rtp.rs:run_audio_silence_loop",
                        "audio periodic sync",
                        "H62",
                        serde_json::json!({
                            "currentRtp": rtp_time,
                            "syncRtp": sync_rtp,
                            "nextRtp": rtp_time,
                        }),
                    );
                }
                // #endregion
                let _ = send_sync_packet(
                    &setup.ctrl_socket,
                    &ctrl_addr,
                    ntp_boot_with_epoch(),
                    rtp_time,
                    latency_samples,
                    false,
                )
                .await;
            }
        }
    }
}

async fn send_audio_packet(
    socket: &UdpSocket,
    remote: &str,
    chacha_key: &[u8; 32],
    nonce_counter: Arc<AtomicU64>,
    payload: &[u8],
    rtp_time: u32,
    seq: u16,
    ssrc: u32,
) -> Result<()> {
    let mut header = [0u8; 12];
    header[0] = 0x80;
    header[1] = 0x60;
    header[2..4].copy_from_slice(&seq.to_be_bytes());
    header[4..8].copy_from_slice(&rtp_time.to_be_bytes());
    header[8..12].copy_from_slice(&ssrc.to_be_bytes());

    let nonce_val = nonce_counter.fetch_add(1, Ordering::Relaxed);
    let nonce_bytes = nonce_val.to_le_bytes();
    let aad = &header[4..12];

    let sealed = chacha64_seal(chacha_key, &nonce_bytes, payload, aad);

    let mut packet = Vec::with_capacity(12 + sealed.len() + AUDIO_CHACHA_NONCE_SIZE);
    packet.extend_from_slice(&header);
    packet.extend_from_slice(&sealed);
    packet.extend_from_slice(&nonce_bytes);

    if nonce_val == 0 {
        // #region agent log
        agent_log(
            "audio_rtp.rs:send_audio_packet",
            "first audio packet chacha64",
            "H69",
            serde_json::json!({
                "cipher": "chacha64",
                "packetLen": packet.len(),
                "sealedLen": sealed.len(),
                "payloadLen": payload.len(),
                "seq": seq,
            }),
        );
        // #endregion
    }

    socket
        .send_to(&packet, remote)
        .await
        .map_err(|e| RottenError::Protocol(format!("audio RTP send: {e}")))?;
    Ok(())
}

fn sync_rtp_for(rtp_now: u32, latency_samples: u32) -> u32 {
    if rtp_now >= latency_samples {
        rtp_now - latency_samples
    } else {
        rtp_now
    }
}

async fn send_sync_packet(
    socket: &UdpSocket,
    remote: &str,
    ntp_time: u64,
    rtp_now: u32,
    latency_samples: u32,
    is_first: bool,
) -> Result<()> {
    let mut packet = [0u8; 20];
    packet[0] = if is_first { 0x90 } else { 0x80 };
    packet[1] = 0xd4;
    packet[2..4].copy_from_slice(&4u16.to_be_bytes());
    let sync_rtp = sync_rtp_for(rtp_now, latency_samples);
    packet[4..8].copy_from_slice(&sync_rtp.to_be_bytes());
    packet[8..16].copy_from_slice(&ntp_time.to_be_bytes());
    packet[16..20].copy_from_slice(&rtp_now.to_be_bytes());
    if is_first {
        // #region agent log
        agent_log(
            "audio_rtp.rs:send_sync_packet",
            "audio initial sync packet",
            "H71",
            serde_json::json!({
                "currentRtp": rtp_now,
                "syncRtp": sync_rtp,
                "nextRtp": rtp_now,
                "latencySamples": latency_samples,
            }),
        );
        // #endregion
    }

    socket
        .send_to(&packet, remote)
        .await
        .map_err(|e| RottenError::Protocol(format!("audio sync send: {e}")))?;
    Ok(())
}

/// ALAC verbatim frame for stereo silence (spf samples per channel).
fn encode_alac_verbatim_silence(spf: u16) -> Vec<u8> {
    let pcm = vec![0u8; spf as usize * 2 * 2];
    let mut out = vec![0u8; pcm.len() + 64];
    let n = encode_alac_verbatim(&mut out, &pcm, spf as usize, 2, 16);
    out.truncate(n);
    out
}

struct BitWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
    bit_buf: u32,
    bit_pos: u8,
}

impl<'a> BitWriter<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self {
            buf,
            pos: 0,
            bit_buf: 0,
            bit_pos: 0,
        }
    }

    fn write(&mut self, val: u32, nbits: u32) {
        let mut v = val;
        let mut remaining = nbits;
        while remaining > 0 {
            let space = (8 - self.bit_pos) as u32;
            let take = remaining.min(space);
            self.bit_buf |= (v & ((1 << take) - 1)) << (space - take);
            v >>= take;
            remaining -= take;
            self.bit_pos += take as u8;
            if self.bit_pos == 8 {
                if self.pos < self.buf.len() {
                    self.buf[self.pos] = self.bit_buf as u8;
                }
                self.pos += 1;
                self.bit_buf = 0;
                self.bit_pos = 0;
            }
        }
    }

    fn flush(mut self) -> usize {
        if self.bit_pos > 0 && self.pos < self.buf.len() {
            self.buf[self.pos] = self.bit_buf as u8;
            self.pos += 1;
        }
        self.pos
    }
}

fn encode_alac_verbatim(
    out: &mut [u8],
    pcm: &[u8],
    frame_size: usize,
    channels: usize,
    bit_depth: u32,
) -> usize {
    let mut bw = BitWriter::new(out);
    if channels == 2 {
        bw.write(1, 3);
    } else {
        bw.write(0, 3);
    }
    bw.write(0, 4);
    bw.write(0, 12);
    bw.write(1, 1);
    bw.write(0, 2);
    bw.write(1, 1);
    bw.write(frame_size as u32, 32);

    for i in 0..frame_size * channels {
        let off = i * 2;
        let sample = if off + 1 < pcm.len() {
            u16::from_le_bytes([pcm[off], pcm[off + 1]])
        } else {
            0
        };
        bw.write(sample as u32, bit_depth);
    }
    bw.write(7, 3);
    bw.flush()
}
