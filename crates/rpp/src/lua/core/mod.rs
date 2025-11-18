use include_directory::{include_directory, Dir};

static LIB: Dir<'_> = include_directory!("$CARGO_MANIFEST_DIR/src/lua/core/lib");
