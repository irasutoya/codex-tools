use crate::models::{AppError, FetchedModel, ProviderProfile};
use serde_json::{Map, Value, json};
use std::{collections::HashSet, fs, path::Path, process::Command};

pub const DEFAULT_CONTEXT_WINDOW: u64 = 272_000;

/// Builds a Codex model catalog from the agent schema shipped with the user's
/// installed Codex version. Provider metadata is deliberately treated as
/// untrusted: it may describe model identity and context size, but it must not
/// replace Codex instructions or tool capability declarations.
pub fn build(provider: &ProviderProfile, codex_home: &Path) -> Result<Value, AppError> {
    let catalog = load_installed_catalog(codex_home)?;
    let template = find_agent_template(&catalog).ok_or_else(missing_template_error)?;
    Ok(build_from_catalog(provider, &catalog, &template))
}

fn load_installed_catalog(codex_home: &Path) -> Result<Value, AppError> {
    let cache_path = codex_home.join("models_cache.json");
    if cache_path.exists() {
        let bytes = fs::read(&cache_path).map_err(|error| AppError::Internal(error.to_string()))?;
        let catalog: Value = serde_json::from_slice(&bytes)
            .map_err(|error| AppError::InvalidConfig(format!("models_cache.json 无效：{error}")))?;
        if catalog.get("models").is_some_and(Value::is_array) {
            return Ok(catalog);
        }
    }

    if let Ok(output) = Command::new("codex")
        .args(["debug", "models", "--bundled"])
        .output()
        && output.status.success()
        && let Ok(catalog) = serde_json::from_slice::<Value>(&output.stdout)
        && catalog.get("models").is_some_and(Value::is_array)
    {
        return Ok(catalog);
    }

    Err(missing_template_error())
}

fn missing_template_error() -> AppError {
    AppError::InvalidConfig(
        "找不到当前 Codex 版本的完整 agent 模型定义。请先启动一次 Codex，或确保 codex CLI 可用；为避免生成只能聊天的模型目录，本次切换已取消。".into(),
    )
}

fn find_agent_template(catalog: &Value) -> Option<Value> {
    catalog
        .get("models")
        .and_then(Value::as_array)
        .and_then(|models| {
            models
                .iter()
                .filter(|model| is_complete_agent_template(model))
                .max_by_key(|model| agent_template_score(model))
        })
        .cloned()
}

fn agent_template_score(model: &Value) -> u8 {
    let Some(model) = model.as_object() else {
        return 0;
    };
    let mut score = 1;
    if model.get("use_responses_lite").and_then(Value::as_bool) != Some(true) {
        score += 2;
    }
    if model.get("tool_mode").and_then(Value::as_str) != Some("code_mode_only") {
        score += 2;
    }
    if model.get("multi_agent_version").is_none_or(Value::is_null) {
        score += 1;
    }
    score
}

fn is_complete_agent_template(model: &Value) -> bool {
    let Some(model) = model.as_object() else {
        return false;
    };
    non_empty_string(model, "base_instructions")
        && model.get("model_messages").is_some_and(Value::is_object)
        && non_empty_string(model, "apply_patch_tool_type")
        && non_empty_string(model, "shell_type")
}

