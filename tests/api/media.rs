//! Public-API tests for `vivid_protocol::media`.
use std::borrow::Cow;
use vivid_protocol::media::*;

#[test]
fn video_packet_uses_documented_48_byte_prefix() {
    let body = video_packet_body(VideoPacket {
        epoch: 1,
        packet_id: 9,
        pts_us: 10,
        dts_us: 8,
        duration_us: 16_667,
        key: true,
        data: &[1, 2, 3],
    })
    .unwrap();
    assert_eq!(body.len(), 51);
    assert_eq!(u32::from_be_bytes(body[0..4].try_into().unwrap()), 1);
    assert_eq!(
        u32::from_be_bytes(body[4..8].try_into().unwrap()),
        VIDEO_PACKET_KEY
    );
    assert_eq!(&body[48..], &[1, 2, 3]);
}

#[test]
fn portable_media_prefixes_remain_frozen_golden_vectors() {
    let video = video_packet_prefix(&VideoPacket {
        epoch: 0x0102_0304,
        packet_id: 0x0506_0708_090a_0b0c,
        pts_us: -2,
        dts_us: 0x1112_1314_1516_1718,
        duration_us: 0x2122_2324_2526_2728,
        key: true,
        data: &[1],
    })
    .unwrap();
    assert_eq!(
        video,
        [
            0x01, 0x02, 0x03, 0x04, 0x00, 0x00, 0x00, 0x01, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a,
            0x0b, 0x0c, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe, 0x11, 0x12, 0x13, 0x14,
            0x15, 0x16, 0x17, 0x18, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]
    );

    let audio = audio_packet_prefix(&AudioPacket {
        epoch: 0x3132_3334,
        packet_id: 0x3536_3738_393a_3b3c,
        pts_us: -3,
        dts_us: 0x4142_4344_4546_4748,
        duration_us: 0x5152_5354_5556_5758,
        trim_start_samples: 0x6162_6364,
        trim_end_samples: 0x7172_7374,
        data: &[1],
    })
    .unwrap();
    assert_eq!(
        audio,
        [
            0x31, 0x32, 0x33, 0x34, 0x00, 0x00, 0x00, 0x00, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a,
            0x3b, 0x3c, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfd, 0x41, 0x42, 0x43, 0x44,
            0x45, 0x46, 0x47, 0x48, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x61, 0x62,
            0x63, 0x64, 0x71, 0x72, 0x73, 0x74,
        ]
    );

    let raster = raster_full_frame_prefix(&RasterFrame {
        epoch: 0x0102_0304,
        frame_id: 0x0506_0708_090a_0b0c,
        width: 3,
        height: 2,
        rgba: &[0; 24],
    })
    .unwrap();
    assert_eq!(
        raster,
        [
            0x01, 0x02, 0x03, 0x04, // epoch
            0x00, 0x00, 0x00, 0x01, // FULL
            0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, // frame ID
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // base frame ID
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // PTS
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // duration
            0x00, 0x00, 0x00, 0x01, // rectangle count
            0x00, 0x00, 0x00, 0x00, // reserved
            0x00, 0x00, 0x00, 0x00, // x
            0x00, 0x00, 0x00, 0x00, // y
            0x00, 0x00, 0x00, 0x03, // width
            0x00, 0x00, 0x00, 0x02, // height
            0x00, 0x00, 0x00, 0x00, // data offset
            0x00, 0x00, 0x00, 0x18, // data length
        ]
    );
}

