//! Regressions for the independently verified 2026-09-07 protocol audit.
use vivid_protocol::{
    anchor::{self, AnchorKey},
    auth::Secret32,
    cbor::{self, Value},
    media,
    messages::{
        self, ErrorDetail, ErrorReply, Hello, HelloAuthentication, Welcome, WelcomeAuthentication,
    },
    registry::{self, CORE_CONTROL, DESKTOP_SURFACE},
    resource::ResourceContract,
};

fn hello(authentication: HelloAuthentication) -> Hello {
    Hello {
        producer_name: "private-producer-title".into(),
        producer_version: "1".into(),
        required_profiles: vec![DESKTOP_SURFACE.into(), CORE_CONTROL.into()],
        optional_profiles: vec![],
        maximum_control_body: 1024,
        client_nonce: [1; 32],
        authentication,
        target_profile: DESKTOP_SURFACE.into(),
        extensions: vec![],
    }
}

fn activation() -> HelloAuthentication {
    HelloAuthentication::LeaseActivation {
        context_id: 1,
        lease_id: 1,
        activation_secret: Secret32::new([0xab; 32]),
        attempt_id: [2; 16],
        proof_of_possession: None,
    }
}

#[test]
fn error_detail_checks_constructor_and_mutated_public_fields() {
    for key in 0..=18 {
        let good = if key == 10 {
            Value::Bool(true)
        } else {
            Value::Unsigned(0)
        };
        ErrorDetail::new(vec![(key, good)]).unwrap();
        for bad in [
            Value::Text("sensitive".into()),
            Value::Bytes(vec![7; 32]),
            Value::Null,
        ] {
            assert!(ErrorDetail::new(vec![(key, bad.clone())]).is_err());
            let reply = ErrorReply {
                code: 1,
                request_id: 1,
                detail: ErrorDetail {
                    fields: vec![(key, bad.clone())],
                },
                fatal: false,
                diagnostic: String::new(),
            };
            assert!(reply.encode().is_err());
            let body = messages::encode_payload(
                1,
                vec![
                    (0, Value::Unsigned(1)),
                    (1, Value::Unsigned(1)),
                    (2, Value::Map(vec![(key, bad)])),
                    (3, Value::Bool(false)),
                    (4, Value::Text(String::new())),
                ],
            )
            .unwrap();
            assert!(messages::parse_error_reply(&body).is_err());
        }
    }
    assert!(ErrorDetail::new(vec![(10, Value::Unsigned(1))]).is_err());
    assert!(ErrorDetail::new(vec![(0, Value::Bool(false))]).is_err());
    for value in 0..=3 {
        assert_eq!(
            ErrorDetail::new(vec![(12, Value::Unsigned(value))]).is_ok(),
            value <= 2
        );
    }
    for fields in [
        vec![(19, Value::Unsigned(0))],
        vec![(0, Value::Text("x".repeat(4097)))],
        vec![(1, Value::Unsigned(0)), (0, Value::Unsigned(0))],
        vec![(0, Value::Unsigned(0)), (0, Value::Unsigned(0))],
    ] {
        assert!(ErrorDetail::new(fields).is_err());
    }
    for code in [0, 1, 30, 31, u64::MAX] {
        let reply = ErrorReply {
            code,
            request_id: 1,
            detail: ErrorDetail::new(vec![]).unwrap(),
            fatal: false,
            diagnostic: String::new(),
        };
        assert_eq!(reply.encode().is_ok(), registry::error::is_registered(code));
        let body = messages::encode_payload(
            1,
            vec![
                (0, Value::Unsigned(code)),
                (1, Value::Unsigned(1)),
                (2, Value::Map(vec![])),
                (3, Value::Bool(false)),
                (4, Value::Text(String::new())),
            ],
        )
        .unwrap();
        assert_eq!(
            messages::parse_error_reply(&body).is_ok(),
            registry::error::is_registered(code)
        );
    }
}

