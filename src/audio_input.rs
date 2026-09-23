//! Portable, bounded PCM microphone packets for `audio-input-v1`.
use crate::media::{self, AudioPacket};
use crate::messages::LaneClass;
use crate::track::{
    AudioConfiguration, KindConfiguration, TrackConfiguration, TrackDirection, TrackMode,
};
use std::io;

pub const SAMPLE_RATE: u32 = 48_000;
pub const SAMPLES: usize = 960;
pub const PCM_BYTES: usize = SAMPLES * 2;
pub const BODY_BYTES: u32 = PCM_BYTES as u32 + 48;
pub const PACKET_US: u64 = 20_000;
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
            maximum_access_unit_bytes: PCM_BYTES as u32,
            codec_string: None,
        }),
        target_latency_us: 40_000,
        maximum_latency_us: 200_000,
        retained_pixel_charge: 0,
    }
}

pub fn supports(config: &TrackConfiguration) -> bool {
    config.validate(config.track_id == 0).is_ok()
        && *config == configuration(config.context_id, config.surface_id, config.track_id)
}

/// One validated, owned 20 ms packet. Device pointers never cross threads.
#[derive(Clone, Debug)]
pub struct InputPacket {
    pub epoch: u32,
    pub packet_id: u64,
    pub pts_us: i64,
    pub pcm: [u8; PCM_BYTES],
}

impl InputPacket {
    pub fn decode(body: &[u8]) -> io::Result<Self> {
        let packet = media::parse_audio_packet(body)?;
        if packet.epoch == 0
            || packet.packet_id == 0
            || packet.pts_us < 0
            || packet.dts_us != packet.pts_us
            || packet.duration_us != PACKET_US
            || packet.trim_start_samples != 0
            || packet.trim_end_samples != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid microphone packet",
            ));
        }
        let pcm = packet.data.try_into().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "microphone packet must contain 960 mono s16le samples",
            )
        })?;
        Ok(Self {
            epoch: packet.epoch,
            packet_id: packet.packet_id,
            pts_us: packet.pts_us,
            pcm,
        })
    }

    pub fn encode(&self) -> io::Result<Vec<u8>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cbor::Value;

    #[test]
    fn uplink_roundtrip_and_constraints() {
        let config = configuration(1, 2, 3);
        let payload = Value::Map(config.payload(false).unwrap());
        assert_eq!(
            TrackConfiguration::decode(3, &payload, false).unwrap(),
            config
        );
        let mut invalid = config.clone();
        invalid.slot = 2;
        assert!(invalid.validate(false).is_err());
        invalid = config.clone();
        invalid.mode = TrackMode::Timed;
        assert!(invalid.validate(false).is_err());
        invalid = config.clone();
        invalid.lane = LaneClass::Bulk;
        assert!(invalid.validate(false).is_err());
        invalid = config;
        invalid.direction = TrackDirection::Downlink;
        invalid.slot = 2;
        let payload = invalid.payload(false).unwrap();
        assert!(!payload.iter().any(|(key, _)| *key == 16));
        assert_eq!(
            TrackConfiguration::decode(3, &Value::Map(payload), false).unwrap(),
            invalid
        );
    }

    #[test]
    fn pcm_shape_is_checked_before_copying() {
        let packet = InputPacket {
            epoch: 1,
            packet_id: 1,
            pts_us: 0,
            pcm: [0; PCM_BYTES],
        };
        let body = packet.encode().unwrap();
        assert_eq!(body.len(), BODY_BYTES as usize);
        assert!(InputPacket::decode(&body[..body.len() - 2]).is_err());
        assert_eq!(InputPacket::decode(&body).unwrap().pcm, packet.pcm);
    }
}
