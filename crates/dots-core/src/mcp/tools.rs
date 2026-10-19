//! A server's tools as Dots keeps them, and their fingerprint (MCP/skills spec section 3).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::util::json_hash;

/// One tool of a `tools/list` result: what Dots shows, checks and sends to a model. The
/// server's `outputSchema`, `_meta` and other fields are not kept. A catalog's tool list is a
/// JSON array of these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// A JSON Schema of the arguments; a tool that sends none takes no arguments.
    #[serde(default = "no_arguments")]
    pub input_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Value>,
}

fn no_arguments() -> Value {
    json!({ "type": "object" })
}

impl McpTool {
    /// The tool says it changes nothing: `annotations.readOnlyHint` is `true`.
    pub fn read_only(&self) -> bool {
        self.annotations
            .as_ref()
            .and_then(|a| a.get("readOnlyHint"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}

/// SHA-256 (hex) of the tools sorted by name, as one canonical JSON array (keys sorted, no
/// whitespace) of `{name, description, inputSchema, annotations}` per tool; a field the server
/// did not send is left out. `title`, `outputSchema` and `_meta` are not part of it.
pub fn fingerprint(tools: &[McpTool]) -> String {
    let mut sorted: Vec<&McpTool> = tools.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    let parts = sorted
        .into_iter()
        .map(|t| {
            let mut part = Map::new();
            part.insert("name".into(), json!(t.name));
            if let Some(description) = &t.description {
                part.insert("description".into(), json!(description));
            }
            part.insert("inputSchema".into(), t.input_schema.clone());
            if let Some(annotations) = &t.annotations {
                part.insert("annotations".into(), annotations.clone());
            }
            Value::Object(part)
        })
        .collect();
    json_hash(&Value::Array(parts))
}
