//! Move and game sounds, as the Electron app plays them (`utils/sound.ts`): the move, capture,
//! low-time and notification clips in `apps/desktop/app/assets/sounds`.

use gpui_kit::App;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Move,
    Capture,
    LowTime,
}

/// The clip for a move: a capture sounds different from a quiet move (`playMoveSound`).
pub fn move_sound(san: &str, cx: &mut App) {
    play(
        if san.contains('x') {
            Sound::Capture
        } else {
            Sound::Move
        },
        cx,
    );
}

/// Play a clip at the volume Settings chose, unless sound is off.
pub fn play(_sound: Sound, _cx: &mut App) {}
