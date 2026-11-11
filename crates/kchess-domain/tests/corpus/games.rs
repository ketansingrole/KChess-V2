//! The game sessions' transitions, and the board clock over its JSON state.

use serde_json::{Value, json};

use kchess_domain::games::otb_clock::{Config, OtbClock, Side};
use kchess_domain::games::{archive, computer, local};

use super::Out;

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    let transition: fn(&[Value]) -> Out = match method {
        "computerState" => computer::state,
        "computerInit" => computer::init,
        "computerView" => computer::view,
        "computerAvailability" => computer::availability,
        "computerExpire" => computer::expire,
        "computerMove" => computer::play,
        "computerSearchStart" => computer::search_start,
        "computerSearchReply" => computer::search_reply,
        "computerSearchFailed" => computer::search_failed,
        "computerSearchEnd" => computer::search_end,
        "computerStart" => computer::start,
        "computerTakeback" => computer::takeback,
        "computerResign" => computer::resign,
        "localState" => local::state,
        "localInit" => local::init,
        "localView" => local::view,
        "localExpire" => local::expire,
        "localMove" => local::play,
        "localTakeback" => local::takeback,
        "localStart" => local::start,
        "localResign" => local::resign,
        "localAgreeDraw" => local::agree_draw,
        "localTogglePause" => local::toggle_pause,
        "archiveState" => archive::state,
        "archiveInit" => archive::init,
        "archiveSave" => archive::save,
        "archiveReset" => archive::reset,
        _ => return otb_clock(method, args),
    };
    Some(transition(args))
}

fn clock_arg(args: &[Value]) -> Result<OtbClock, String> {
    serde_json::from_value(args.first().cloned().unwrap_or(Value::Null))
        .map_err(|cause| format!("clock must be an otbClock state: {cause}"))
}

fn number_arg(args: &[Value], index: usize, name: &str) -> Result<f64, String> {
    args.get(index)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("{name} must be a number"))
}

fn side_arg(args: &[Value], index: usize) -> Result<Side, String> {
    serde_json::from_value(args.get(index).cloned().unwrap_or(Value::Null))
        .map_err(|_| "side must be \"top\" or \"bottom\"".to_string())
}

fn config_arg(value: Option<&Value>) -> Result<Config, String> {
    let value = value.cloned().unwrap_or(Value::Null);
    let number = |key: &str| value[key].as_f64().filter(|n| n.is_finite());
    match (
        number("minutes"),
        number("bottomMinutes"),
        number("increment"),
    ) {
        (Some(minutes), Some(bottom_minutes), Some(increment)) => Ok(Config {
            minutes,
            bottom_minutes,
            increment,
        }),
        _ => Err("config must have minutes, bottomMinutes and increment".into()),
    }
}

/// A press, pause, start or expiry at the time given: the new state, and whether a flag fell.
fn transition(
    args: &[Value],
    change: impl FnOnce(&mut OtbClock) -> Result<bool, String>,
) -> Result<Value, String> {
    let mut clock = clock_arg(args)?;
    let flag_fell = change(&mut clock)?;
    Ok(json!({ "clock": clock, "flagFell": flag_fell }))
}

/// The clock's state in, the new state (and what happened) out.
fn otb_clock(method: &str, args: &[Value]) -> Option<Out> {
    let at = |index| number_arg(args, index, "at");
    Some(match method {
        "otbClockNew" => config_arg(args.first()).map(|config| json!(OtbClock::new(config))),
        "otbClockLeft" => {
            clock_arg(args).and_then(|clock| Ok(json!(clock.left(side_arg(args, 1)?, at(2)?))))
        }
        "otbClockExpire" => transition(args, |clock| Ok(clock.expire(at(1)?))),
        "otbClockPress" => transition(args, |clock| Ok(clock.press(side_arg(args, 1)?, at(2)?))),
        "otbClockPause" => transition(args, |clock| Ok(clock.pause(at(1)?))),
        "otbClockStart" => transition(args, |clock| Ok(clock.start(at(1)?))),
        "otbClockReset" => clock_arg(args).map(|mut clock| {
            clock.reset();
            json!(clock)
        }),
        "otbClockConfigure" => clock_arg(args).and_then(|mut clock| {
            clock.configure(config_arg(args.get(1))?);
            Ok(json!(clock))
        }),
        "otbClockArchive" => clock_arg(args).map(|clock| {
            json!({
                "hasPlay": clock.has_play(),
                "reason": clock.reason(),
                "timeControl": clock.time_control(),
                "clockSummary": clock.summary(),
            })
        }),
        _ => return None,
    })
}
