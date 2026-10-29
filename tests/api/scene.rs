//! Public-API tests for `vivid_protocol::scene`.
use vivid_protocol::identity::PresenterInstanceId;
use vivid_protocol::scene::*;
use vivid_protocol::{
    identity::{NodeIdentity, SessionIdentity},
    revision::{SceneRevision, TargetGeneration},
};

fn node(owner: u64, surface_owner: u64) -> SceneNode {
    SceneNode {
        owning_context_id: owner,
        node_id: 7,
        surface_context_id: surface_owner,
        surface_id: 9,
        geometry: vec![],
        fit: Fit::Contain,
        linear_sampling: true,
        z_index: 0,
        visible: true,
        opacity: u16::MAX,
        clip: None,
    }
}

fn session() -> SessionIdentity {
    SessionIdentity::new(PresenterInstanceId([1; 16]), 1).unwrap()
}

fn node_identity(context_id: u64) -> NodeIdentity {
    session().context(context_id).unwrap().node(7).unwrap()
}

#[test]
fn failed_transaction_is_atomic() {
    let mut scene = Scene::new(session(), TargetGeneration::ONE).unwrap();
    scene.begin(2, 1).unwrap();
    scene.create(2, 1, node(2, 2)).unwrap();
    scene.create(2, 1, node(2, 2)).unwrap();
    scene.commit(2, 1, TargetGeneration::ONE, None).unwrap_err();
    assert_eq!(scene.revision(), SceneRevision::ZERO);
    assert!(scene.node(node_identity(2)).is_none());
}

#[test]
fn cleanup_is_scoped_when_numeric_ids_are_reused() {
    let mut scene = Scene::new(session(), TargetGeneration::ONE).unwrap();
    for context in [2, 3] {
        scene.begin(context, 1).unwrap();
        scene.create(context, 1, node(context, context)).unwrap();
        scene
            .commit(context, 1, TargetGeneration::ONE, None)
            .unwrap();
    }
    assert_eq!(scene.remove_surface_references(2, 9).unwrap(), 1);
    assert!(scene.node(node_identity(2)).is_none());
    assert!(scene.node(node_identity(3)).is_some());
}
