#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= (self.x + self.w) && py >= self.y && py <= (self.y + self.h)
    }

    pub fn inset(&self, padding: f32) -> Self {
        Self {
            x: self.x + padding,
            y: self.y + padding,
            w: (self.w - padding * 2.0).max(0.0),
            h: (self.h - padding * 2.0).max(0.0),
        }
    }
}

#[derive(Debug, Clone)]
pub enum UiAction {
    ToggleEntity(String),
    SetClimateTemp(String, f32), // entity_id, delta (+0.5 or -0.5)
    MediaPlayPause(String),
    MediaNext(String),
    MediaPrevious(String),
    ActivateScene(String),
    /// A service of `domain` with JSON `data` payload, e.g. ("cover", "open_cover", {"entity_id": id})
    CallService(String, String, serde_json::Value),
    SwitchPage(usize),
    /// The header's new-version button: show the release notes
    ShowUpdate,
    /// Back to the page from the release notes
    CloseUpdate,
    /// Don't tell about this version again
    SkipUpdate(String),
    Refresh,
    ToggleFrontlight,
    /// The header's eye button: keep the Kobo awake (no sleep, no light auto-off)
    ToggleKeepAwake,
    ExitToReader,
}

#[derive(Debug, Clone)]
pub struct InteractiveTarget {
    pub rect: Rect,
    pub action: UiAction,
}