#[test]
fn borrowed_prefixes_match_owned_media_bodies() {
    let video = VideoPacket {
        epoch: 3,
        packet_id: 11,
        pts_us: 20,
        dts_us: 18,
        duration_us: 16_667,
        key: false,
        data: &[1, 2, 3, 4],
    };
    let video_prefix = video_packet_prefix(&video).unwrap();
    let video_body = video_packet_body(VideoPacket {
        epoch: video.epoch,
        packet_id: video.packet_id,
        pts_us: video.pts_us,
        dts_us: video.dts_us,
        duration_us: video.duration_us,
        key: video.key,
        data: video.data,
    })
    .unwrap();
    assert_eq!([video_prefix.as_slice(), video.data].concat(), video_body);

    let audio = AudioPacket {
        epoch: 4,
        packet_id: 12,
        pts_us: 30,
        dts_us: 28,
        duration_us: 20_000,
        trim_start_samples: 32,
        trim_end_samples: 16,
        data: &[5, 6, 7],
    };
    let audio_prefix = audio_packet_prefix(&audio).unwrap();
    let audio_body = audio_packet_body(AudioPacket {
        epoch: audio.epoch,
        packet_id: audio.packet_id,
        pts_us: audio.pts_us,
        dts_us: audio.dts_us,
        duration_us: audio.duration_us,
        trim_start_samples: audio.trim_start_samples,
        trim_end_samples: audio.trim_end_samples,
        data: audio.data,
    })
    .unwrap();
    assert_eq!([audio_prefix.as_slice(), audio.data].concat(), audio_body);

    let pixels = [8; 24];
    let raster_prefix = raster_full_frame_prefix(&RasterFrame {
        epoch: 5,
        frame_id: 13,
        width: 3,
        height: 2,
        rgba: &pixels,
    })
    .unwrap();
    let raster_body = raster_frame_body(&RasterFrame {
        epoch: 5,
        frame_id: 13,
        width: 3,
        height: 2,
        rgba: &pixels,
    })
    .unwrap();
    assert_eq!(
        [raster_prefix.as_slice(), pixels.as_slice()].concat(),
        raster_body
    );
    raster_full_frame_prefix(&RasterFrame {
        epoch: 5,
        frame_id: 13,
        width: 3,
        height: 2,
        rgba: &pixels[..pixels.len() - 1],
    })
    .unwrap_err();
}

#[test]
fn audio_packet_round_trip_uses_documented_prefix() {
    let body = audio_packet_body(AudioPacket {
        epoch: 2,
        packet_id: 7,
        pts_us: 12,
        dts_us: 10,
        duration_us: 20_000,
        trim_start_samples: 2112,
        trim_end_samples: 17,
        data: &[4, 5, 6],
    })
    .unwrap();
    let parsed = parse_audio_packet(&body).unwrap();
    assert_eq!(parsed.epoch, 2);
    assert_eq!(parsed.packet_id, 7);
    assert_eq!(parsed.trim_start_samples, 2112);
    assert_eq!(parsed.trim_end_samples, 17);
    assert_eq!(parsed.data, &[4, 5, 6]);
    assert_eq!(audio_body_len(3).unwrap(), 51);
}

#[test]
fn audio_packets_reject_malformed_headers_and_reused_ids() {
    parse_audio_packet(&[0; 47]).unwrap_err();
    parse_audio_packet(&[0; 48]).unwrap_err();
    let mut body = audio_packet_body(AudioPacket {
        epoch: 1,
        packet_id: 1,
        pts_us: 0,
        dts_us: 0,
        duration_us: 1_000,
        trim_start_samples: 0,
        trim_end_samples: 0,
        data: &[1],
    })
    .unwrap();
    body[7] = 1;
    parse_audio_packet(&body).unwrap_err();

    let mut sequence = MediaSequence::default();
    sequence.accept(1, 1).unwrap();
    assert!(sequence.accept(1, 1).is_err());
    assert!(sequence.accept(2, 0).is_err());
}

#[test]
fn full_raster_contains_one_rectangle() {
    let body = raster_frame_body(&RasterFrame {
        epoch: 1,
        frame_id: 2,
        width: 2,
        height: 1,
        rgba: &[0; 8],
    })
    .unwrap();
    assert_eq!(body.len(), 48 + 24 + 8);
    assert_eq!(u32::from_be_bytes(body[40..44].try_into().unwrap()), 1);
    assert_eq!(&body[72..], &[0; 8]);
    let parsed = parse_full_raster_frame(&body).unwrap();
    assert_eq!((parsed.width, parsed.height), (2, 1));
    assert_eq!(parsed.pixels, &[0; 8]);
}

fn mixed_delta_body(compress: bool) -> Vec<u8> {
    raster_delta_frame_body(
        &RasterDeltaFrame {
            epoch: 3,
            frame_id: 9,
            base_frame_id: 8,
            pts_us: -10,
            duration_us: 16_667,
            width: 4,
            height: 3,
            operations: &[
                RasterDeltaOperation::Copy {
                    destination_x: 0,
                    destination_y: 1,
                    width: 4,
                    height: 2,
                    source_x: 0,
                    source_y: 0,
                },
                RasterDeltaOperation::Overwrite {
                    x: 0,
                    y: 0,
                    width: 4,
                    height: 1,
                    rgba: &[7; 16],
                },
            ],
        },
        4,
        compress,
    )
    .unwrap()
}

