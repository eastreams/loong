use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use loong_context::{ContextItem, ContextStore, OpenError, Role};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn test_path(name: &str) -> PathBuf {
    let next = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "loong-context-{name}-{}-{next}",
        std::process::id()
    ))
}

fn head_path(base: &PathBuf, generation: u64) -> PathBuf {
    base.join(format!("{generation}.jsonl"))
}

fn item(role: Role, text: &str) -> ContextItem {
    ContextItem {
        role,
        text: text.to_owned(),
    }
}

#[test]
fn open_append_snapshot_flush_and_replay() {
    let path = test_path("roundtrip");
    let mut store = ContextStore::open(path.clone()).unwrap();

    assert_eq!(store.append(vec![item(Role::User, "hello")]), 1);
    assert_eq!(store.append(vec![item(Role::Assistant, "hi")]), 2);

    let snapshot = store.snapshot();
    assert_eq!(snapshot.version, 2);
    assert_eq!(snapshot.usage_tokens, 7);
    assert_eq!(snapshot.items.len(), 2);

    store.flush().unwrap();
    assert_eq!(store.replace(vec![item(Role::User, "again")]), 3);
    store.flush().unwrap();
    drop(store);

    // A fresh store replays the current head. Version restarts from the head
    // generation and is not durable.
    let store = ContextStore::open(path.clone()).unwrap();
    let snapshot = store.snapshot();
    assert_eq!(snapshot.version, 1);
    assert_eq!(snapshot.items, vec![item(Role::User, "again")]);
    drop(store);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn unflushed_records_do_not_replay() {
    let path = test_path("unflushed");
    let mut store = ContextStore::open(path.clone()).unwrap();
    store.append(vec![item(Role::User, "unsaved")]);
    drop(store);

    let store = ContextStore::open(path.clone()).unwrap();
    let snapshot = store.snapshot();
    assert_eq!(snapshot.version, 0);
    assert!(snapshot.items.is_empty());
    drop(store);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn torn_head_line_does_not_break_replay() {
    let path = test_path("torn");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(head_path(&path, 0), "{\"role\":\"user\"").unwrap();

    let mut store = ContextStore::open(path.clone()).unwrap();
    let snapshot = store.snapshot();
    assert_eq!(snapshot.version, 0);
    assert!(snapshot.items.is_empty());

    store.append(vec![item(Role::User, "after")]);
    store.flush().unwrap();
    drop(store);

    let store = ContextStore::open(path.clone()).unwrap();
    let snapshot = store.snapshot();
    assert_eq!(snapshot.version, 0);
    assert_eq!(snapshot.items, vec![item(Role::User, "after")]);
    drop(store);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn open_reports_failure() {
    let base = test_path("parent-file");
    std::fs::write(&base, "not a directory").unwrap();

    match ContextStore::open(base.join("sub").join("log.jsonl")) {
        Err(OpenError::Io(_)) => {}
        other => panic!("expected Io error, got {other:?}"),
    }

    let _ = std::fs::remove_file(&base);
}

#[test]
fn open_locks_the_path_exclusively() {
    let path = test_path("exclusive");
    let store = ContextStore::open(path.clone()).unwrap();

    match ContextStore::open(path.clone()) {
        Err(OpenError::InUse(_)) => {}
        other => panic!("expected InUse, got {other:?}"),
    }

    drop(store);
    let store = ContextStore::open(path.clone()).unwrap();
    drop(store);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn replace_publishes_a_new_head_and_keeps_the_old_one() {
    let path = test_path("heads");
    let mut store = ContextStore::open(path.clone()).unwrap();
    store.append(vec![item(Role::User, "v1")]);
    store.flush().unwrap();
    store.replace(vec![item(Role::User, "v2")]);
    store.flush().unwrap();
    drop(store);

    assert!(head_path(&path, 0).exists());
    assert!(head_path(&path, 1).exists());
    let head0 = std::fs::read_to_string(head_path(&path, 0)).unwrap();
    assert!(head0.contains("v1"));
    assert!(!head0.contains("v2"));

    let mut store = ContextStore::open(path.clone()).unwrap();
    let snapshot = store.snapshot();
    assert_eq!(snapshot.version, 1);
    assert_eq!(snapshot.items, vec![item(Role::User, "v2")]);

    // Appends after reopening go to the current head, not a new one.
    store.append(vec![item(Role::User, "v3")]);
    store.flush().unwrap();
    drop(store);

    let head1 = std::fs::read_to_string(head_path(&path, 1)).unwrap();
    assert!(head1.contains("v2"));
    assert!(head1.contains("v3"));

    let store = ContextStore::open(path.clone()).unwrap();
    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.items,
        vec![item(Role::User, "v2"), item(Role::User, "v3")]
    );
    drop(store);

    let _ = std::fs::remove_file(head_path(&path, 0));
    let _ = std::fs::remove_file(head_path(&path, 1));
}
