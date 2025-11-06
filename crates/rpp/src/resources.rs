use include_directory::{include_directory, Dir};

pub static RESOURCES: Dir<'_> = include_directory!("$CARGO_MANIFEST_DIR/resources");

pub static VERSION: &str = env!("CARGO_PKG_VERSION");
