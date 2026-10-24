//! Apps (app library spec): small programs inside Dots with their own data, one screen a person
//! uses and tools a dot calls.

pub mod call;
pub mod data;
pub mod engine;
pub mod manifest;
mod registry;
pub mod run;
pub mod schema;
pub mod source;

pub use registry::{AppCall, Apps, APPS_DIR};
pub use run::{RunApp, RunApps};

/// The policy name of every app tool (spec section 6): a rule `App`, `App(<app>)`,
/// `App(<app>.<tool>)` or `App(<app>.write)`.
pub const APP_POLICY: &str = "App";
/// The policy name of `propose_app` (spec section 6.2). A rule names it alone.
pub const PROPOSE_POLICY: &str = "ProposeApp";
/// What `App(<app>.write)` names: the app's tools that write.
pub const WRITES: &str = "write";

/// The input the policy checks an app call against: the app, the tool, whether it writes, and
/// the tool's own input (spec section 6).
pub fn policy_input(
    app: &str,
    tool: &str,
    writes: bool,
    input: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({ "app": app, "tool": tool, "writes": writes, "input": input })
}

/// A rule's argument for `App`: `<app>`, `<app>.<tool>` or `<app>.write`. `Err` is the sentence
/// for the person.
pub fn check_rule_spec(spec: &str) -> std::result::Result<(), String> {
    let refuse = || {
        "an App rule names an app, an app's tool (projects.get_task) or its writing tools \
         (projects.write)"
            .to_string()
    };
    let (app, tool) = match spec.trim().split_once('.') {
        Some((app, tool)) => (app, Some(tool)),
        None => (spec.trim(), None),
    };
    manifest::check_app_name(app).map_err(|_| refuse())?;
    if let Some(tool) = tool {
        if tool != WRITES {
            manifest::check_tool_name(tool).map_err(|_| refuse())?;
        }
    }
    Ok(())
}

/// Whether the rule argument `spec` covers the app call whose policy input is `input`; `None`
/// for a call without its `app`, `tool` or `writes` (deny and ask rules then match, allow rules
/// do not).
pub fn rule_spec_matches(spec: &str, input: &serde_json::Value) -> Option<bool> {
    let app = input.get("app")?.as_str()?;
    let tool = input.get("tool")?.as_str()?;
    let writes = input.get("writes")?.as_bool()?;
    Some(match spec.trim().split_once('.') {
        None => spec.trim() == app,
        Some((a, WRITES)) => a == app && writes,
        Some((a, t)) => a == app && t == tool,
    })
}