#[test]
fn raster_delta_round_trip_preserves_order_without_payload_offsets() {
    let body = mixed_delta_body(false);
    assert_eq!(body.len(), 48 + 2 * 32 + 16);
    assert_eq!(
        u32::from_be_bytes(body[4..8].try_into().unwrap()),
        RASTER_FRAME_DELTA
    );
    assert_eq!(u64::from_be_bytes(body[16..24].try_into().unwrap()), 8);
    assert_eq!(u32::from_be_bytes(body[40..44].try_into().unwrap()), 2);
    assert_eq!(
        u32::from_be_bytes(body[48 + 28..48 + 32].try_into().unwrap()),
        0
    );
    assert_eq!(
        u32::from_be_bytes(body[80 + 28..80 + 32].try_into().unwrap()),
        16
    );
    assert_eq!(&body[112..], &[7; 16]);

    let parsed = parse_delta_raster_frame(
        &body,
        RasterSize {
            width: 4,
            height: 3,
        },
        4,
    )
    .unwrap();
    assert_eq!(
        (parsed.epoch, parsed.frame_id, parsed.base_frame_id),
        (3, 9, 8)
    );
    assert_eq!((parsed.pts_us, parsed.duration_us), (-10, 16_667));
    assert!(!parsed.compressed);
    assert_eq!(
        parsed.operations[0],
        ParsedRasterDeltaOperation::Copy {
            destination_x: 0,
            destination_y: 1,
            width: 4,
            height: 2,
            source_x: 0,
            source_y: 0,
        }
    );
    assert_eq!(
        parsed.operations[1],
        ParsedRasterDeltaOperation::Overwrite {
            x: 0,
            y: 0,
            width: 4,
            height: 1,
            rgba: Cow::Borrowed(&[7; 16]),
        }
    );
}

#[test]
#[cfg(feature = "native")]
fn raster_delta_zstd_validates_and_decodes_each_overwrite_frame() {
    let body = mixed_delta_body(true);
    let parsed = parse_delta_raster_frame(
        &body,
        RasterSize {
            width: 4,
            height: 3,
        },
        2,
    )
    .unwrap();
    assert!(parsed.compressed);
    match &parsed.operations[1] {
        ParsedRasterDeltaOperation::Overwrite { rgba, .. } => {
            assert_eq!(rgba.as_ref(), &[7; 16]);
            assert!(matches!(rgba, Cow::Owned(_)));
        }
        _ => panic!("second operation should be an overwrite"),
    }

    let mut concatenated = body;
    let second = zstd::bulk::compress(&[7; 16], 1).unwrap();
    concatenated.extend_from_slice(&second);
    let length = u32::from_be_bytes(concatenated[108..112].try_into().unwrap())
        + u32::try_from(second.len()).unwrap();
    concatenated[108..112].copy_from_slice(&length.to_be_bytes());
    parse_delta_raster_frame(
        &concatenated,
        RasterSize {
            width: 4,
            height: 3,
        },
        2,
    )
    .unwrap_err();
}

#[test]
fn raster_delta_rejects_invalid_header_and_operation_counts() {
    let baseline = mixed_delta_body(false);
    let mut malformed = baseline.clone();
    malformed[4..8].copy_from_slice(&(RASTER_FRAME_FULL | RASTER_FRAME_DELTA).to_be_bytes());
    parse_delta_raster_frame(
        &malformed,
        RasterSize {
            width: 4,
            height: 3,
        },
        4,
    )
    .unwrap_err();

    let mut malformed = baseline.clone();
    malformed[16..24].copy_from_slice(&0_u64.to_be_bytes());
    parse_delta_raster_frame(
        &malformed,
        RasterSize {
            width: 4,
            height: 3,
        },
        4,
    )
    .unwrap_err();

    let mut malformed = baseline.clone();
    malformed[44..48].copy_from_slice(&1_u32.to_be_bytes());
    parse_delta_raster_frame(
        &malformed,
        RasterSize {
            width: 4,
            height: 3,
        },
        4,
    )
    .unwrap_err();

    let mut malformed = baseline.clone();
    malformed[40..44].copy_from_slice(&0_u32.to_be_bytes());
    parse_delta_raster_frame(
        &malformed,
        RasterSize {
            width: 4,
            height: 3,
        },
        4,
    )
    .unwrap_err();
    parse_delta_raster_frame(
        &baseline,
        RasterSize {
            width: 4,
            height: 3,
        },
        1,
    )
    .unwrap_err();
    parse_delta_raster_frame(
        &baseline,
        RasterSize {
            width: 4,
            height: 3,
        },
        17,
    )
    .unwrap_err();
    parse_delta_raster_frame(
        &baseline[..79],
        RasterSize {
            width: 4,
            height: 3,
        },
        4,
    )
    .unwrap_err();
}

