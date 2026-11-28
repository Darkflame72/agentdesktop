use std::path::PathBuf;

use agentdesktop_core::{
    config::DaemonConfig,
    model::{DaemonScope, Discovery},
};

use super::{Provider, ReconcileContext};
use crate::reconcile::ReconcilePlan;

pub(super) mod discovery;
mod reconcile;

#[derive(Clone, Default)]
pub struct Goose {
    /// User paths only; system daemons have no Goose files to manage.
    pub config_path: Option<PathBuf>,
    /// Goose resolves custom providers relative to its config directory, even
    /// when the config YAML file itself has been overridden.
    pub custom_provider_path: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use agentdesktop_core::config::parse_daemon;

    use super::{Goose, Provider, ReconcileContext};

    #[test]
    fn system_mode_skips_goose_without_touching_user_configuration() {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("goose/config.yaml");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::write(&config_path, "GOOSE_MODE: manual\n").unwrap();
        let provider = Goose {
            config_path: Some(config_path.clone()),
            custom_provider_path: Some(
                root.path()
                    .join("goose/custom_providers/custom_agentdesktop.json"),
            ),
        };
        let context = ReconcileContext {
            merge_user_settings: false,
            user_mode: false,
            credential_helper: PathBuf::from("agentdesktop"),
            socket: PathBuf::from("agentdesktop.sock"),
            llm_proxy: None,
        };
        let config =
            parse_daemon("programs:\n  goose:\n    managedConfig:\n      GOOSE_MODE: auto\n")
                .unwrap();

        let plan = provider.plan(&context, &config).unwrap();
        assert!(plan.inactive_reason().unwrap().contains("--user"));
        plan.apply().unwrap();
        assert_eq!(
            fs::read_to_string(config_path).unwrap(),
            "GOOSE_MODE: manual\n"
        );
    }
}

impl Goose {
    pub const ID: &'static str = "goose";
    pub const DISPLAY_NAME: &'static str = "Goose";
}

#[async_trait::async_trait]
impl Provider for Goose {
    fn id(&self) -> &'static str {
        Self::ID
    }

    async fn discover(&self) -> Discovery {
        Discovery {
            agents: discovery::discover().into_iter().collect(),
            model_runtimes: Vec::new(),
        }
    }

    fn plan(&self, ctx: &ReconcileContext, config: &DaemonConfig) -> anyhow::Result<ReconcilePlan> {
        if let Some(plan) = ctx.skip_unless(
            DaemonScope::User,
            config.programs.goose.is_some(),
            Self::DISPLAY_NAME,
        ) {
            return Ok(plan);
        }
        let (Some(config_path), Some(provider_path)) =
            (&self.config_path, &self.custom_provider_path)
        else {
            if config.programs.goose.is_some() {
                anyhow::bail!("Goose user configuration path is unavailable");
            }
            return Ok(ReconcilePlan::default());
        };
        let configured = config.programs.goose.as_ref().map(|provider| {
            let gateway = config
                .llm_gateway
                .as_ref()
                .filter(|_| provider.use_llm_gateway);
            (provider, gateway)
        });
        let plan = ReconcilePlan::default();
        reconcile::plan(
            config_path,
            provider_path,
            &ctx.credential_helper,
            &ctx.socket,
            configured,
            &plan,
        )?;
        Ok(plan)
    }
}