fn non_empty_string(model: &Map<String, Value>, key: &str) -> bool {
    model
        .get(key)
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

fn build_from_catalog(provider: &ProviderProfile, installed: &Value, template: &Value) -> Value {
    let mut seen = HashSet::new();
    let models = provider
        .models
        .iter()
        .filter_map(|raw_model| {
            let model = raw_model.trim();
            if model.is_empty() || !seen.insert(model.to_owned()) {
                return None;
            }

            if let Some(mut entry) = find_exact_model(installed, model) {
                if provider.protocol == crate::models::ProviderProtocol::ChatCompletions {
                    make_proxy_transport_safe(&mut entry);
                }
                return Some(entry);
            }

            let metadata = provider
                .model_metadata
                .iter()
                .find(|metadata| metadata.id.trim() == model);
            let context_window = metadata
                .and_then(model_context_window)
                .or(provider.context_window)
                .filter(|value| *value > 0)
                .unwrap_or(DEFAULT_CONTEXT_WINDOW);

            let mut entry = template.clone();
            let object = entry.as_object_mut()?;
            object.insert("slug".into(), model.into());
            object.insert("display_name".into(), model.into());
            object.insert(
                "description".into(),
                format!("{model} via {}", provider.name).into(),
            );
            object.insert("priority".into(), (1000 + seen.len() as u64).into());
            object.insert("visibility".into(), "list".into());
            object.insert("supported_in_api".into(), true.into());
            object.insert("context_window".into(), context_window.into());
            object.insert("max_context_window".into(), context_window.into());
            object.insert("effective_context_window_percent".into(), 100.into());
            object.insert("auto_compact_token_limit".into(), Value::Null);
            // These fields select model-specific Codex transport/runtime
            // paths. They are valid for the source model but unsafe to clone
            // onto an unrelated upstream model. Third-party providers always
            // use the normal Responses surface exposed by this application.
            object.insert("use_responses_lite".into(), false.into());
            object.remove("tool_mode");
            object.remove("multi_agent_version");
            object.insert("additional_speed_tiers".into(), json!([]));
            object.insert("service_tiers".into(), json!([]));
            object.insert("availability_nux".into(), Value::Null);
            object.insert("upgrade".into(), Value::Null);
            Some(entry)
        })
        .collect::<Vec<_>>();

    json!({ "models": models })
}

fn find_exact_model(catalog: &Value, slug: &str) -> Option<Value> {
    catalog
        .get("models")
        .and_then(Value::as_array)
        .and_then(|models| {
            models.iter().find(|model| {
                model.get("slug").and_then(Value::as_str) == Some(slug)
                    && is_complete_agent_template(model)
            })
        })
        .cloned()
}

fn make_proxy_transport_safe(entry: &mut Value) {
    let Some(model) = entry.as_object_mut() else {
        return;
    };
    model.insert("use_responses_lite".into(), false.into());
    model.remove("tool_mode");
    model.remove("multi_agent_version");
}

fn model_context_window(model: &FetchedModel) -> Option<u64> {
    [
        "context_window",
        "contextWindow",
        "max_context_window",
        "maxContextWindow",
    ]
    .iter()
    .find_map(|name| model.metadata.get(*name))
    .and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str()?.trim().parse().ok())
    })
    .filter(|value| *value > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProviderProfile, ProviderProtocol};

    fn provider() -> ProviderProfile {
        ProviderProfile {
            id: "provider".into(),
            name: "Third party".into(),
            protocol: ProviderProtocol::Responses,
            base_url: "https://example.test/v1".into(),
            models: vec!["model-b".into(), "model-a".into(), "model-b".into()],
            model_metadata: vec![FetchedModel {
                id: "model-b".into(),
                owned_by: None,
                metadata: serde_json::from_value(json!({
                    "context_window": 96_000,
                    "base_instructions": "malicious replacement",
                    "model_messages": {"instructions_template": "malicious"},
                    "apply_patch_tool_type": "disabled",
                    "shell_type": "none",
                    "tool_mode": "chat",
                    "supports_parallel_tool_calls": false
                }))
                .unwrap(),
            }],
            codex_chat_reasoning: None,
            headers: json!({}),
            timeout_secs: 30,
            context_window: None,
            auto_compact_threshold: None,
            enabled: true,
            active: false,
            active_account_id: None,
            account_count: 0,
        }
    }

    fn complete_template() -> Value {
        json!({
            "slug": "installed-model",
            "display_name": "Installed model",
            "base_instructions": "complete Codex instructions",
            "model_messages": {"instructions_template": "Codex agent template"},
            "apply_patch_tool_type": "freeform",
            "shell_type": "shell_command",
            "tool_mode": "default",
            "supports_parallel_tool_calls": true,
            "default_reasoning_level": "high",
            "supported_reasoning_levels": [{"effort": "high", "description": "High"}]
        })
    }

    #[test]
    fn chooses_first_complete_agent_entry_without_fixed_slug() {
        let catalog = json!({"models": [
            {"slug": "incomplete", "base_instructions": "text"},
            complete_template(),
            {"slug": "later"}
        ]});
        assert_eq!(
            find_agent_template(&catalog).unwrap()["slug"],
            "installed-model"
        );
    }

    #[test]
    fn prefers_standard_agent_template_over_model_specific_lite_mode() {
        let mut lite = complete_template();
        lite["slug"] = json!("newer-model-specific-entry");
        lite["use_responses_lite"] = json!(true);
        lite["tool_mode"] = json!("code_mode_only");
        lite["multi_agent_version"] = json!("v2");
        let mut standard = complete_template();
        standard["slug"] = json!("standard-agent-entry");
        standard["use_responses_lite"] = json!(false);
        standard.as_object_mut().unwrap().remove("tool_mode");
        let catalog = json!({"models": [lite, standard]});

        assert_eq!(
            find_agent_template(&catalog).unwrap()["slug"],
            "standard-agent-entry"
        );
    }

    #[test]
    fn rejects_templates_missing_any_agent_capability_field() {
        for field in [
            "base_instructions",
            "model_messages",
            "apply_patch_tool_type",
            "shell_type",
        ] {
            let mut template = complete_template();
            template.as_object_mut().unwrap().remove(field);
            assert!(
                !is_complete_agent_template(&template),
                "accepted without {field}"
            );
        }
    }

    #[test]
    fn only_identity_and_context_can_change_agent_template() {
        let template = complete_template();
        let installed = json!({"models":[template.clone()]});
        let catalog = build_from_catalog(&provider(), &installed, &template);
        let models = catalog["models"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["slug"], "model-b");
        assert_eq!(models[1]["slug"], "model-a");
        assert_eq!(models[0]["context_window"], 96_000);
        assert_eq!(models[1]["context_window"], DEFAULT_CONTEXT_WINDOW);
        assert_eq!(
            models[0]["base_instructions"],
            "complete Codex instructions"
        );
        assert_eq!(
            models[0]["model_messages"]["instructions_template"],
            "Codex agent template"
        );
        assert_eq!(models[0]["apply_patch_tool_type"], "freeform");
        assert_eq!(models[0]["shell_type"], "shell_command");
        assert!(models[0].get("tool_mode").is_none());
        assert!(models[0].get("multi_agent_version").is_none());
        assert_eq!(models[0]["use_responses_lite"], false);
        assert_eq!(models[0]["effective_context_window_percent"], 100);
        assert!(models[0]["auto_compact_token_limit"].is_null());
        assert_eq!(models[0]["supports_parallel_tool_calls"], true);
    }

    fn exact_model() -> Value {
        json!({
            "slug":"gpt-5.6-sol",
            "display_name":"GPT-5.6-Sol",
            "description":"Installed model description",
            "base_instructions":"model-specific instructions",
            "model_messages":{"instructions_template":"model-specific template","future_variable":"kept"},
            "apply_patch_tool_type":"freeform",
            "shell_type":"shell_command",
            "context_window":372000,
            "max_context_window":372000,
            "effective_context_window_percent":95,
            "supported_reasoning_levels":[{"effort":"max","description":"Maximum"}],
            "use_responses_lite":true,
            "tool_mode":"code_mode_only",
            "multi_agent_version":"v1",
            "future_codex_field":{"nested":[1,2,3]}
        })
    }

    #[test]
    fn responses_exact_match_is_copied_byte_for_byte_as_json_value() {
        let exact = exact_model();
        let template = complete_template();
        let installed = json!({"models":[template.clone(),exact.clone()]});
        let mut provider = provider();
        provider.models = vec!["gpt-5.6-sol".into()];
        provider.model_metadata = vec![FetchedModel {
            id: "gpt-5.6-sol".into(),
            owned_by: None,
            metadata: serde_json::from_value(json!({"context_window":96000})).unwrap(),
        }];

        let generated = build_from_catalog(&provider, &installed, &template);
        assert_eq!(generated["models"][0], exact);
    }

    #[test]
    fn chat_exact_match_only_removes_transport_specific_switches() {
        let exact = exact_model();
        let template = complete_template();
        let installed = json!({"models":[template.clone(),exact.clone()]});
        let mut provider = provider();
        provider.protocol = ProviderProtocol::ChatCompletions;
        provider.models = vec!["gpt-5.6-sol".into()];

        let generated = build_from_catalog(&provider, &installed, &template);
        let model = &generated["models"][0];
        assert_eq!(model["context_window"], 372000);
        assert_eq!(model["effective_context_window_percent"], 95);
        assert_eq!(model["base_instructions"], "model-specific instructions");
        assert_eq!(model["model_messages"]["future_variable"], "kept");
        assert_eq!(
            model["supported_reasoning_levels"],
            exact["supported_reasoning_levels"]
        );
        assert_eq!(model["future_codex_field"], exact["future_codex_field"]);
        assert_eq!(model["use_responses_lite"], false);
        assert!(model.get("tool_mode").is_none());
        assert!(model.get("multi_agent_version").is_none());
    }

    #[test]
    fn exact_and_custom_models_share_one_catalog_in_provider_order() {
        let exact = exact_model();
        let template = complete_template();
        let installed = json!({"models":[template.clone(),exact]});
        let mut provider = provider();
        provider.models = vec!["custom-model".into(), "gpt-5.6-sol".into()];
        provider.model_metadata.clear();

        let generated = build_from_catalog(&provider, &installed, &template);
        assert_eq!(generated["models"][0]["slug"], "custom-model");
        assert_eq!(
            generated["models"][0]["context_window"],
            DEFAULT_CONTEXT_WINDOW
        );
        assert_eq!(generated["models"][1]["slug"], "gpt-5.6-sol");
        assert_eq!(generated["models"][1]["context_window"], 372000);
    }
}
