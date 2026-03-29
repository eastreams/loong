use crate::onboard_state::{OnboardDraft, OnboardWizardStep};

#[derive(Debug, Clone, PartialEq)]
pub struct OnboardFlowController {
    draft: OnboardDraft,
    cursor: usize,
}

impl OnboardFlowController {
    pub const fn ordered_steps() -> &'static [OnboardWizardStep] {
        &[
            OnboardWizardStep::Welcome,
            OnboardWizardStep::Authentication,
            OnboardWizardStep::RuntimeDefaults,
            OnboardWizardStep::Workspace,
            OnboardWizardStep::Protocols,
            OnboardWizardStep::EnvironmentCheck,
            OnboardWizardStep::ReviewAndWrite,
            OnboardWizardStep::Ready,
        ]
    }

    pub fn new(draft: OnboardDraft) -> Self {
        Self { draft, cursor: 0 }
    }

    pub fn current_step(&self) -> OnboardWizardStep {
        Self::ordered_steps()
            .get(self.cursor)
            .copied()
            .unwrap_or(OnboardWizardStep::Ready)
    }

    pub const fn draft(&self) -> &OnboardDraft {
        &self.draft
    }

    pub fn draft_mut(&mut self) -> &mut OnboardDraft {
        &mut self.draft
    }

    pub fn advance(&mut self) -> OnboardWizardStep {
        if self.cursor + 1 < Self::ordered_steps().len() {
            self.cursor += 1;
        }
        self.current_step()
    }

    pub fn back(&mut self) -> OnboardWizardStep {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
        self.current_step()
    }

    pub fn skip(&mut self) -> OnboardWizardStep {
        self.advance()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use loongclaw_app as mvp;

    use super::*;
    use crate::onboard_state::OnboardValueOrigin;

    fn sample_draft() -> OnboardDraft {
        let mut config = mvp::config::LoongClawConfig::default();
        config.memory.sqlite_path = "/starting/memory.sqlite3".to_owned();
        config.tools.file_root = Some("/starting/workspace".to_owned());
        config.acp.backend = Some("builtin".to_owned());
        OnboardDraft::from_config(
            config,
            PathBuf::from("/tmp/loongclaw.toml"),
            Some(OnboardValueOrigin::DetectedStartingPoint),
        )
    }

    #[test]
    fn wizard_steps_follow_the_expected_single_pass_order() {
        let mut controller = OnboardFlowController::new(sample_draft());
        let mut visited = vec![controller.current_step()];
        while controller.current_step() != OnboardWizardStep::Ready {
            visited.push(controller.advance());
        }

        assert_eq!(
            visited,
            vec![
                OnboardWizardStep::Welcome,
                OnboardWizardStep::Authentication,
                OnboardWizardStep::RuntimeDefaults,
                OnboardWizardStep::Workspace,
                OnboardWizardStep::Protocols,
                OnboardWizardStep::EnvironmentCheck,
                OnboardWizardStep::ReviewAndWrite,
                OnboardWizardStep::Ready,
            ]
        );
    }

    #[test]
    fn wizard_transition_rules_preserve_draft_state_across_back_and_skip() {
        let mut controller = OnboardFlowController::new(sample_draft());

        assert_eq!(controller.advance(), OnboardWizardStep::Authentication);
        assert_eq!(controller.advance(), OnboardWizardStep::RuntimeDefaults);
        assert_eq!(controller.advance(), OnboardWizardStep::Workspace);
        controller
            .draft_mut()
            .set_workspace_file_root(PathBuf::from("/user/workspace"));

        assert_eq!(controller.advance(), OnboardWizardStep::Protocols);
        controller
            .draft_mut()
            .set_acp_backend(Some("jsonrpc".to_owned()));

        assert_eq!(controller.skip(), OnboardWizardStep::EnvironmentCheck);
        assert_eq!(controller.back(), OnboardWizardStep::Protocols);
        assert_eq!(
            controller.draft().workspace.file_root,
            PathBuf::from("/user/workspace")
        );
        assert_eq!(
            controller.draft().protocols.acp_backend.as_deref(),
            Some("jsonrpc")
        );
        assert_eq!(controller.skip(), OnboardWizardStep::EnvironmentCheck);
        assert_eq!(controller.advance(), OnboardWizardStep::ReviewAndWrite);
    }
}
