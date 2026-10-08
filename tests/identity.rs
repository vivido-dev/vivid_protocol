//! Owner-scoped identities reject zero IDs and expose their complete tuple.
use vivid_protocol::{
    identity::{PresenterInstanceId, SessionIdentity},
    revision::ChannelGeneration,
};

const PRESENTER: PresenterInstanceId = PresenterInstanceId([1; 16]);

#[test]
fn every_level_rejects_a_zero_id() {
    SessionIdentity::new(PRESENTER, 0).unwrap_err();
    let session = SessionIdentity::new(PRESENTER, 1).unwrap();
    session.context(0).unwrap_err();
    let context = session.context(2).unwrap();
    context.surface(0).unwrap_err();
    context.node(0).unwrap_err();
    context.transaction(0).unwrap_err();
    context.anchor(0).unwrap_err();
    context.lease(0).unwrap_err();
    let surface = context.surface(3).unwrap();
    surface.track(0).unwrap_err();
    let track = surface.track(4).unwrap();
    track.channel(ChannelGeneration::ZERO).unwrap_err();
}

#[test]
fn accessors_return_the_complete_owner_tuple() {
    let session = SessionIdentity::new(PRESENTER, 1).unwrap();
    let context = session.context(2).unwrap();
    let channel = context
        .surface(3)
        .unwrap()
        .track(4)
        .unwrap()
        .channel(ChannelGeneration::ONE)
        .unwrap();

    let track = channel.track();
    assert_eq!(channel.generation(), ChannelGeneration::ONE);
    assert_eq!(track.track_id(), 4);
    assert_eq!(track.surface().surface_id(), 3);
    assert_eq!(track.surface().context(), context);
    assert_eq!(context.context_id(), 2);
    assert_eq!(context.session(), session);
    assert_eq!(session.session_id(), 1);
    assert_eq!(session.presenter(), PRESENTER);

    let node = context.node(5).unwrap();
    assert_eq!((node.context(), node.node_id()), (context, 5));
    let transaction = context.transaction(6).unwrap();
    assert_eq!(
        (transaction.context(), transaction.transaction_id()),
        (context, 6)
    );
    let anchor = context.anchor(7).unwrap();
    assert_eq!((anchor.context(), anchor.anchor_id()), (context, 7));
    let lease = context.lease(8).unwrap();
    assert_eq!((lease.context(), lease.lease_id()), (context, 8));
}
