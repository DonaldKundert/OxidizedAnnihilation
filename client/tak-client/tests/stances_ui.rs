//! Shared, Bevy-free stance labels and Classic panel actions.
use tak_client::{feel::action_label, hud};
use tak_commands::FireState;
use tak_input::Action;

#[test]
fn defensive_panel_action_and_all_stance_labels_are_truthful() {
    for (state, label) in [
        (FireState::FireAtWill, "Offensive"),
        (FireState::Defensive, "Defensive"),
        (FireState::HoldFire, "Passive"),
        (FireState::ReturnFire, "Retaliate"),
    ] {
        assert_eq!(hud::stance_name(state), label);
        assert_eq!(action_label(&Action::SetFireState { state }), label);
    }
    for (index, state) in [
        FireState::FireAtWill,
        FireState::Defensive,
        FireState::HoldFire,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            hud::PANEL_ACTIONS[10 + index],
            Action::SetFireState { state }
        );
        assert_eq!(hud::PANEL_NAMES[10 + index], hud::stance_name(state));
    }
}
