use std::path::Path;

use agentdesktop_core::config::{GooseConfig, LlmGatewayConfig};
use anyhow::Context;
use serde_json::{Value, json};

use crate::{
    provider::{json_merge, shared::responses_base_url},
    reconcile::ReconcilePlan,
};

use super::Goose;

const SETTINGS_OPTIONS: json_merge::MergeOptions = json_merge::MergeOptions {
    mode: 0o600,
    keyed_arrays: &[],
    redact_diff: true,
    format: json_merge::JsonFormat::Yaml,
    replace_paths: &[],
};

const PROVIDER_OPTIONS: json_merge::MergeOptions = json_merge::MergeOptions {
    mode: 0o600,
    keyed_arrays: &[json_merge::KeyedArray {
        field: "models",
        keys: &["name"],
    }],
    redact_diff: true,
    format: json_merge::JsonFormat::Json,
    replace_paths: &[],
};

pub(super) fn plan(
    config_path: &Path,
    provider_path: &Path,
    credential_helper: &Path,
    socket: &Path,
    config: Option<(&GooseConfig, Option<&LlmGatewayConfig>)>,
    plan: &ReconcilePlan,
) -> anyhow::Result<()> {
    let config_state = json_merge::state_path(config_path);
    let provider_state = json_merge::state_path(provider_path);
    let Some((config, gateway)) = config else {
        json_merge::plan_remove_with(
            config_path,
            &config_state,
            "configuration",
            Goose::DISPLAY_NAME,
            SETTINGS_OPTIONS,
            plan,
        )?;
        json_merge::plan_remove_with(
            provider_path,
            &provider_state,
            "gateway provider",
            Goose::DISPLAY_NAME,
            PROVIDER_OPTIONS,
            plan,
        )?;
        return Ok(());
    };

    let settings = managed_settings(config, gateway)?;
    if !settings.as_object().is_some_and(serde_json::Map::is_empty)
        || json_merge::managed_after(&config_state, Goose::DISPLAY_NAME, plan)?.is_some()
    {
        json_merge::plan_merge_with(
            config_path,
            &config_state,
            settings,
            false,
            "configuration",
            Goose::DISPLAY_NAME,
            SETTINGS_OPTIONS,
            plan,
        )?;
    }

    if let Some(gateway) = gateway {
        let provider = managed_provider(config, gateway, credential_helper, socket)?;
        json_merge::plan_merge_with(
            provider_path,
            &provider_state,
            provider,
            false,
            "gateway provider",
            Goose::DISPLAY_NAME,
            PROVIDER_OPTIONS,
            plan,
        )?;
    } else {
        json_merge::plan_remove_with(
            provider_path,
            &provider_state,
            "gateway provider",
            Goose::DISPLAY_NAME,
            PROVIDER_OPTIONS,
            plan,
        )?;
    }
    Ok(())
}

fn managed_settings(
    config: &GooseConfig,
    gateway: Option<&LlmGatewayConfig>,
) -> anyhow::Result<Value> {
    let mut settings = serde_json::to_value(&config.managed_config)
        .context("serialize Goose pass-through managed configuration")?;
    if gateway.is_some() {
        let model = config
            .model
            .as_deref()
            .filter(|model| !model.trim().is_empty())
            .context("Goose gateway configuration has no model")?;
        let generated = json!({
            "active_provider": "custom_agentdesktop",
            "GOOSE_PROVIDER": "custom_agentdesktop",
            "GOOSE_MODEL": model,
            "providers": {
                "custom_agentdesktop": {
                    "enabled": true,
                    "model": model,
                    "configured": true,
                }
            },
        });
        crate::provider::shared::deep_merge(&mut settings, generated);
    }
    Ok(settings)
}

