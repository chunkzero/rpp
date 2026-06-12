//! Rendering of mlua errors into human-readable messages with tracebacks.

/// Render an mlua error, including a Lua traceback when present.
pub(crate) fn render(err: &mlua::Error) -> String {
    match err {
        mlua::Error::CallbackError { traceback, cause } => {
            format!("{}\n{traceback}", render(cause))
        }
        mlua::Error::RuntimeError(msg) => msg.clone(),
        other => other.to_string(),
    }
}
