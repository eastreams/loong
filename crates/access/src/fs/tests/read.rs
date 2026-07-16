use super::*;

#[tokio::test]
async fn tool_context_like_chain_grants_read_file_via_access_then_fs() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-read-output");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("notes")).expect("create notes dir");
    fs::write(workspace_root.join("notes/todo.md"), "hello").expect("write note");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .read_file("notes/todo.md")
        .await
        .expect("grant should succeed");

    let expected_path =
        dunce::canonicalize(workspace_root.join("notes/todo.md")).expect("canonical note path");
    assert_eq!(output.path, expected_path);
    assert_eq!(output.bytes, b"hello");

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn read_access_grants_resolve_path_and_read_actions_in_order() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-read-actions");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::write(workspace_root.join("note.txt"), "evidence").expect("write note");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    ctx.access()
        .fs()
        .read_file("note.txt")
        .await
        .expect("read should be authorized");

    let evidence = kernel
        .policy
        .evidence
        .lock()
        .expect("filesystem access evidence log");
    assert_eq!(evidence.len(), 3);
    let mut grant_ids = Vec::new();
    for (item, kind) in evidence
        .iter()
        .zip(["fs.resolve_path", "fs.path", "fs.read"])
    {
        assert_eq!(item.action.kind, kind);
        let AuthorizationAttempt::Started {
            event:
                AuthorizationAttemptEvent::Policy {
                    event:
                        AuthorizationPolicyEvent::Terminal(AuthorizationTerminalOutcome::Allow {
                            grant_id,
                        }),
                    ..
                },
            ..
        } = &item.attempt
        else {
            panic!("filesystem action should record a terminal allow grant");
        };
        grant_ids.push(*grant_id);
    }
    grant_ids.sort_unstable();
    grant_ids.dedup();
    assert_eq!(grant_ids.len(), 3, "each action must receive its own grant");

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_read_execution_boundary_consumes_granted_action() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-granted-boundary");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("notes")).expect("create notes dir");
    fs::write(workspace_root.join("notes/todo.md"), "hello").expect("write note");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let path = grant_target_path(&kernel, &ctx, "notes/todo.md").await;
    let action = FsReadAction::new(path);
    let grant = kernel
        .policy_engine()
        .grant(&ctx, action)
        .await
        .expect("policy should grant read");

    let output = grant
        .into_granted()
        .run(&ctx)
        .await
        .expect("granted read should execute");

    let expected_path =
        dunce::canonicalize(workspace_root.join("notes/todo.md")).expect("canonical note path");
    assert_eq!(output.path, expected_path);
    assert_eq!(output.bytes, b"hello");

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_access_denies_before_reading_file() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-deny-before-read");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .read_file("missing.txt")
        .await
        .expect_err("policy denial should happen before file read");

    assert!(matches!(error, FsAccessError::Authorization(_)));

    fs::remove_dir_all(base).ok();
}
