#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleState { Disabled, Active, Blocked }

pub fn decide(enabled: bool, blocked: bool) -> ToggleState {
    if !enabled { ToggleState::Disabled }
    else if blocked { ToggleState::Blocked }
    else { ToggleState::Active }
}
