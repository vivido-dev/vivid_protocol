//! Portable, bounded PCM microphone packets for `audio-input-v1` (media §17).
//!
//! The uplink track has one canonical configuration, returned by [`configuration`]: 48 kHz mono
//! 16-bit PCM in 20 ms packets, so the receiver needs no codec. [`supports`] checks that a
//! requested track matches it exactly, and [`InputPacket`] holds one validated packet.
use crate::CodecError;
use crate::media::{self, AudioPacket};
use crate::messages::LaneClass;
use crate::track::{
    AudioConfiguration, KindConfiguration, TrackConfiguration, TrackDirection, TrackMode,
};

/// Microphone sample rate in hertz.
pub const SAMPLE_RATE: u32 = 48_000;
/// Samples in one 20 ms packet.
pub const SAMPLES: usize = 960;
/// Bytes of mono 16-bit PCM in one packet.
pub const PCM_BYTES: usize = SAMPLES * 2;
/// `AUDIO_PACKET` body bytes: the 48-byte prefix and one packet of PCM.
pub const BODY_BYTES: u32 = crate::const_u32(PCM_BYTES) + 48;
/// Duration of one packet in microseconds.
pub const PACKET_US: u64 = 20_000;
/// Packets the uplink may have in flight.
pub const QUEUE_PACKETS: usize = 10;

/// Canonical microphone configuration, requiring no receiver codec.
pub fn configuration(context_id: u64, surface_id: u64, track_id: u64) -> TrackConfiguration {
    TrackConfiguration {
        direction: TrackDirection::Uplink,
        context_id,
        surface_id,
        track_id,
        slot: 0,
        mode: TrackMode::Live,
        lane: LaneClass::Realtime,
        maximum_record_body: BODY_BYTES,
        maximum_rate_millihertz: 50_000,
        maximum_encoded_bits_per_second: u64::from(BODY_BYTES) * 8 * 50,
        maximum_records_per_second: 50,
        maximum_inflight_body_bytes: u64::from(BODY_BYTES) * QUEUE_PACKETS as u64,
        kind: KindConfiguration::Audio(AudioConfiguration {
            codec: "pcm_s16le".into(),
            packetization: "pcm-packet-v1".into(),
            extradata: Vec::new(),
            sample_rate: SAMPLE_RATE,
            channels: 1,
            channel_mask: 4,
            maximum_access_unit_bytes: const { crate::const_u32(PCM_BYTES) },
            codec_string: None,
        }),
        target_latency_us: 40_000,
        maximum_latency_us: 200_000,
        retained_pixel_charge: 0,
    }
}

/// Returns whether `config` is exactly the canonical microphone configuration.
pub fn supports(config: &TrackConfiguration) -> bool {
    config.validate(config.track_id == 0).is_ok()
        && *config == configuration(config.context_id, config.surface_id, config.track_id)
}

/// One validated, owned 20 ms packet. Device pointers never cross threads.
#[derive(Clone, Debug)]
pub struct InputPacket {
    /// Media epoch.
    pub epoch: u32,
    /// Nonzero packet ID; IDs strictly increase.
    pub packet_id: u64,
    /// Capture time in signed microseconds.
    pub pts_us: i64,
    /// One packet of mono 16-bit little-endian PCM.
    pub pcm: [u8; PCM_BYTES],
}

impl InputPacket {
    /// Decodes a microphone packet body.
    ///
    /// # Errors
    ///
    /// Returns a malformed-input [`CodecError`] when `body` is not a valid audio packet, or its
    /// epoch, packet ID, timestamps, duration, trim, or PCM length do not match the fixed
    /// microphone format.
    pub fn decode(body: &[u8]) -> Result<Self, CodecError> {
        let packet = media::parse_audio_packet(body)?;
        if packet.epoch == 0
            || packet.packet_id == 0
            || packet.pts_us < 0
            || packet.dts_us != packet.pts_us
            || packet.duration_us != PACKET_US
            || packet.trim_start_samples != 0
            || packet.trim_end_samples != 0
        {
            return Err(CodecError::malformed("invalid microphone packet"));
        }
        let pcm = packet.data.try_into().map_err(|_| {
            CodecError::malformed("microphone packet must contain 960 mono s16le samples")
        })?;
        Ok(Self {
            epoch: packet.epoch,
            packet_id: packet.packet_id,
            pts_us: packet.pts_us,
            pcm,
        })
    }

    /// Encodes this microphone packet body.
    ///
    /// # Errors
    ///
    /// Returns a malformed-input [`CodecError`] when the packet does not match the fixed microphone
    /// format, and an invalid-argument [`CodecError`] when the body would be too large.
    pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        let body = media::audio_packet_body(AudioPacket {
            epoch: self.epoch,
            packet_id: self.packet_id,
            pts_us: self.pts_us,
            dts_us: self.pts_us,
            duration_us: PACKET_US,
            trim_start_samples: 0,
            trim_end_samples: 0,
            data: &self.pcm,
        })?;
        Self::decode(&body)?;
        Ok(body)
    }
}
