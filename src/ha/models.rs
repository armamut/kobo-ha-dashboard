use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityState {
    pub entity_id: String,
    pub state: String,
    #[serde(default)]
    pub attributes: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub last_changed: Option<String>,
    #[serde(default)]
    pub last_updated: Option<String>,
}

impl EntityState {
    pub fn is_on(&self) -> bool {
        let s = self.state.to_lowercase();
        s == "on" || s == "playing" || s == "home" || s == "heat" || s == "cool" || s == "open"
    }

    pub fn get_attr_f32(&self, key: &str) -> Option<f32> {
        self.attributes.get(key).and_then(|v| {
            if let Some(f) = v.as_f64() {
                Some(f as f32)
            } else if let Some(s) = v.as_str() {
                s.parse::<f32>().ok()
            } else {
                None
            }
        })
    }

    pub fn get_attr_str<'a>(&'a self, key: &str) -> Option<&'a str> {
        self.attributes.get(key).and_then(|v| v.as_str())
    }

    pub fn get_attr_string(&self, key: &str) -> Option<String> {
        self.attributes.get(key).map(|v| {
            if let Some(s) = v.as_str() {
                s.to_string()
            } else {
                v.to_string()
            }
        })
    }

    pub fn friendly_name(&self) -> String {
        self.get_attr_str("friendly_name")
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.entity_id.clone())
    }

    pub fn brightness_percent(&self) -> Option<u8> {
        self.get_attr_f32("brightness")
            .map(|b| ((b / 255.0) * 100.0).round().clamp(0.0, 100.0) as u8)
    }

    pub fn unit_of_measurement(&self) -> Option<String> {
        self.get_attr_string("unit_of_measurement")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastItem {
    pub datetime: String,
    pub condition: String,
    pub temperature: f32,
    #[serde(default)]
    pub templow: Option<f32>,
    #[serde(default)]
    pub precipitation: Option<f32>,
    #[serde(default)]
    pub precipitation_probability: Option<f32>,
    #[serde(default)]
    pub wind_speed: Option<f32>,
}
