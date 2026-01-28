use crate::build::BuildError;
use serde_json::Value;

/// JSON encoding/decoding API for plugins.
pub struct JsonApi;

impl JsonApi {
    pub fn new() -> Self {
        Self
    }

    /// Decode JSON bytes to a Value.
    pub fn decode(&self, data: &[u8]) -> Result<Value, BuildError> {
        serde_json::from_slice(data)
            .map_err(|e| BuildError::Plugin(format!("JSON decode error: {}", e)))
    }

    /// Decode JSON string to a Value.
    pub fn decode_str(&self, s: &str) -> Result<Value, BuildError> {
        serde_json::from_str(s).map_err(|e| BuildError::Plugin(format!("JSON decode error: {}", e)))
    }

    /// Encode a Value to pretty-printed JSON bytes.
    pub fn encode(&self, value: &Value) -> Result<Vec<u8>, BuildError> {
        serde_json::to_vec_pretty(value)
            .map_err(|e| BuildError::Plugin(format!("JSON encode error: {}", e)))
    }

    /// Encode a Value to compact JSON bytes.
    pub fn encode_compact(&self, value: &Value) -> Result<Vec<u8>, BuildError> {
        serde_json::to_vec(value)
            .map_err(|e| BuildError::Plugin(format!("JSON encode error: {}", e)))
    }

    /// Encode a Value to a pretty JSON string.
    pub fn encode_string(&self, value: &Value) -> Result<String, BuildError> {
        serde_json::to_string_pretty(value)
            .map_err(|e| BuildError::Plugin(format!("JSON encode error: {}", e)))
    }

    /// Encode a Value to a compact JSON string.
    pub fn encode_string_compact(&self, value: &Value) -> Result<String, BuildError> {
        serde_json::to_string(value)
            .map_err(|e| BuildError::Plugin(format!("JSON encode error: {}", e)))
    }
}

impl Default for JsonApi {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_roundtrip() {
        let api = JsonApi::new();
        let original = r#"{"key": "value", "num": 42}"#;
        let value = api.decode_str(original).unwrap();
        let encoded = api.encode_string_compact(&value).unwrap();
        assert_eq!(encoded, r#"{"key":"value","num":42}"#);
    }
}
