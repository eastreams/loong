use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

use crate::{contracts::CapabilityToken, errors::PolicyError, pack::VerticalPackManifest};

pub trait PolicyEngine: Send + Sync {
    fn issue_token(
        &self,
        pack: &VerticalPackManifest,
        agent_id: &str,
        now_epoch_s: u64,
        ttl_s: u64,
    ) -> Result<CapabilityToken, PolicyError>;

    fn authorize(
        &self,
        token: &CapabilityToken,
        runtime_pack_id: &str,
        now_epoch_s: u64,
        required: &std::collections::BTreeSet<crate::contracts::Capability>,
    ) -> Result<(), PolicyError>;

    fn revoke_token(&self, token_id: &str) -> Result<(), PolicyError>;
}

#[derive(Debug, Default)]
pub struct StaticPolicyEngine {
    token_seq: AtomicU64,
    revoked_tokens: Mutex<BTreeSet<String>>,
}

impl StaticPolicyEngine {
    fn next_token_id(&self) -> String {
        let seq = self.token_seq.fetch_add(1, Ordering::Relaxed) + 1;
        format!("tok-{seq:016x}")
    }
}

impl PolicyEngine for StaticPolicyEngine {
    fn issue_token(
        &self,
        pack: &VerticalPackManifest,
        agent_id: &str,
        now_epoch_s: u64,
        ttl_s: u64,
    ) -> Result<CapabilityToken, PolicyError> {
        Ok(CapabilityToken {
            token_id: self.next_token_id(),
            pack_id: pack.pack_id.clone(),
            agent_id: agent_id.to_owned(),
            allowed_capabilities: pack.granted_capabilities.clone(),
            issued_at_epoch_s: now_epoch_s,
            expires_at_epoch_s: now_epoch_s.saturating_add(ttl_s),
        })
    }

    fn authorize(
        &self,
        token: &CapabilityToken,
        runtime_pack_id: &str,
        now_epoch_s: u64,
        required: &std::collections::BTreeSet<crate::contracts::Capability>,
    ) -> Result<(), PolicyError> {
        if self
            .revoked_tokens
            .lock()
            .map_err(|_| PolicyError::RevokedToken {
                token_id: token.token_id.clone(),
            })?
            .contains(&token.token_id)
        {
            return Err(PolicyError::RevokedToken {
                token_id: token.token_id.clone(),
            });
        }

        if token.pack_id != runtime_pack_id {
            return Err(PolicyError::PackMismatch {
                token_pack_id: token.pack_id.clone(),
                runtime_pack_id: runtime_pack_id.to_owned(),
            });
        }

        if now_epoch_s > token.expires_at_epoch_s {
            return Err(PolicyError::ExpiredToken {
                token_id: token.token_id.clone(),
                expires_at_epoch_s: token.expires_at_epoch_s,
            });
        }

        for capability in required {
            if !token.allowed_capabilities.contains(capability) {
                return Err(PolicyError::MissingCapability {
                    token_id: token.token_id.clone(),
                    capability: *capability,
                });
            }
        }

        Ok(())
    }

    fn revoke_token(&self, token_id: &str) -> Result<(), PolicyError> {
        let mut revoked = self
            .revoked_tokens
            .lock()
            .map_err(|_| PolicyError::RevokedToken {
                token_id: token_id.to_owned(),
            })?;
        revoked.insert(token_id.to_owned());
        Ok(())
    }
}
