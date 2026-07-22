//! Bearer-backed ingress retained for kernel planes that have not migrated.
//!
//! Typed Tool and Access execution must never depend on this module. Keeping
//! token minting private here prevents `Runtime`, `Session`, and `Context` APIs
//! from being shaped by the legacy pack/token protocol.

use loong_contracts::CapabilityToken;
use loong_runtime::runtime::Runtime;

use crate::{RuntimeContextFactory, Session};

pub(crate) const EMBEDDED_RUNTIME_PACK_ID: &str = "dev-automation";
pub(crate) const DEFAULT_LEGACY_TOKEN_TTL_S: u64 = 86_400;

/// Mint bearer evidence only while constructing a concrete legacy owner.
///
/// This helper exists because several independent legacy leaves still consume
/// the same kernel token protocol. It must not be exported or called by typed
/// Tool, Access, Action, or Policy code.
pub(crate) fn issue_session_token(
    runtime: &Runtime<RuntimeContextFactory>,
    session: &Session,
    ttl_s: u64,
) -> Result<CapabilityToken, String> {
    let allowed_capabilities = session.baseline_capabilities().iter().collect();
    runtime
        .legacy_kernel()
        .issue_scoped_token(
            EMBEDDED_RUNTIME_PACK_ID,
            session.agent_id(),
            &allowed_capabilities,
            ttl_s,
        )
        .map_err(|error| format!("legacy session token issue failed: {error}"))
}
