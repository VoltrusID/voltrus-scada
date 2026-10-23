use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Message {
    pub payload: Value,
    pub metadata: HashMap<String, String>,
}

impl Message {
    pub fn new(payload: Value) -> Self {
        Self {
            payload,
            metadata: HashMap::new(),
        }
    }
}
