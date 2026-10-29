//! Public-API tests for `vivid_protocol::messages`.
use vivid_protocol::messages::*;
use vivid_protocol::{
    VIVID_MAJOR, VIVID_MINOR,
    auth::{self, ResumeKey, Secret32},
    cbor::Value,
    registry::CORE_CONTROL,
    resource::ResourceContract,
};

use std::fmt::Write as _;
use vivid_protocol::{
    CONTROL_MAX_RECORD_BODY,
    registry::{DESKTOP_SURFACE, LIVE_MEDIA},
    wire::{ConnectionKind, encode_preface},
};

fn root_hello() -> Hello {
    Hello {
        producer_name: "test producer".into(),
        producer_version: "1.5.0".into(),
        required_profiles: vec![
            DESKTOP_SURFACE.into(),
            LIVE_MEDIA.into(),
            CORE_CONTROL.into(),
        ],
        optional_profiles: vec![],
        maximum_control_body: CONTROL_MAX_RECORD_BODY,
        client_nonce: [1; 32],
        authentication: HelloAuthentication::Root { proof: [2; 32] },
        target_profile: DESKTOP_SURFACE.into(),
        extensions: vec![(100, Value::Text("preserved".into()))],
    }
}

#[test]
fn envelope_round_trip_includes_execution_metadata() {
    let mut envelope = Envelope::correlated(9, vec![(0, Value::Unsigned(3))]).unwrap();
    envelope.transaction_id = Some(4);
    envelope.expected_target_generation = Some(7);
    envelope.preconditions = vec![(0, Value::Unsigned(8))];
    envelope.idempotency_key = Some([5; 16]);
    envelope.causation_id = Some([6; 16]);
    assert_eq!(
        decode_control(&envelope.encode().unwrap()).unwrap(),
        envelope
    );
}

#[test]
fn hello_has_no_version_range_and_preserves_extensions() {
    let hello = root_hello();
    let body = hello.encode(1).unwrap();
    let (request_id, parsed) = Hello::decode(&body).unwrap();
    assert_eq!(request_id, 1);
    assert_eq!(parsed.extensions, hello.extensions);
    assert_eq!(parsed.target_profile, DESKTOP_SURFACE);
}

#[test]
fn hello_accepts_zero_as_the_initial_resume_generation() {
    let mut hello = root_hello();
    hello.authentication = HelloAuthentication::Resume {
        context_id: 1,
        lease_id: 2,
        session_id: 3,
        resume_generation: 0,
        attempt_id: [4; 16],
        proof: [5; 32],
    };

    let (_, parsed) = Hello::decode(&hello.encode(1).unwrap()).unwrap();
    assert!(matches!(
        parsed.authentication,
        HelloAuthentication::Resume {
            resume_generation: 0,
            ..
        }
    ));
}

#[test]
fn resume_proof_verifies_only_with_the_signing_key_and_preface() {
    let mut hello = root_hello();
    hello.authentication = HelloAuthentication::Resume {
        context_id: 1,
        lease_id: 2,
        session_id: 3,
        resume_generation: 4,
        attempt_id: [5; 16],
        proof: [0; 32],
    };
    let key = ResumeKey::new([6; 32]);
    let preface = [7; 16];
    hello.authenticate_resume(&key, &preface).unwrap();
    assert!(hello.verify_resume(&key, &preface).unwrap());
    assert!(
        !hello
            .verify_resume(&ResumeKey::new([8; 32]), &preface)
            .unwrap()
    );
    assert!(!hello.verify_resume(&key, &[9; 16]).unwrap());
    if let HelloAuthentication::Resume { lease_id, .. } = &mut hello.authentication {
        *lease_id = 10;
    }
    assert!(!hello.verify_resume(&key, &preface).unwrap());
    assert!(!root_hello().verify_resume(&key, &preface).unwrap());
}

#[test]
fn hello_rejects_unclosed_profiles() {
    let mut hello = root_hello();
    hello.required_profiles = vec!["desktop-input-v1".into(), CORE_CONTROL.into()];
    assert!(hello.validate().is_err());
}

