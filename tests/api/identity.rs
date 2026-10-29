//! Public-API tests for `vivid_protocol::identity`.
use vivid_protocol::identity::*;

#[test]
fn reused_local_numbers_do_not_alias_across_owners_or_presenters() {
    let presenter_a = PresenterInstanceId([1; 16]);
    let presenter_b = PresenterInstanceId([2; 16]);
    let first = SessionIdentity::new(presenter_a, 1)
        .unwrap()
        .context(2)
        .unwrap()
        .surface(3)
        .unwrap()
        .track(4)
        .unwrap();
    let sibling = SessionIdentity::new(presenter_a, 1)
        .unwrap()
        .context(5)
        .unwrap()
        .surface(3)
        .unwrap()
        .track(4)
        .unwrap();
    let other_presenter = SessionIdentity::new(presenter_b, 1)
        .unwrap()
        .context(2)
        .unwrap()
        .surface(3)
        .unwrap()
        .track(4)
        .unwrap();
    assert_ne!(first, sibling);
    assert_ne!(first, other_presenter);
}
