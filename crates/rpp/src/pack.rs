use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Debug)]
pub struct PackConfig {
    pub pack: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlays: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<Value>,
}
