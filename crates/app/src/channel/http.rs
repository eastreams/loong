// Re-export from new location for backward compatibility
pub use super::core::http::ChannelOutboundHttpPolicy;
pub use super::core::http::build_outbound_http_client;
pub use super::core::http::read_json_or_text_response;
pub use super::core::http::redact_endpoint_status_url;
pub use super::core::http::redact_generic_webhook_status_url;
pub use super::core::http::response_body_detail;
pub use super::core::http::validate_outbound_http_base_url;
pub use super::core::http::validate_outbound_http_target;
pub use super::runtime::http::outbound_http_policy_from_config;