#[test]
fn hello_ignores_unknown_optional_profile_syntax() {
    let mut hello = root_hello();
    hello.optional_profiles = vec!["future-profile-v9".into()];
    hello.validate().unwrap();
}

#[test]
fn hello_root_builder_matches_transcript_verifier_and_redacts_debug() {
    let secret = Secret32::new([7; 32]);
    let preface = encode_preface(ConnectionKind::Control, CONTROL_MAX_RECORD_BODY);
    let mut hello = root_hello();
    hello.authenticate_root(&secret, &preface).unwrap();
    let authless = hello.authless_payload().unwrap();
    let HelloAuthentication::Root { proof } = &hello.authentication else {
        panic!("not root authentication");
    };
    assert!(auth::verify_root_hello_proof(
        &secret, &preface, &authless, proof
    ));
    assert!(format!("{:?}", hello.authentication).contains("[REDACTED]"));
    assert!(!format!("{:?}", hello.authentication).contains(&hex(proof)));
}

#[test]
fn welcome_round_trip_carries_all_resource_fields() {
    let welcome = Welcome {
        session_id: 1,
        session_tag: [2; 16],
        root_context_id: 3,
        target_generation: 1,
        target_profile: DESKTOP_SURFACE.into(),
        target_descriptor: vec![],
        accepted_profiles: vec![DESKTOP_SURFACE.into(), CORE_CONTROL.into()],
        maximum_control_body: CONTROL_MAX_RECORD_BODY,
        server_nonce: [4; 32],
        authentication: WelcomeAuthentication {
            kind: AUTHENTICATION_ROOT,
            confirmation: [5; 32],
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
    let body = welcome.encode(9).unwrap();
    let (request_id, parsed) = Welcome::decode(&body).unwrap();
    assert_eq!(request_id, 9);
    assert_eq!(parsed, welcome);
}

#[test]
fn lane_and_track_open_schemas_bind_complete_owner_fields() {
    let lane = LaneOpen {
        session_id: 1,
        lane_generation: 1,
        client_nonce: [2; 16],
        authentication_tag: [3; 16],
    };
    let lane_body = encode_payload(1, lane.payload()).unwrap();
    assert_eq!(LaneOpen::decode(&lane_body).unwrap().session_id, 1);

    let channel = ChannelOpen {
        session_id: 1,
        context_id: 2,
        surface_id: 3,
        track_id: 4,
        channel_generation: 1,
        track_kind: TrackKind::Video,
        lane: LaneClass::Realtime,
        client_nonce: [5; 16],
        authentication_tag: [6; 16],
    };
    let channel_body = encode_payload(1, channel.payload()).unwrap();
    let parsed = ChannelOpen::decode(4, &channel_body).unwrap();
    assert_eq!(
        (parsed.context_id, parsed.surface_id, parsed.track_id),
        (2, 3, 4)
    );
    ChannelOpen::decode(5, &channel_body).unwrap_err();
}

#[test]
fn typed_version_error_round_trip() {
    let parsed = parse_error_reply(&unsupported_version_error()).unwrap();
    assert_eq!(parsed.code, ERROR_UNSUPPORTED_VERSION);
    assert!(parsed.fatal);
    assert_eq!(
        parsed.detail.supported_version_tuple(),
        Some((u64::from(VIVID_MAJOR), u64::from(VIVID_MINOR)))
    );
}

#[test]
fn complete_identity_does_not_alias_reused_local_ids() {
    let session = vivid_protocol::identity::SessionIdentity::new(
        vivid_protocol::identity::PresenterInstanceId([1; 16]),
        1,
    )
    .unwrap();
    let first = session
        .context(10)
        .unwrap()
        .surface(20)
        .unwrap()
        .track(7)
        .unwrap();
    let second = session
        .context(11)
        .unwrap()
        .surface(20)
        .unwrap()
        .track(7)
        .unwrap();
    assert_ne!(first, second);
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}
