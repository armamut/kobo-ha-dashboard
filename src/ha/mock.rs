use crate::ha::models::EntityState;
use serde_json::json;
use std::collections::HashMap;

pub fn generate_mock_states() -> HashMap<String, EntityState> {
    let mut map = HashMap::new();

    // 1. Weather
    map.insert(
        "weather.home".to_string(),
        EntityState {
            entity_id: "weather.home".to_string(),
            state: "partlycloudy".to_string(),
            attributes: serde_json::from_value(json!({
                "friendly_name": "Home",
                "temperature": 22.4,
                "humidity": 52,
                "wind_speed": 14.5,
                "pressure": 1014,
                "forecast": [
                    { "datetime": "2026-10-17T12:00:00", "condition": "partlycloudy", "temperature": 23.0, "templow": 15.0 },
                    { "datetime": "2026-10-18T12:00:00", "condition": "sunny", "temperature": 24.5, "templow": 16.0 },
                    { "datetime": "2026-10-19T12:00:00", "condition": "rainy", "temperature": 19.0, "templow": 13.5 },
                    { "datetime": "2026-10-20T12:00:00", "condition": "cloudy", "temperature": 20.0, "templow": 14.0 },
                    { "datetime": "2026-10-21T12:00:00", "condition": "sunny", "temperature": 22.0, "templow": 15.0 }
                ]
            })).unwrap(),
            last_changed: None,
            last_updated: None,
        },
    );

    // 2. Lights
    map.insert(
        "light.ceiling".to_string(),
        EntityState {
            entity_id: "light.ceiling".to_string(),
            state: "on".to_string(),
            attributes: serde_json::from_value(json!({
                "friendly_name": "Ceiling Light",
                "brightness": 204.0, // 80%
                "color_temp_kelvin": 3000
            })).unwrap(),
            last_changed: None,
            last_updated: None,
        },
    );

    map.insert(
        "light.dining_table".to_string(),
        EntityState {
            entity_id: "light.dining_table".to_string(),
            state: "off".to_string(),
            attributes: serde_json::from_value(json!({
                "friendly_name": "Dining Table Lamp",
                "brightness": 128.0,
            })).unwrap(),
            last_changed: None,
            last_updated: None,
        },
    );

    // 3. Climate
    map.insert(
        "climate.living_room".to_string(),
        EntityState {
            entity_id: "climate.living_room".to_string(),
            state: "heat".to_string(),
            attributes: serde_json::from_value(json!({
                "friendly_name": "Thermostat",
                "current_temperature": 21.2,
                "temperature": 22.5,
                "hvac_modes": ["heat", "off", "auto"],
                "hvac_action": "heating"
            })).unwrap(),
            last_changed: None,
            last_updated: None,
        },
    );

    map.insert(
        "climate.lounge".to_string(),
        EntityState {
            entity_id: "climate.lounge".to_string(),
            state: "heat".to_string(),
            attributes: serde_json::from_value(json!({
                "friendly_name": "Living Room Thermostat",
                "current_temperature": 21.6,
                "temperature": 22.5,
                "hvac_action": "heating"
            })).unwrap(),
            last_changed: None,
            last_updated: None,
        },
    );

    map.insert(
        "climate.bedroom".to_string(),
        EntityState {
            entity_id: "climate.bedroom".to_string(),
            state: "off".to_string(),
            attributes: serde_json::from_value(json!({
                "friendly_name": "Bedroom Thermostat",
                "current_temperature": 19.8,
                "temperature": 20.0,
                "hvac_action": "idle"
            })).unwrap(),
            last_changed: None,
            last_updated: None,
        },
    );

    // 4. Sensors
    map.insert(
        "sensor.temperature".to_string(),
        EntityState {
            entity_id: "sensor.temperature".to_string(),
            state: "21.6".to_string(),
            attributes: serde_json::from_value(json!({
                "friendly_name": "Living Room Temperature",
                "unit_of_measurement": "°C"
            })).unwrap(),
            last_changed: None,
            last_updated: None,
        },
    );

    map.insert(
        "sensor.humidity".to_string(),
        EntityState {
            entity_id: "sensor.humidity".to_string(),
            state: "48".to_string(),
            attributes: serde_json::from_value(json!({
                "friendly_name": "Living Room Humidity",
                "unit_of_measurement": "%"
            })).unwrap(),
            last_changed: None,
            last_updated: None,
        },
    );

    map.insert(
        "sensor.co2".to_string(),
        EntityState {
            entity_id: "sensor.co2".to_string(),
            state: "540".to_string(),
            attributes: serde_json::from_value(json!({
                "friendly_name": "Living Room CO2",
                "unit_of_measurement": "ppm"
            })).unwrap(),
            last_changed: None,
            last_updated: None,
        },
    );

    map.insert(
        "sensor.power".to_string(),
        EntityState {
            entity_id: "sensor.power".to_string(),
            state: "432".to_string(),
            attributes: serde_json::from_value(json!({
                "friendly_name": "Power Usage",
                "unit_of_measurement": "W"
            })).unwrap(),
            last_changed: None,
            last_updated: None,
        },
    );

    map.insert(
        "sensor.energy_today".to_string(),
        EntityState {
            entity_id: "sensor.energy_today".to_string(),
            state: "7.84".to_string(),
            attributes: serde_json::from_value(json!({
                "friendly_name": "Energy Today",
                "unit_of_measurement": "kWh",
                "__history": [0.2, 0.4, 0.5, 0.9, 1.6, 2.1, 2.4, 2.6, 3.1, 3.9, 4.6, 5.0, 5.3, 5.9, 6.6, 7.1, 7.5, 7.84]
            })).unwrap(),
            last_changed: None,
            last_updated: None,
        },
    );

    // 5. Switch
    map.insert(
        "switch.air_purifier".to_string(),
        EntityState {
            entity_id: "switch.air_purifier".to_string(),
            state: "on".to_string(),
            attributes: serde_json::from_value(json!({
                "friendly_name": "Air Purifier"
            })).unwrap(),
            last_changed: None,
            last_updated: None,
        },
    );

    // 6. Media Player
    map.insert(
        "media_player.speaker".to_string(),
        EntityState {
            entity_id: "media_player.speaker".to_string(),
            state: "playing".to_string(),
            attributes: serde_json::from_value(json!({
                "friendly_name": "Living Room Speaker",
                "media_title": "Weightless (Ambient Mix)",
                "media_artist": "Marconi Union",
                "volume_level": 0.45
            })).unwrap(),
            last_changed: None,
            last_updated: None,
        },
    );

    map
}
