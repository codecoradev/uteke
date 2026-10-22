//! What the transports use instead of reaching into the store (#1521).

use crate::test_support::open_keyword;

#[test]
fn lifecycle_counts_and_the_deprecated_list_come_from_uteke() {
    let u = open_keyword();
    let keep = u
        .remember_detailed("keep this one", &[], None, None)
        .unwrap()
        .id;
    let drop = u
        .remember_detailed("retire this one", &[], None, None)
        .unwrap()
        .id;
    u.store().deprecate(&drop).unwrap();

    assert_eq!(u.count_active(None).unwrap(), 1);
    assert_eq!(u.count_deprecated(None).unwrap(), 1);
    assert_eq!(u.count_active(Some("no-such-namespace")).unwrap(), 0);

    let listed = u.list_deprecated(None, 10).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, drop);
    assert_ne!(listed[0].id, keep);
}

#[test]
fn the_graph_is_reachable_without_the_raw_connection() {
    let u = open_keyword();
    assert!(u.graph().all_nodes().unwrap().is_empty());
    let a = u.remember_detailed("graph a", &[], None, None).unwrap().id;
    let b = u.remember_detailed("graph b", &[], None, None).unwrap().id;
    u.graph()
        .add_edge_for_memories(&a, &b, "related_to", 1.0)
        .unwrap();
    assert!(u.graph().all_nodes().unwrap().len() >= 2);
}