fn managed_provider(
    config: &GooseConfig,
    gateway: &LlmGatewayConfig,
    credential_helper: &Path,
    socket: &Path,
) -> anyhow::Result<Value> {
    let model = config
        .model
        .as_deref()
        .filter(|model| !model.trim().is_empty())
        .context("Goose gateway configuration has no model")?;
    let auth = gateway.authentication.as_ref().map(|_| {
        json!({
            "command": credential_helper,
            "args": [
                "--socket",
                socket.to_string_lossy(),
                "credential",
                "--client-id",
                Goose::ID,
            ],
            "refresh_interval": 60,
            "timeout_seconds": 30,
        })
    });
    let mut provider = json!({
        "name": "custom_agentdesktop",
        "engine": "openai",
        "display_name": "Agentdesktop Gateway",
        "description": "Managed by Agentdesktop",
        "api_key_env": "",
        "base_url": responses_base_url(gateway),
        "models": [{"name": model}],
        "supports_streaming": true,
        "requires_auth": auth.is_some(),
        "dynamic_models": config.dynamic_models,
    });
    if let Some(auth) = auth {
        provider["auth"] = auth;
    }
    Ok(provider)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use agentdesktop_core::config::parse_daemon;
    use serde_json::Value;

    use crate::reconcile::ReconcilePlan;

    use super::{managed_provider, plan};

    fn fixture(config_path: &Path, config: Option<&str>) -> ReconcilePlan {
        let config = config.map(|yaml| parse_daemon(yaml).unwrap());
        let provider_path = config_path
            .parent()
            .unwrap()
            .join("custom_providers/custom_agentdesktop.json");
        let changes = ReconcilePlan::default();
        plan(
            config_path,
            &provider_path,
            Path::new("/usr/bin/agentdesktop"),
            Path::new("/run/agentdesktop.sock"),
            config.as_ref().map(|config| {
                (
                    config.programs.goose.as_ref().unwrap(),
                    config.llm_gateway.as_ref(),
                )
            }),
            &changes,
        )
        .unwrap();
        changes
    }

    #[test]
    fn writes_goose_yaml_and_native_command_auth_provider() {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("goose/config.yaml");
        let config = parse_daemon(
            "llmGateway:\n  url: https://gateway.example.com\n  authentication:\n    type: controllerJwt\n    audience: gateway\n    allowedClientIds: [goose]\nprograms:\n  goose:\n    model: gpt-4.1\n",
        )
        .unwrap();
        let provider = managed_provider(
            config.programs.goose.as_ref().unwrap(),
            config.llm_gateway.as_ref().unwrap(),
            Path::new("/usr/bin/agentdesktop"),
            Path::new("/run/agentdesktop.sock"),
        )
        .unwrap();
        assert_eq!(provider["name"], "custom_agentdesktop");
        assert_eq!(provider["base_url"], "https://gateway.example.com/v1");
        assert_eq!(provider["auth"]["command"], "/usr/bin/agentdesktop");
        assert_eq!(provider["auth"]["args"][4], "goose");
        assert_eq!(provider["auth"]["refresh_interval"], 60);

        let changes = fixture(
            &config_path,
            Some(
                "llmGateway:\n  url: https://gateway.example.com\n  authentication:\n    type: controllerJwt\n    audience: gateway\n    allowedClientIds: [goose]\nprograms:\n  goose:\n    model: gpt-4.1\n    managedConfig:\n      GOOSE_MODE: auto\n",
            ),
        );
        changes.apply().unwrap();
        let settings = agentdesktop_core::serdes::yamlviajson::from_str::<Value>(
            &fs::read_to_string(&config_path).unwrap(),
        )
        .unwrap();
        assert_eq!(settings["GOOSE_PROVIDER"], "custom_agentdesktop");
        assert_eq!(settings["GOOSE_MODEL"], "gpt-4.1");
        assert_eq!(settings["GOOSE_MODE"], "auto");
        let provider_path = root
            .path()
            .join("goose/custom_providers/custom_agentdesktop.json");
        let provider: Value = serde_json::from_slice(&fs::read(provider_path).unwrap()).unwrap();
        assert_eq!(provider["auth"]["args"][4], "goose");
    }

    #[test]
    fn preserves_existing_goose_values_and_restores_them_on_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("goose/config.yaml");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::write(
            &config_path,
            "GOOSE_PROVIDER: anthropic\nGOOSE_MODEL: user-model\nGOOSE_MODE: auto\n",
        )
        .unwrap();
        fixture(
            &config_path,
            Some("llmGateway:\n  url: https://gateway.example.com\nprograms:\n  goose:\n    model: gpt-4.1\n"),
        )
        .apply()
        .unwrap();
        let settings: Value = agentdesktop_core::serdes::yamlviajson::from_str(
            &fs::read_to_string(&config_path).unwrap(),
        )
        .unwrap();
        assert_eq!(settings["GOOSE_PROVIDER"], "custom_agentdesktop");
        assert_eq!(settings["GOOSE_MODE"], "auto");
        fixture(&config_path, None).apply().unwrap();
        let restored: Value = agentdesktop_core::serdes::yamlviajson::from_str(
            &fs::read_to_string(&config_path).unwrap(),
        )
        .unwrap();
        assert_eq!(restored["GOOSE_PROVIDER"], "anthropic");
        assert_eq!(restored["GOOSE_MODEL"], "user-model");
        assert_eq!(restored["GOOSE_MODE"], "auto");
    }

    #[test]
    fn invalid_goose_yaml_reports_a_conflict_without_overwriting_it() {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("goose/config.yaml");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::write(&config_path, "[not: valid yaml\n").unwrap();
        let changes = fixture(
            &config_path,
            Some("programs:\n  goose:\n    managedConfig:\n      GOOSE_MODE: auto\n"),
        );
        assert!(changes.render().contains("CONFLICT"));
        assert!(changes.apply().is_err());
        assert_eq!(
            fs::read_to_string(config_path).unwrap(),
            "[not: valid yaml\n"
        );
    }
}
