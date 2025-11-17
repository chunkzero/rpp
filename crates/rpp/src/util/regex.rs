use once_cell::sync::Lazy;
use regex::Regex;

pub(crate) static ID_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r#"^[a-zA-Z0-9_-]+$"#).unwrap());

pub(crate) static SEMVER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$"#).unwrap()
});