#[test]
fn registered_error_membership_matches_the_normative_registry() {
    let registry = include_str!("../vivid-protocol-1.5-registry.toml");
    let mut assigned = Vec::new();
    for section in registry.split("[[error]]").skip(1) {
        let section = section.split("[[").next().unwrap();
        let code = section
            .lines()
            .find_map(|line| {
                line.strip_prefix("code = ")
                    .or_else(|| line.strip_prefix("value = "))
            })
            .unwrap();
        assigned.push(code.parse::<u64>().unwrap());
    }
    assert!(!assigned.is_empty());
    for code in 0..=assigned.iter().copied().max().unwrap() + 1 {
        assert_eq!(
            registry::error::is_registered(code),
            assigned.contains(&code),
            "code {code}"
        );
    }
}

#[test]
fn welcome_authentication_is_checked_on_both_boundaries() {
    let mut welcome = Welcome {
        session_id: 1,
        session_tag: [0; 16],
        root_context_id: 1,
        target_generation: 1,
        target_profile: DESKTOP_SURFACE.into(),
        target_descriptor: vec![],
        accepted_profiles: vec![DESKTOP_SURFACE.into(), CORE_CONTROL.into()],
        maximum_control_body: 1024,
        server_nonce: [0; 32],
        authentication: WelcomeAuthentication {
            kind: 0,
            confirmation: [0; 32],
            lease_state: 0,
            activation_attempt_status: 0,
        },
        session_revision: 1,
        scene_revision: 0,
        resource_contract: ResourceContract::denied(),
        establishment_state: 0,
        resume_generation: 0,
        extensions: vec![],
    };
    for kind in 0..=2 {
        welcome.authentication.kind = kind;
        welcome.authentication.lease_state = if kind == 0 { 0 } else { 3 };
        for status in 0..=1 {
            welcome.authentication.activation_attempt_status = status;
            let body = welcome.encode(1).unwrap();
            assert_eq!(Welcome::decode(&body).unwrap().1, welcome);
            for (key, bad) in [(0, 99), (2, 99), (3, 99)] {
                let mut envelope = messages::decode_control(&body).unwrap();
                let Value::Map(fields) = &mut envelope
                    .payload
                    .iter_mut()
                    .find(|(key, _)| *key == 9)
                    .unwrap()
                    .1
                else {
                    panic!()
                };
                fields
                    .iter_mut()
                    .find(|(field, _)| *field == key)
                    .unwrap()
                    .1 = Value::Unsigned(bad);
                assert!(Welcome::decode(&envelope.encode().unwrap()).is_err());
            }
        }
    }
    welcome.authentication.kind = 99;
    assert!(welcome.encode(1).is_err());
    welcome.authentication.kind = 1;
    welcome.authentication.activation_attempt_status = 99;
    assert!(welcome.encode(1).is_err());
    welcome.authentication.activation_attempt_status = 0;
    welcome.authentication.lease_state = 0;
    assert!(welcome.encode(1).is_err());
}

#[test]
fn authentication_errors_are_bounded_and_preserve_valid_transcripts() {
    let hello = hello(activation());
    let body = hello.encode(1).unwrap();
    assert_eq!(
        Hello::decode(&body).unwrap().1.authless_payload().unwrap(),
        hello.authless_payload().unwrap()
    );
    assert!(hello.encode(0).is_err());
    for key in [0, 5, 6, 7] {
        let mut envelope = messages::decode_control(&body).unwrap();
        envelope
            .payload
            .iter_mut()
            .find(|(field, _)| *field == key)
            .unwrap()
            .1 = Value::Null;
        assert!(Hello::decode(&envelope.encode().unwrap()).is_err());
    }
    let mut envelope = messages::decode_control(&body).unwrap();
    envelope.request_id = 0;
    assert!(Hello::decode(&envelope.encode().unwrap()).is_err());
    for end in 0..body.len() {
        assert!(Hello::decode(&body[..end]).is_err());
    }
    let mut invalid = body;
    invalid.push(0);
    assert!(Hello::decode(&invalid).is_err());
    let value = Value::Array(vec![
        Value::Bytes(vec![0xab; 32]),
        Value::Text("sensitive".into()),
    ]);
    let mut bytes = Vec::new();
    let mut invalid = value;
    if let Value::Array(ref mut values) = invalid {
        values.push(Value::Map(vec![(1, Value::Null), (0, Value::Null)]));
    }
    assert!(cbor::encode_into(&mut bytes, &invalid).is_err());
    assert!(
        bytes.is_empty(),
        "an encode failure must wipe its partial output"
    );
}