#[test]
fn raster_delta_rejects_every_malformed_rectangle_form() {
    let baseline = mixed_delta_body(false);
    for (offset, value) in [
        (48, 3_u32),    // unknown copy operation kind
        (60, 0),        // zero destination width
        (56, u32::MAX), // overflowing destination Y
        (52, u32::MAX), // overflowing destination X
        (68, u32::MAX), // overflowing copy source X
        (72, u32::MAX), // overflowing copy source Y
    ] {
        let mut malformed = baseline.clone();
        malformed[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        assert!(
            parse_delta_raster_frame(
                &malformed,
                RasterSize {
                    width: 4,
                    height: 3
                },
                4
            )
            .is_err(),
            "field at offset {offset} unexpectedly passed"
        );
    }

    let mut overwrite_reserved = baseline.clone();
    overwrite_reserved[100..104].copy_from_slice(&1_u32.to_be_bytes());
    parse_delta_raster_frame(
        &overwrite_reserved,
        RasterSize {
            width: 4,
            height: 3,
        },
        4,
    )
    .unwrap_err();

    let mut copy_payload = baseline.clone();
    copy_payload[76..80].copy_from_slice(&1_u32.to_be_bytes());
    parse_delta_raster_frame(
        &copy_payload,
        RasterSize {
            width: 4,
            height: 3,
        },
        4,
    )
    .unwrap_err();
}

#[test]
fn raster_delta_rejects_payload_length_mismatch_and_trailing_bytes() {
    let baseline = mixed_delta_body(false);
    for length in [15_u32, 17, u32::MAX] {
        let mut malformed = baseline.clone();
        malformed[108..112].copy_from_slice(&length.to_be_bytes());
        parse_delta_raster_frame(
            &malformed,
            RasterSize {
                width: 4,
                height: 3,
            },
            4,
        )
        .unwrap_err();
    }
    let mut trailing = baseline;
    trailing.push(0);
    parse_delta_raster_frame(
        &trailing,
        RasterSize {
            width: 4,
            height: 3,
        },
        4,
    )
    .unwrap_err();
}

#[test]
fn raster_delta_builder_rejects_invalid_inputs_before_encoding() {
    let overwrite = RasterDeltaOperation::Overwrite {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        rgba: &[0; 4],
    };
    for (frame, base, width, height, limit) in [
        (0, 1, 1, 1, 1),
        (1, 0, 1, 1, 1),
        (1, 1, 0, 1, 1),
        (1, 1, 1, 0, 1),
        (1, 1, 1, 1, 0),
        (1, 1, 1, 1, 17),
    ] {
        raster_delta_frame_body(
            &RasterDeltaFrame {
                epoch: 0,
                frame_id: frame,
                base_frame_id: base,
                pts_us: 0,
                duration_us: 0,
                width,
                height,
                operations: &[overwrite],
            },
            limit,
            false,
        )
        .unwrap_err();
    }
    raster_delta_frame_body(
        &RasterDeltaFrame {
            epoch: 0,
            frame_id: 1,
            base_frame_id: 1,
            pts_us: 0,
            duration_us: 0,
            width: 1,
            height: 1,
            operations: &[],
        },
        1,
        false,
    )
    .unwrap_err();
    let wrong_length = RasterDeltaOperation::Overwrite {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        rgba: &[0; 3],
    };
    raster_delta_frame_body(
        &RasterDeltaFrame {
            epoch: 0,
            frame_id: 1,
            base_frame_id: 1,
            pts_us: 0,
            duration_us: 0,
            width: 1,
            height: 1,
            operations: &[wrong_length],
        },
        1,
        false,
    )
    .unwrap_err();
}

#[test]
fn raster_body_limits_are_exact() {
    rgba8_raw_frame_body_len(4095, 4095).unwrap();
    assert_eq!(
        rgba8_raw_frame_body_len(4096, 4096),
        Err(SizeError::TooLarge)
    );
    rgba8_raw_frame_body_len(8192, 1).unwrap();
}

#[test]
#[cfg(feature = "native")]
fn zstd_raster_round_trip() {
    let pixels = vec![7; 64];
    let body = raster_frame_body_with_compression(
        &RasterFrame {
            epoch: 1,
            frame_id: 1,
            width: 4,
            height: 4,
            rgba: &pixels,
        },
        true,
    )
    .unwrap();
    let parsed = parse_full_raster_frame(&body).unwrap();
    assert!(parsed.compressed);
    assert_eq!(decode_raster_pixels(parsed).unwrap(), pixels);
}

#[test]
#[cfg(feature = "native")]
fn zstd_rejects_concatenated_frames() {
    let pixels = vec![7; 64];
    let mut body = raster_frame_body_with_compression(
        &RasterFrame {
            epoch: 1,
            frame_id: 1,
            width: 4,
            height: 4,
            rgba: &pixels,
        },
        true,
    )
    .unwrap();
    body.extend_from_slice(&zstd::bulk::compress(&pixels, 1).unwrap());
    let compressed_length = u32::try_from(body.len() - 72).unwrap();
    body[68..72].copy_from_slice(&compressed_length.to_be_bytes());
    let parsed = parse_full_raster_frame(&body).unwrap();
    decode_raster_pixels(parsed).unwrap_err();
}

#[test]
#[cfg(feature = "native")]
fn zstd_rejects_legacy_and_skippable_frames_before_decoding() {
    let pixels = vec![7; 64];
    let body = raster_frame_body_with_compression(
        &RasterFrame {
            epoch: 1,
            frame_id: 1,
            width: 4,
            height: 4,
            rgba: &pixels,
        },
        true,
    )
    .unwrap();
    let standard = parse_full_raster_frame(&body).unwrap();
    assert_eq!(decode_raster_pixels(standard).unwrap(), pixels);

    // zstd 0.7 legacy magic (0xFD2FB527) and a skippable frame (0x184D2A50), each followed by
    // the bytes of an otherwise valid frame body.
    for magic in [0xfd2f_b527_u32, 0x184d_2a50] {
        let mut forged = body.clone();
        forged[72..76].copy_from_slice(&magic.to_le_bytes());
        let parsed = parse_full_raster_frame(&forged).unwrap();
        let error = decode_raster_pixels(parsed).unwrap_err();
        assert!(error.is_malformed(), "{error}");
    }
}

#[test]
fn rejects_ambiguous_or_reserved_media_flags() {
    let mut packet = video_packet_body(VideoPacket {
        epoch: 1,
        packet_id: 1,
        pts_us: 0,
        dts_us: 0,
        duration_us: 0,
        key: true,
        data: &[1],
    })
    .unwrap();
    packet[4..8].copy_from_slice(&(VIDEO_PACKET_KEY | VIDEO_PACKET_DELTA).to_be_bytes());
    parse_video_packet(&packet).unwrap_err();

    let mut raster = raster_frame_body(&RasterFrame {
        epoch: 1,
        frame_id: 1,
        width: 1,
        height: 1,
        rgba: &[0; 4],
    })
    .unwrap();
    raster[4..8].copy_from_slice(&(RASTER_FRAME_FULL | 4).to_be_bytes());
    parse_full_raster_frame(&raster).unwrap_err();
}

#[test]
fn portable_key_status_comes_from_codec_syntax() {
    assert!(access_unit_is_key("h264", &[0, 0, 0, 1, 0x65]).unwrap());
    assert!(!access_unit_is_key("h264", &[0, 0, 1, 0x41]).unwrap());
    assert!(access_unit_is_key("hevc", &[0, 0, 1, 19 << 1]).unwrap());
    assert!(access_unit_is_key("vp9", &[0x80]).unwrap());
    assert!(!access_unit_is_key("vp9", &[0x84]).unwrap());
    assert!(access_unit_is_key("av1", &[0x32, 0x01, 0x00]).unwrap());
    assert!(!access_unit_is_key("av1", &[0x32, 0x01, 0x20]).unwrap());
}
