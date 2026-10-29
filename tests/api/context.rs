//! Public-API tests for `vivid_protocol::context`.
use vivid_protocol::context::*;
use vivid_protocol::identity::PresenterInstanceId;
use vivid_protocol::resource::Resource;
use vivid_protocol::{identity::SessionIdentity, resource::ResourceContract};

#[test]
fn sibling_contracts_reserve_parent_capacity() {
    let mut contract = ResourceContract::denied();
    contract.set(Resource::Surfaces, 4);
    let session = SessionIdentity::new(PresenterInstanceId([1; 16]), 1).unwrap();
    let mut root = ContextState::root(session, 1, OP_DELEGATE, contract.clone()).unwrap();
    let mut requested = ResourceContract::denied();
    requested.set(Resource::Surfaces, 3);
    let definition = ContextDefinition {
        context_id: 2,
        parent_context_id: 1,
        operation_classes: 0,
        label: String::new(),
        lifetime_us: 0,
        requested_contract: requested,
    };
    let (_, first) = root.reserve_child(&definition, &contract).unwrap();
    let (_, second) = root.reserve_child(&definition, &contract).unwrap();
    assert_eq!(first.get(Resource::Surfaces), 3);
    assert_eq!(second.get(Resource::Surfaces), 1);
}
