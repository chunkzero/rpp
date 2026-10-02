//! Public types for the WASM component host.

use std::path::PathBuf;
use std::time::Duration;

/// Default per-call epoch deadline.
const DEFAULT_DEADLINE: Duration = Duration::from_secs(60);
/// Default linear-memory cap (512 MiB).
const DEFAULT_MEMORY_LIMIT: usize = 512 * 1024 * 1024;

/// Resource limits applied to every component instance.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum wall-clock time a single guest call may run.
    pub deadline: Duration,
    /// Maximum linear-memory size in bytes.
    pub memory_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            deadline: DEFAULT_DEADLINE,
            memory_bytes: DEFAULT_MEMORY_LIMIT,
        }
    }
}

/// Host directory exposed through WASI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preopen {
    /// Host path.
    pub host: PathBuf,
    /// Path visible to the component.
    pub guest: String,
    /// Whether mutation is allowed.
    pub writable: bool,
}

/// Capabilities available to one component instance.
#[derive(Debug, Clone, Default)]
pub struct Permissions {
    /// Real wall and monotonic clocks instead of fixed ones.
    pub clocks: bool,
    /// Permit secure and insecure random sources.
    pub random: bool,
    /// Permit inherited stdout/stderr.
    pub stdio: bool,
    /// Permit arbitrary sockets.
    pub network: bool,
    /// Exact environment variables exposed to the component.
    pub environment: Vec<(String, String)>,
    /// Scoped filesystem preopens.
    pub preopens: Vec<Preopen>,
}

/// A description of a component value type.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum ValueType {
    Bool,
    S8,
    U8,
    S16,
    U16,
    S32,
    U32,
    S64,
    U64,
    Float32,
    Float64,
    Char,
    String,
    List(Box<ValueType>),
    Record(Vec<(String, ValueType)>),
    Tuple(Vec<ValueType>),
    Variant(Vec<(String, Option<ValueType>)>),
    Enum(Vec<String>),
    Option(Box<ValueType>),
    Result {
        ok: Option<Box<ValueType>>,
        err: Option<Box<ValueType>>,
    },
    Flags(Vec<String>),
    Unsupported(String),
}

/// A dynamic component value used by language runtimes.
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub enum Value {
    Bool(bool),
    S8(i8),
    U8(u8),
    S16(i16),
    U16(u16),
    S32(i32),
    U32(u32),
    S64(i64),
    U64(u64),
    Float32(f32),
    Float64(f64),
    Char(char),
    String(String),
    List(Vec<Value>),
    Record(Vec<(String, Value)>),
    Tuple(Vec<Value>),
    Variant(String, Option<Box<Value>>),
    Enum(String),
    Option(Option<Box<Value>>),
    Result(std::result::Result<Option<Box<Value>>, Option<Box<Value>>>),
    Flags(Vec<String>),
}

/// One callable component export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    /// Export path. Functions nested in exported instances are written as
    /// `instance#function`.
    pub path: String,
    /// Named function parameters.
    pub params: Vec<(String, ValueType)>,
    /// Function result types.
    pub results: Vec<ValueType>,
}

/// Static component interface discovered from the binary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Schema {
    /// Top-level component imports.
    pub imports: Vec<String>,
    /// Recursively flattened function exports.
    pub functions: Vec<Function>,
}
