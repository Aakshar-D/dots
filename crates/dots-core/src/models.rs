//! The models a dot can pick (model switcher spec section 2.1): a curated list for the Claude
//! engine, and what a local endpoint lists at `/v1/models`.

use serde::{Deserialize, Serialize};

use crate::engine::local;
use crate::model::{DotSpec, EngineKind};
use crate::settings::Settings;
use crate::{Error, Result};

/// One model in a picker: the id a dot stores in `spec.model`, the name shown and one line
/// saying what it is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    pub hint: String,
}

/// The Claude models offered, as `(id, label, hint)`. The ids are the `claude` CLI's aliases,
/// passed as `--model`, so they follow the newest model of each family. Update the list here.
pub const CLAUDE_MODELS: &[(&str, &str, &str)] = &[
    (
        "opus",
        "Opus",
        "Most capable, for hard work; slower and uses more of your limits.",
    ),
    (
        "sonnet",
        "Sonnet",
        "Balanced speed and capability; the usual choice.",
    ),
    (
        "haiku",
        "Haiku",
        "Fastest and lightest on your limits, for simple tasks.",
    ),
];

/// The model a new Claude dot starts with when Settings name none.
pub const DEFAULT_CLAUDE_MODEL: &str = "sonnet";

/// Auto, offered first for a Claude dot (auto model spec section 1): a model picked per run.
pub fn auto_option() -> ModelOption {
    ModelOption {
        id: crate::auto_model::AUTO_MODEL.to_string(),
        label: crate::auto_model::AUTO_LABEL.to_string(),
        hint: crate::auto_model::AUTO_HINT.to_string(),
    }
}

/// The curated Claude models, without Auto.
pub fn claude_models() -> Vec<ModelOption> {
    CLAUDE_MODELS
        .iter()
        .map(|(id, label, hint)| ModelOption {
            id: (*id).to_string(),
            label: (*label).to_string(),
            hint: (*hint).to_string(),
        })
        .collect()
}

/// A dot with the core's defaults and the default model of `settings` (model switcher spec
/// section 2.2): what every dot made from the UI starts from.
pub fn new_dot_spec(settings: &Settings, name: &str, instructions: &str, workdir: &str) -> DotSpec {
    let mut spec = DotSpec::new(name, instructions, workdir);
    spec.model = settings.default_model.clone();
    spec
}

/// The models `engine` offers: Auto and the Claude list, or the ids the local `endpoint` lists. A local
/// endpoint is checked as the "Test endpoint" probe checks it; every failure is an error that
/// says what happened.
pub async fn list_models(engine: EngineKind, endpoint: Option<&str>) -> Result<Vec<ModelOption>> {
    match engine {
        EngineKind::Claude => {
            let mut models = vec![auto_option()];
            models.extend(claude_models());
            Ok(models)
        }
        EngineKind::Local => {
            let endpoint = endpoint
                .map(str::trim)
                .filter(|e| !e.is_empty())
                .ok_or_else(|| Error::Invalid("Set the endpoint to list its models.".into()))?;
            let ids = local::list_models(endpoint).await?;
            Ok(ids
                .into_iter()
                .map(|id| ModelOption {
                    label: id.clone(),
                    id,
                    hint: String::new(),
                })
                .collect())
        }
    }
}
