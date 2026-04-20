pub mod auth;
pub mod client;
pub mod error;
pub mod messaging_api;
pub mod outbound;
pub mod principal;
pub mod resources;
pub mod runtime;
pub mod token_store;

pub use auth::{
    FEISHU_DOC_WRITE_ACCEPTED_SCOPES, FEISHU_MESSAGE_WRITE_ACCEPTED_SCOPES,
    FEISHU_MESSAGE_WRITE_RECOMMENDED_SCOPES, FeishuAuthStartSpec, FeishuGrantAnyScopeStatus,
    FeishuGrantStatus, FeishuTokenExchangeRequest, build_authorize_url, map_user_info_to_principal,
    parse_token_exchange_response, summarize_doc_write_scope_status,
    summarize_message_write_scope_status,
};
pub use client::{
    FeishuClient, FeishuUserInfo, FeishuWsEndpoint, FeishuWsEndpointClientConfig,
    parse_user_info_response,
};
pub use error::FeishuApiError;
pub use outbound::{
    FeishuOperatorOutboundMessageInput, parse_post_json_argument,
    resolve_operator_outbound_message_body, validate_operator_outbound_message_input,
};
pub use principal::{FeishuAccountBinding, FeishuGrantScopeSet, FeishuUserPrincipal};
pub use resources::types::{
    FeishuCalendarEntry, FeishuCalendarFreebusyResult, FeishuCalendarFreebusySlot,
    FeishuCalendarListPage, FeishuCardUpdateReceipt, FeishuDocumentContent, FeishuDocumentMetadata,
    FeishuDownloadedMessageResource, FeishuMessageDetail, FeishuMessageHistoryPage,
    FeishuMessageResourceType, FeishuMessageSummary, FeishuMessageWriteReceipt,
    FeishuPrimaryCalendarEntry, FeishuPrimaryCalendarList, FeishuSearchMessagePage,
    FeishuUploadedFile, FeishuUploadedImage,
};
pub use runtime::{
    FeishuGrantInventory, FeishuGrantResolution, describe_grant_selection_error,
    describe_grant_selection_error_for_display, effective_selected_open_id,
    ensure_fresh_user_grant, inspect_grants_for_account, resolve_grant_selection,
    resolve_requested_feishu_account, resolve_selected_grant, unix_ts_now,
};
pub use token_store::{
    FeishuGrant, FeishuOauthStateRecord, FeishuStoredOauthState, FeishuTokenStore,
};
