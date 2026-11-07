//! Auto: a Claude dot whose model is `auto` gets a model picked for each run (auto model spec,
//! `docs/superpowers/specs/2026-11-06-auto-model-design.md`). The pick is made from what the run
//! is, with fixed rules and no extra model call, so the same run always gets the same model and
//! the reason can be shown in a few words.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{Dot, EngineKind, ModelTier, Run, TriggerKind, WorkspaceMode};
use crate::policy::Action;
use crate::settings::Settings;

/// The model id a dot stores for Auto.
pub const AUTO_MODEL: &str = "auto";

/// What the picker shows for Auto.
pub const AUTO_LABEL: &str = "Auto";
pub const AUTO_HINT: &str =
    "Picks Haiku, Sonnet or Opus for each task, to spend less of your limits on easy ones.";

/// A task text longer than this, in characters, is no quick one (`medium`).
pub const MEDIUM_CHARS: usize = 1500;
/// A task text longer than this is a long one (`big`).
pub const BIG_CHARS: usize = 6000;
/// A chat message this short, with nothing else to it, is a quick question.
pub const SHORT_CHARS: usize = 280;
/// This many attached files make a big task.
pub const BIG_ATTACHMENTS: usize = 3;

/// Whether `model` names Auto (any case, spaces around it ignored).
pub fn is_auto(model: &str) -> bool {
    model.trim().eq_ignore_ascii_case(AUTO_MODEL)
}

/// The model of each tier (Settings, Claude tab). The ids are what `--model` takes: the
/// curated aliases of `models::CLAUDE_MODELS` by default, or any id the person types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoModels {
    pub small: String,
    pub standard: String,
    pub large: String,
}

impl Default for AutoModels {
    fn default() -> Self {
        Self {
            small: "haiku".into(),
            standard: "sonnet".into(),
            large: "opus".into(),
        }
    }
}

impl AutoModels {
    pub fn model(&self, tier: ModelTier) -> &str {
        match tier {
            ModelTier::Small => self.small.trim(),
            ModelTier::Standard => self.standard.trim(),
            ModelTier::Large => self.large.trim(),
        }
    }

    /// Each tier names a model, and not Auto itself.
    pub fn validate(&self) -> crate::Result<()> {
        for (name, model) in [
            ("small", &self.small),
            ("standard", &self.standard),
            ("large", &self.large),
        ] {
            let model = model.trim();
            if model.is_empty() {
                return Err(crate::Error::Invalid(format!(
                    "auto_models.{name} must name a model"
                )));
            }
            if is_auto(model) || model.chars().any(char::is_whitespace) {
                return Err(crate::Error::Invalid(format!(
                    "auto_models.{name} must be a model ID such as haiku, sonnet or opus"
                )));
            }
        }
        Ok(())
    }
}

/// The model's name as the person knows it: the curated label of an alias ("Sonnet"), else
/// the id itself.
pub fn model_label(model: &str) -> String {
    crate::models::CLAUDE_MODELS
        .iter()
        .find(|(id, _, _)| id.eq_ignore_ascii_case(model.trim()))
        .map(|(_, label, _)| (*label).to_string())
        .unwrap_or_else(|| model.trim().to_string())
}

/// What the rules read from a run.
#[derive(Debug, Clone, PartialEq)]
pub struct Signals {
    pub trigger: TriggerKind,
    /// The task: the run's message, or the dot's instructions when it has none.
    pub text: String,
    /// Whether `text` is a message (the person's, an app's, a follow-up's, a workflow step's),
    /// not the dot's own instructions. Only a message can name a model.
    pub is_message: bool,
    pub attachments: usize,
    /// The dot may edit files or run commands (its policy does not deny them all).
    pub can_act: bool,
    /// The dot works in a git worktree: code work.
    pub worktree: bool,
    /// The tier the run's thread had: the run it resumes or continues, when Auto picked it.
    pub thread: Option<ModelTier>,
}

/// Auto's pick: the tier and a reason in a few words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub tier: ModelTier,
    pub reason: String,
}

impl Route {
    fn new(tier: ModelTier, reason: impl Into<String>) -> Self {
        Self {
            tier,
            reason: reason.into(),
        }
    }
}

/// Words that ask for hard work. Whole words only, so "plan" never matches "planet".
const HARD: &[&str] = &[
    "build",
    "building",
    "implement",
    "implementing",
    "implementation",
    "refactor",
    "refactoring",
    "debug",
    "debugging",
    "design",
    "designing",
    "architect",
    "architecture",
    "analyse",
    "analyze",
    "analysing",
    "analyzing",
    "analysis",
    "plan",
    "planning",
    "investigate",
    "investigating",
    "migrate",
    "migration",
    "optimise",
    "optimize",
    "optimisation",
    "optimization",
    "rewrite",
    "code",
    "coding",
    "program",
    "programming",
    "algorithm",
    "bug",
    "bugs",
    "diagnose",
    "research",
    "prove",
    "proof",
    "strategy",
    "audit",
    "review",
];