#[test]
fn marker_ids_must_be_hex_digits() {
    let key = AnchorKey::new([7; 32]);
    let canonical = anchor::encode_marker(&key, &[1; 16], 1, 1).unwrap();
    let body = &canonical[2..canonical.len() - 2];
    for prefix in ['+', '-', ' ', 'g'] {
        let malformed = body.replace("0000000000000001", &format!("{prefix}000000000000001"));
        assert!(anchor::parse_marker(&malformed).is_err());
        assert!(anchor::parse_conpty_marker(&format!("{malformed};VIVID-END")).is_err());
    }
    let valid = anchor::encode_marker(&key, &[1; 16], 0xab, 0xcd).unwrap();
    let upper = valid[2..valid.len() - 2]
        .replace("00000000000000ab", "00000000000000AB")
        .replace("00000000000000cd", "00000000000000CD");
    assert!(anchor::verify_marker(
        &key,
        &anchor::parse_marker(&upper).unwrap()
    ));
}

#[test]
fn malformed_lengths_do_not_panic_on_32_bit_parsers() {
    for count in 1..=8 {
        let mut obu = vec![0x12];
        obu.extend(std::iter::repeat_n(0x80, count));
        assert!(media::access_unit_is_key("av1", &obu).is_err());
        *obu.last_mut().unwrap() = 0x7f;
        assert!(media::access_unit_is_key("av1", &obu).is_err());
    }
    let mut body = [0; 72];
    for (offset, value) in [(4, 3u32), (40, 1), (56, 1), (60, 1)] {
        body[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }
    for length in [0, 1, u32::MAX - 72, u32::MAX - 71, u32::MAX] {
        body[68..72].copy_from_slice(&length.to_be_bytes());
        // Zero-length compressed data is parsed structurally; decoding rejects it.
        if length != 0 {
            assert!(media::parse_full_raster_frame(&body).is_err());
        }
    }
}

#[test]
fn aac_layout_numbers_are_not_channel_counts() {
    let counts = [
        None,
        Some(1),
        Some(2),
        Some(3),
        Some(4),
        Some(5),
        Some(6),
        Some(8),
        None,
        None,
        None,
        Some(7),
        Some(8),
        Some(24),
        Some(8),
        None,
    ];
    for (config, count) in counts.iter().enumerate().skip(1) {
        let asc = [0x12, (config as u8) << 3];
        for channels in 1..=24 {
            assert_eq!(
                media::validate_aac_audio_specific_config(&asc, 44100, channels).is_ok(),
                *count == Some(channels),
                "configuration {config}, channels {channels}"
            );
        }
    }
    // AAC-LC 44.1 kHz, config 0, GA flags 0, PCE with one front CPE and no comments.
    let mut bits = String::from("0001001000000000");
    bits.push_str("0000"); // instance tag
    bits.push_str("01"); // LC
    bits.push_str("0100"); // frequency index
    bits.push_str("000100000000000000000"); // front=1, side/back/lfe/assoc/cc=0
    bits.push_str("000"); // no mixdowns
    bits.push_str("10000"); // one channel pair, tag zero
    while bits.len() % 8 != 0 {
        bits.push('0');
    }
    bits.push_str("00000000");
    let pce = bits
        .as_bytes()
        .chunks(8)
        .map(|chunk| u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 2).unwrap())
        .collect::<Vec<_>>();
    media::validate_aac_audio_specific_config(&pce, 44100, 2).unwrap();
    for object_type in [6, 20] {
        let mut scalable = pce.clone();
        scalable[0] = (object_type << 3) | (scalable[0] & 7);
        scalable.push(0); // layerNr follows the byte-aligned PCE, not the GA flags.
        assert!(media::validate_aac_audio_specific_config(&scalable, 44100, 2).is_ok());
    }
    assert!(media::validate_aac_audio_specific_config(&pce, 44100, 1).is_err());
    for end in 0..pce.len() {
        assert!(media::validate_aac_audio_specific_config(&pce[..end], 44100, 2).is_err());
    }
}

