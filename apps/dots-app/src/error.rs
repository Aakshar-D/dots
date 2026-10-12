use serde::{Deserialize, Serialize};

/// The error every command returns; the UI branches on `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CmdError {
    /// `not_found`, `invalid`, `conflict` or `internal`.
    pub kind: String,
    pub message: String,
}

impl CmdError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: "internal".into(),
            message: message.into(),
        }
    }
}

impl From<dots_core::Error> for CmdError {
    fn from(e: dots_core::Error) -> Self {
        let kind = match &e {
            dots_core::Error::NotFound(_) => "not_found",
            dots_core::Error::Invalid(_) => "invalid",
            dots_core::Error::Conflict(_) => "conflict",
            _ => "internal",
        };
        // The variant prefix ("invalid input: ") is redundant next to `kind`.
        let message = match e {
            dots_core::Error::NotFound(m)
            | dots_core::Error::Invalid(m)
            | dots_core::Error::Conflict(m) => m,
            other => other.to_string(),
        };
        Self {
            kind: kind.into(),
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dots_core::Error;

    #[test]
    fn core_errors_map_to_kinds_and_plain_messages() {
        let cases = [
            (Error::NotFound("run x".into()), "not_found", "run x"),
            (Error::Invalid("bad cron".into()), "invalid", "bad cron"),
            (Error::Conflict("busy".into()), "conflict", "busy"),
            (Error::Other("boom".into()), "internal", "boom"),
        ];
        for (e, kind, message) in cases {
            let c = CmdError::from(e);
            assert_eq!((c.kind.as_str(), c.message.as_str()), (kind, message));
        }
    }

    #[test]
    fn serializes_as_kind_and_message() {
        let v = serde_json::to_value(CmdError::internal("x")).unwrap();
        assert_eq!(v, serde_json::json!({"kind": "internal", "message": "x"}));
    }
}