/// Words that ask for light work.
const LIGHT: &[&str] = &[
    "summarise",
    "summarize",
    "summary",
    "tldr",
    "check",
    "remind",
    "reminder",
    "list",
    "translate",
    "rephrase",
    "reword",
    "notify",
    "ping",
    "status",
    "count",
    "hi",
    "hello",
    "hey",
    "thanks",
    "thank",
    "weather",
    "define",
    "spell",
    "lookup",
];

/// The trigger is a background check nobody waits on: it leans small.
pub fn is_background(trigger: TriggerKind) -> bool {
    matches!(
        trigger,
        TriggerKind::Schedule | TriggerKind::Watcher | TriggerKind::Webhook
    )
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// The tier a message names with "use opus", "use sonnet" or "use haiku".
fn asked_for(words: &[String]) -> Option<(ModelTier, &'static str)> {
    words.windows(2).find_map(|pair| {
        if pair[0] != "use" {
            return None;
        }
        match pair[1].as_str() {
            "opus" => Some((ModelTier::Large, "Opus")),
            "sonnet" => Some((ModelTier::Standard, "Sonnet")),
            "haiku" => Some((ModelTier::Small, "Haiku")),
            _ => None,
        }
    })
}

/// The pick for `signals`, by these rules in order (auto model spec section 2):
///
/// 1. A message that says "use opus", "use sonnet" or "use haiku" gets that.
/// 2. A resume keeps the tier of the run it resumes.
/// 3. Hard words: Large when the task is big, or is code work (a worktree dot) with two hard
///    words or more than a quick text; else Standard.
/// 4. A big task without hard words: Standard.
/// 5. Light words in a short task: Small, even for a dot that can act.
/// 6. A short background run, or a quick chat question, of a dot that cannot edit files or run
///    commands: Small.
/// 7. Everything else: Standard ("can edit files or run commands", "has attachments", "long
///    message" or "general task").
///
/// In a thread, the tier the thread had stays unless rule 1 applies, or rules 3 and 4 pick a
/// larger tier: a conversation never moves to a smaller model on its own.
pub fn route(signals: &Signals) -> Route {
    let words = words(&signals.text);
    if signals.is_message {
        if let Some((tier, name)) = asked_for(&words) {
            return Route::new(tier, format!("you asked for {name}"));
        }
    }
    if signals.trigger == TriggerKind::Resume {
        if let Some(tier) = signals.thread {
            return Route::new(tier, "continues the run it resumes");
        }
    }
    let fresh = route_fresh(signals, &words);
    match signals.thread {
        Some(thread) if fresh.tier.rank() > thread.rank() && fresh.escalates => {
            Route::new(fresh.tier, format!("escalated: {}", fresh.route.reason))
        }
        Some(thread) => Route::new(thread, "kept from earlier in this conversation"),
        None => fresh.route,
    }
}

struct Fresh {
    tier: ModelTier,
    route: Route,
    /// The pick comes from the task itself (hard words, size), strong enough to move a thread
    /// up.
    escalates: bool,
}

fn route_fresh(signals: &Signals, words: &[String]) -> Fresh {
    let mut hard: Vec<&str> = Vec::new();
    for w in words {
        if let Some(h) = HARD.iter().find(|h| **h == w.as_str()) {
            if !hard.contains(h) {
                hard.push(h);
            }
        }
    }
    let light = words
        .iter()
        .find_map(|w| LIGHT.iter().find(|l| **l == w.as_str()));
    let len = signals.text.chars().count();
    let medium = len > MEDIUM_CHARS || signals.attachments > 0;
    let big = len > BIG_CHARS || signals.attachments >= BIG_ATTACHMENTS;
    let pick = |tier: ModelTier, reason: String, escalates: bool| Fresh {
        tier,
        route: Route::new(tier, reason),
        escalates,
    };
    if !hard.is_empty() {
        let named = hard.iter().take(2).copied().collect::<Vec<_>>().join(", ");
        if big {
            return pick(ModelTier::Large, format!("long, hard task: {named}"), true);
        }
        if signals.worktree && (medium || hard.len() >= 2) {
            return pick(ModelTier::Large, format!("code work: {named}"), true);
        }
        return pick(ModelTier::Standard, format!("hard task: {named}"), true);
    }
    if big {
        let reason = if signals.attachments >= BIG_ATTACHMENTS {
            "many attachments"
        } else {
            "long task"
        };
        return pick(ModelTier::Standard, reason.into(), true);
    }
    if let Some(light) = light {
        if !medium {
            return pick(ModelTier::Small, format!("light task: {light}"), false);
        }
    }
    let quick_chat = !is_background(signals.trigger) && signals.is_message && len <= SHORT_CHARS;
    if !medium && !signals.can_act {
        if is_background(signals.trigger) {
            return pick(ModelTier::Small, "background check".into(), false);
        }
        if quick_chat {
            return pick(ModelTier::Small, "short question".into(), false);
        }
    }
    let reason = if signals.attachments > 0 {
        "has attachments"
    } else if medium {
        "long message"
    } else if signals.can_act {
        "can edit files or run commands"
    } else {
        "general task"
    };
    pick(ModelTier::Standard, reason.into(), false)
}

/// The run's message: what the person, an app or a follow-up wrote, or the section a workflow
/// wrote for its step.
fn message_of(run: &Run) -> Option<String> {
    let payload = run.payload.as_ref()?;
    let text = payload
        .get("message")
        .or_else(|| payload.get("answer"))
        .and_then(Value::as_str)
        .or_else(|| payload.pointer("/workflow/section").and_then(Value::as_str))?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// The policy lets the dot edit a file or run a command, with or without asking.
fn can_act(dot: &Dot) -> bool {
    let policy = &dot.spec.policy;
    let file = serde_json::json!({ "file_path": "notes.txt" });
    let command = serde_json::json!({ "command": "echo hi" });
    let mut acts = false;
    for (tool, input) in [
        ("Edit", &file),
        ("Write", &file),
        ("Bash", &command),
        ("PowerShell", &command),
    ] {
        acts |= policy.resolve(tool, input) != Action::Deny;
    }
    acts
}

/// The signals of `run` of `dot`, with the tier its thread had.
pub fn signals(dot: &Dot, run: &Run, thread: Option<ModelTier>) -> Signals {
    let message = message_of(run);
    let attachments = run
        .payload
        .as_ref()
        .and_then(|p| p.get("attachments"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    Signals {
        trigger: run.trigger,
        is_message: message.is_some(),
        text: message.unwrap_or_else(|| dot.spec.instructions.clone()),
        attachments,
        can_act: can_act(dot),
        worktree: dot.spec.workspace_mode == WorkspaceMode::Worktree,
        thread,
    }
}

/// The model a run is started with, and Auto's pick when Auto made it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    pub model: String,
    pub auto: Option<Route>,
}

/// The model for `run`: the dot's own, or Auto's pick mapped by `settings` for a Claude dot
/// set to Auto.
pub fn choose(dot: &Dot, run: &Run, thread: Option<ModelTier>, settings: &Settings) -> ModelChoice {
    if dot.spec.engine != EngineKind::Claude || !is_auto(&dot.spec.model) {
        return ModelChoice {
            model: dot.spec.model.clone(),
            auto: None,
        };
    }
    let route = route(&signals(dot, run, thread));
    ModelChoice {
        model: settings.auto_models.model(route.tier).to_string(),
        auto: Some(route),
    }
}

/// The tier tried when the CLI could not use the model of `tier`: the next one down, or
/// Standard after Small.
pub fn fallback_tier(tier: ModelTier) -> ModelTier {
    match tier {
        ModelTier::Large => ModelTier::Standard,
        ModelTier::Standard => ModelTier::Small,
        ModelTier::Small => ModelTier::Standard,
    }
}

/// Whether a run's error says the CLI could not use the model it was given: unknown, not
/// allowed for the account, or overloaded (auto model spec section 4). Usage limits are not
/// among them: the hold handles those, and a smaller model would be refused too.
pub fn model_unavailable(error: &str) -> bool {
    let e = error.to_lowercase();
    if e.contains("usage limit") || e.contains("rate limit") || e.contains("rate_limit") {
        return false;
    }
    if e.contains("issue with the selected model")
        || e.contains("overloaded_error")
        || e.contains("overloaded")
        || e.contains("invalid model")
        || e.contains("unknown model")
    {
        return true;
    }
    let names_model =
        e.contains("model") || ["opus", "sonnet", "haiku"].iter().any(|m| e.contains(m));
    names_model
        && [
            "not_found_error",
            "model not found",
            "not available",
            "unavailable",
            "does not exist",
            "not have access",
            "no access",
            "not allowed",
            "not supported",
            "permission_error",
        ]
        .iter()
        .any(|p| e.contains(p))
}

/// The words shown beside a reply: "Auto · Sonnet — code work: refactor".
pub fn label(model: &str, reason: &str) -> String {
    format!("{AUTO_LABEL} · {} — {reason}", model_label(model))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(trigger: TriggerKind, text: &str) -> Signals {
        Signals {
            trigger,
            text: text.into(),
            is_message: true,
            attachments: 0,
            can_act: true,
            worktree: false,
            thread: None,
        }
    }

    #[test]
    fn words_match_whole_words_only() {
        let r = route(&Signals {
            can_act: false,
            ..sig(TriggerKind::Chat, "Name a planet")
        });
        assert_eq!(r.tier, ModelTier::Small, "{r:?}");
    }

    #[test]
    fn a_short_override_in_another_word_does_not_count() {
        let r = route(&sig(TriggerKind::Chat, "I misuse opus in my poems"));
        assert_eq!(r.tier, ModelTier::Standard, "{r:?}");
    }

    #[test]
    fn labels_use_the_curated_names() {
        assert_eq!(label("sonnet", "code work"), "Auto · Sonnet — code work");
        assert_eq!(
            label("claude-x-1", "long task"),
            "Auto · claude-x-1 — long task"
        );
    }
}
