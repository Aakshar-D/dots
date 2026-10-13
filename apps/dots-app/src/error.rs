use serde::{Deserialize, Serialize};

/// The error every command returns; the UI branches on `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CmdError {
    pub kind: CmdErrorKind,
    pub message: String,
}

/// Declares [`CmdErrorKind`] and its `ALL` from one list: a command cannot answer with a kind
/// that the contract fixture, and so the UI's tests, does not list.
macro_rules! kinds {
    ($($(#[$doc:meta])* $variant:ident),+ $(,)?) => {
        /// Every `kind` a command can answer with; snake_case in JSON.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum CmdErrorKind {
            $( $(#[$doc])* $variant ),+
        }

        impl CmdErrorKind {
            /// Every kind, in declaration order.
            pub const ALL: &'static [CmdErrorKind] = &[ $( CmdErrorKind::$variant ),+ ];
        }
    };
}

kinds! {
    NotFound,
    Invalid,
    Conflict,
    Internal,
    /// The user declined a native confirmation and nothing was changed.
    Cancelled,
}

impl CmdError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: CmdErrorKind::Internal,
            message: message.into(),
        }
    }

    pub fn cancelled(message: impl Into<String>) -> Self {
        Self {
            kind: CmdErrorKind::Cancelled,
            message: message.into(),
        }
    }
}

impl From<dots_core::Error> for CmdError {
    fn from(e: dots_core::Error) -> Self {
        let kind = match &e {
            dots_core::Error::NotFound(_) => CmdErrorKind::NotFound,
            dots_core::Error::Invalid(_) => CmdErrorKind::Invalid,
            dots_core::Error::Conflict(_) => CmdErrorKind::Conflict,
            _ => CmdErrorKind::Internal,
        };
        // The variant prefix ("invalid input: ") is redundant next to `kind`.
        let message = match e {
            dots_core::Error::NotFound(m)
            | dots_core::Error::Invalid(m)
            | dots_core::Error::Conflict(m) => m,
            other => other.to_string(),
        };
        Self { kind, message }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dots_core::Error;

    #[test]
    fn core_errors_map_to_kinds_and_plain_messages() {
        let cases = [
            (
                Error::NotFound("run x".into()),
                CmdErrorKind::NotFound,
                "run x",
            ),
            (
                Error::Invalid("bad cron".into()),
                CmdErrorKind::Invalid,
                "bad cron",
            ),
            (
                Error::Conflict("busy".into()),
                CmdErrorKind::Conflict,
                "busy",
            ),
            (Error::Other("boom".into()), CmdErrorKind::Internal, "boom"),
        ];
        for (e, kind, message) in cases {
            let c = CmdError::from(e);
            assert_eq!((c.kind, c.message.as_str()), (kind, message));
        }
    }

    /// What answers with each kind. No wildcard arm: a kind nothing produces does not compile.
    fn produced(kind: CmdErrorKind) -> CmdError {
        match kind {
            CmdErrorKind::NotFound => Error::NotFound("x".into()).into(),
            CmdErrorKind::Invalid => Error::Invalid("x".into()).into(),
            CmdErrorKind::Conflict => Error::Conflict("x".into()).into(),
            CmdErrorKind::Internal => CmdError::internal("x"),
            CmdErrorKind::Cancelled => CmdError::cancelled("x"),
        }
    }

    #[test]
    fn every_listed_kind_is_produced_by_something() {
        for &kind in CmdErrorKind::ALL {
            assert_eq!(produced(kind).kind, kind);
        }
    }

    #[test]
    fn serializes_as_kind_and_message() {
        let v = serde_json::to_value(CmdError::internal("x")).unwrap();
        assert_eq!(v, serde_json::json!({"kind": "internal", "message": "x"}));
    }
}
