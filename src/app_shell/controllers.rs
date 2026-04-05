use kchess_board::Side;

use crate::computer::ComputerStrength;

#[derive(Debug, Default)]
pub struct SearchControllerState {
    ignore_next_outside_click: bool,
}

impl SearchControllerState {
    pub fn mark_inside_region_click(&mut self) {
        self.ignore_next_outside_click = true;
    }

    pub fn consume_outside_click_guard(&mut self) -> bool {
        if self.ignore_next_outside_click {
            self.ignore_next_outside_click = false;
            return true;
        }
        false
    }

    pub fn reset_outside_click_guard(&mut self) {
        self.ignore_next_outside_click = false;
    }
}

#[derive(Debug, Default)]
pub struct PlayScreenState {
    ignore_next_outside_computer_click: bool,
    computer_dropdown_open: bool,
}

impl PlayScreenState {
    pub fn computer_dropdown_open(&self) -> bool {
        self.computer_dropdown_open
    }

    pub fn toggle_computer_dropdown(&mut self) {
        self.computer_dropdown_open = !self.computer_dropdown_open;
    }

    pub fn close_computer_dropdown(&mut self) -> bool {
        if !self.computer_dropdown_open {
            return false;
        }

        self.computer_dropdown_open = false;
        true
    }

    pub fn mark_inside_computer_region_click(&mut self) {
        self.ignore_next_outside_computer_click = true;
    }

    pub fn consume_outside_computer_click_guard(&mut self) -> bool {
        if self.ignore_next_outside_computer_click {
            self.ignore_next_outside_computer_click = false;
            return true;
        }
        false
    }

    pub fn reset_outside_computer_click_guard(&mut self) {
        self.ignore_next_outside_computer_click = false;
    }
}

#[derive(Clone, Debug, Default)]
pub struct ComputerControllerState {
    strength: Option<ComputerStrength>,
    user_side: Option<Side>,
    thinking: bool,
    request_nonce: u64,
    active_request: Option<(u64, String)>,
    status: Option<String>,
    error: Option<String>,
}

impl ComputerControllerState {
    pub fn strength(&self) -> Option<ComputerStrength> {
        self.strength
    }

    pub fn set_strength(&mut self, strength: Option<ComputerStrength>) {
        self.strength = strength;
    }

    pub fn user_side(&self) -> Option<Side> {
        self.user_side
    }

    pub fn set_user_side(&mut self, side: Option<Side>) {
        self.user_side = side;
    }

    pub fn thinking(&self) -> bool {
        self.thinking
    }

    pub fn set_thinking(&mut self, thinking: bool) {
        self.thinking = thinking;
    }

    pub fn active_request(&self) -> Option<(u64, &str)> {
        self.active_request
            .as_ref()
            .map(|(id, fingerprint)| (*id, fingerprint.as_str()))
    }

    pub fn clear_active_request(&mut self) {
        self.active_request = None;
    }

    pub fn begin_request(&mut self, fingerprint: String) -> u64 {
        self.request_nonce += 1;
        let request_id = self.request_nonce;
        self.active_request = Some((request_id, fingerprint));
        request_id
    }

    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    pub fn set_status(&mut self, value: Option<String>) {
        self.status = value;
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn set_error(&mut self, value: Option<String>) {
        self.error = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_controller_consumes_inside_click_guard_once() {
        let mut controller = SearchControllerState::default();
        controller.mark_inside_region_click();

        assert!(controller.consume_outside_click_guard());
        assert!(!controller.consume_outside_click_guard());
    }

    #[test]
    fn play_screen_dropdown_and_click_guard_behave_as_expected() {
        let mut play_screen = PlayScreenState::default();
        assert!(!play_screen.computer_dropdown_open());

        play_screen.toggle_computer_dropdown();
        assert!(play_screen.computer_dropdown_open());
        assert!(play_screen.close_computer_dropdown());
        assert!(!play_screen.close_computer_dropdown());

        play_screen.mark_inside_computer_region_click();
        assert!(play_screen.consume_outside_computer_click_guard());
        assert!(!play_screen.consume_outside_computer_click_guard());
    }

    #[test]
    fn computer_controller_tracks_request_lifecycle() {
        let mut controller = ComputerControllerState::default();
        let first = controller.begin_request("a".to_string());
        let second = controller.begin_request("b".to_string());

        assert!(second > first);
        assert_eq!(controller.active_request(), Some((second, "b")));

        controller.clear_active_request();
        assert!(controller.active_request().is_none());
    }
}
