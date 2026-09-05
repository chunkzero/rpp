//! The `file` userdata passed to processors (spec §4).

use std::sync::Arc;

use mlua::{MetaMethod, UserData, UserDataFields, UserDataMethods};
use parking_lot::Mutex;

use crate::util::path::validate_relative;

/// The mutable state behind a `file` userdata.
///
/// Shared via `Rc<RefCell<_>>` so the Rust side can read back the final state
/// after the Lua processor function returns.
#[derive(Debug, Clone)]
pub(crate) struct FileState {
    /// Current output path (relative, forward-slash).
    pub path: String,
    /// Current contents.
    pub contents: Vec<u8>,
    /// Whether the path or contents were mutated.
    pub modified: bool,
    /// Whether `file:drop()` was called.
    pub dropped: bool,
}

impl FileState {
    pub(crate) fn new(path: String, contents: Vec<u8>) -> Self {
        Self {
            path,
            contents,
            modified: false,
            dropped: false,
        }
    }
}

/// A handle to a [`FileState`] exposed to Lua as the `file` userdata.
#[derive(Clone)]
pub(crate) struct FileHandle(pub(crate) Arc<Mutex<FileState>>);

impl UserData for FileHandle {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("path", |_, this| Ok(this.0.lock().path.clone()));
        fields.add_field_method_set("path", |_, this, value: mlua::String| {
            let mut state = this.0.lock();
            let new = value.to_str()?.to_string();
            validate_relative(&new).map_err(mlua::Error::external)?;
            if new != state.path {
                state.path = new;
                state.modified = true;
            }
            Ok(())
        });

        fields.add_field_method_get("bytes", |lua, this| {
            lua.create_string(&this.0.lock().contents)
        });
        fields.add_field_method_set("bytes", |_, this, value: mlua::String| {
            set_contents(this, &value);
            Ok(())
        });

        // `text` is an alias for `bytes`; both are Lua strings.
        fields.add_field_method_get("text", |lua, this| {
            lua.create_string(&this.0.lock().contents)
        });
        fields.add_field_method_set("text", |_, this, value: mlua::String| {
            set_contents(this, &value);
            Ok(())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("drop", |_, this, ()| {
            let mut state = this.0.lock();
            state.dropped = true;
            state.modified = true;
            Ok(())
        });

        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!("file<{}>", this.0.lock().path))
        });
    }
}

fn set_contents(this: &FileHandle, value: &mlua::String) {
    let mut state = this.0.lock();
    let new = value.as_bytes().to_vec();
    if new != state.contents {
        state.contents = new;
        state.modified = true;
    }
}
