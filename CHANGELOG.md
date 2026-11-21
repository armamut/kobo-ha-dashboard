# Changelog

## Unreleased

### Added
- **`vacuum` card** ([vacuum_card.rs](src/ui/widgets/vacuum_card.rs)): dedicated card for Home Assistant `vacuum.*` entities showing localized state (`cleaning`, `docked`, `returning`, `paused`, `idle`), battery percentage, and round action buttons (`start_pause`, `return_home`, `start`, `pause`, `stop`, `locate`).
- **Custom service & `data` payloads on `scene` / `scenes_row` buttons**: `SceneItemConfig` and `CardConfig::Scene` now accept optional `service` (e.g. `remote.send_command`) and `data` JSON payloads, backed by `UiAction::CallService(domain, service, Value)`.
- **Custom `state_on` / `state_off` labels**: `SensorItemConfig`, `CardConfig::Sensor`, and `CardConfig::Switch` accept optional `state_on` and `state_off` overrides so binary sensors and switches can display custom status text instead of `device_class` defaults (e.g. *Detected* / *Clear*).
- **Lock toggle support**: tapping a `lock.*` entity now dispatches `lock.unlock` / `lock.lock` based on its current state instead of `homeassistant.toggle`.
- **Lovelace importer**: imports `vacuum.*` and `lock.*` entities as well as `button` cards with `tap_action: perform-action` / `call-service`.
- **Icons & translations**: added `volume_up`, `volume_down`, and `volume_mute` icons, plus `cleaning`, `docked`, and `returning` state translations in English and Russian.
