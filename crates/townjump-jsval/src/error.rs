// JS: V8 and Node exceptions visible to the user (`TypeError`, `SyntaxError`, `RangeError` with a code).
// `Display` is the exception `.message`; the class name is `name()`.

/// JS exception: class and text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JsError {
    /// `TypeError`, for example `Cannot read properties of null (reading 'port')`.
    #[error("{0}")]
    Type(String),
    /// `SyntaxError` from `JSON.parse`, V8 text.
    #[error("{0}")]
    Syntax(String),
    /// Node `RangeError` with a code, for example `ERR_SOCKET_BAD_PORT`.
    #[error("{message}")]
    Range { code: String, message: String },
}

impl JsError {
    /// Exception class name: `TypeError`, `SyntaxError`, `RangeError`.
    pub fn name(&self) -> &'static str {
        match self {
            JsError::Type(_) => "TypeError",
            JsError::Syntax(_) => "SyntaxError",
            JsError::Range { .. } => "RangeError",
        }
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