#[cfg(any(feature = "native", feature = "native-transport"))]
#[test]
fn connection_trace_persists_only_metadata_and_rejects_existing_paths() {
    use std::io::{self, IoSlice, Write};
    use vivid_protocol::wire::{Connection, ConnectionKind};
    let path = std::env::temp_dir().join(format!(
        "vivid-audit-trace-{}-{}.ndjson",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut trace = Connection::trace(&path, ConnectionKind::Control).unwrap();
    for auth in [
        HelloAuthentication::Root { proof: [0xcd; 32] },
        activation(),
        HelloAuthentication::Resume {
            context_id: 1,
            lease_id: 1,
            session_id: 1,
            resume_generation: 0,
            attempt_id: [2; 16],
            proof: [0xef; 32],
        },
    ] {
        trace
            .write_record(messages::HELLO, 0, 0, &hello(auth).encode(1).unwrap())
            .unwrap();
    }
    for kind in [
        messages::WELCOME,
        messages::LANE_OPEN,
        messages::CHANNEL_OPEN,
        messages::FILE_TRANSFER_OPEN,
        0xffff,
    ] {
        trace
            .write_record_parts(kind, 0, 0, &[b"private", &[0xab; 32], b"body"])
            .unwrap();
    }
    trace.writer().shutdown().unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 8);
    assert!(text.lines().all(|line| line.starts_with("{\"version\":1,")));
    assert!(!text.contains("private"));
    assert!(!text.as_bytes().windows(32).any(|w| w == [0xab; 32]));
    assert!(Connection::trace(&path, ConnectionKind::Control).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    std::fs::remove_file(path).unwrap();
    struct OverReporting;
    impl Write for OverReporting {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
        fn write_vectored(&mut self, b: &[IoSlice<'_>]) -> io::Result<usize> {
            Ok(b.iter().map(|s| s.len()).sum::<usize>() + 1)
        }
    }
    let mut connection = Connection::from_streams(
        Box::new(io::empty()),
        Box::new(OverReporting),
        ConnectionKind::Control,
    )
    .unwrap();
    // More than the offered batch, but less than all remaining parts.
    assert!(
        connection
            .write_record_parts(messages::HELLO, 0, 0, &[&[7][..]; 32])
            .is_err()
    );
}

#[test]
fn owned_secret_tree_can_be_scrubbed_before_release() {
    use zeroize::Zeroize;
    let mut envelope = messages::Envelope::new(
        1,
        vec![(
            0,
            Value::Map(vec![
                (0, Value::Bytes(vec![0xab; 32])),
                (
                    1,
                    Value::Array(vec![Value::Text("temporary-secret".into())]),
                ),
            ]),
        )],
    );
    envelope.zeroize();
    assert!(envelope.payload.is_empty());
    assert_eq!(envelope.request_id, 0);
    // Inspect a live allocation; do not read freed memory to infer Drop behavior.
    let mut value = Value::Bytes(vec![0xab; 32]);
    value.zeroize();
    assert_eq!(value, Value::Bytes(Vec::new()));
}
