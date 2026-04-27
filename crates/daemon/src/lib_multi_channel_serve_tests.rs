use std::collections::BTreeSet;

use super::*;

#[test]
fn supported_multi_channel_serve_channel_ids_follow_background_runtime_registry() {
    let expected_ids = mvp::channel::background_channel_runtime_descriptors()
        .into_iter()
        .map(|descriptor| descriptor.channel_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let actual_ids = supported_multi_channel_serve_channel_ids();

    assert_eq!(actual_ids, expected_ids);
}

#[test]
fn supported_multi_channel_serve_channel_ids_match_gateway_supervised_runtime_channels() {
    let expected_ids = mvp::channel::gateway_supervised_channel_descriptors()
        .into_iter()
        .map(|descriptor| descriptor.id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let actual_ids = supported_multi_channel_serve_channel_ids();

    assert_eq!(actual_ids, expected_ids);
}

#[test]
fn parse_multi_channel_serve_channel_account_rejects_compiled_out_matrix_runtime() {
    let supported_channel_ids = supported_multi_channel_serve_channel_ids();
    let matrix_is_supported = supported_channel_ids.contains(&"matrix");
    if matrix_is_supported {
        return;
    }

    let error = parse_multi_channel_serve_channel_account("matrix=bridge-sync")
        .expect_err("compiled-out matrix runtime should be rejected");

    assert!(
        error.contains(
            "multi-channel service channel `matrix` resolves to `matrix` but is not supported in this build"
        )
    );
}

#[test]
fn parse_multi_channel_serve_channel_account_rejects_unknown_runtime_channel() {
    let error = parse_multi_channel_serve_channel_account("unknown=bridge-sync")
        .expect_err("unknown runtime channel should be rejected");

    assert!(error.contains("unrecognized multi-channel service channel `unknown`"));
}
