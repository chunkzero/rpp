//! Clock, randomness and `console` capture for the current load or call.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use deno_core::{op2, JsRuntime, OpState};
use deno_error::JsErrorBox;

use crate::error::{Error, Result};
use crate::model::{Clock, Log, LogLevel};

const MAX_LOG_MESSAGES: usize = 1000;
const MAX_LOG_BYTES: usize = 1024 * 1024;
const MAX_DATE_MS: i64 = 8_640_000_000_000_000;

struct Context {
    /// `None` reads the real clock.
    timestamp: Option<f64>,
    random: u64,
    logs: Vec<Log>,
    log_bytes: usize,
}

pub(crate) fn initialize(runtime: &mut JsRuntime) {
    runtime.op_state().borrow_mut().put(None::<Context>);
}

#[allow(clippy::cast_precision_loss)]
pub(crate) fn begin(runtime: &mut JsRuntime, clock: Clock) -> Result<()> {
    let (timestamp, random) = match clock {
        Clock::Fixed { timestamp_ms, seed } => {
            // Date's range fits within the integers a JS number represents exactly.
            if !(-MAX_DATE_MS..=MAX_DATE_MS).contains(&timestamp_ms) {
                return Err(Error::Invalid(format!(
                    "fixed clock timestamp {timestamp_ms} is outside the Date range"
                )));
            }
            (Some(timestamp_ms as f64), seed)
        }
        Clock::Real => (None, RandomState::new().build_hasher().finish()),
    };
    runtime.op_state().borrow_mut().put(Some(Context {
        timestamp,
        random,
        logs: Vec::new(),
        log_bytes: 0,
    }));
    Ok(())
}

pub(crate) fn end(runtime: &mut JsRuntime) -> Vec<Log> {
    let state = runtime.op_state();
    let mut state = state.borrow_mut();
    state
        .borrow_mut::<Option<Context>>()
        .take()
        .map_or_else(Vec::new, |context| context.logs)
}

fn context(state: &mut OpState) -> std::result::Result<&mut Context, JsErrorBox> {
    state
        .borrow_mut::<Option<Context>>()
        .as_mut()
        .ok_or_else(|| JsErrorBox::generic("time and randomness are unavailable"))
}

#[op2(fast)]
fn op_rpp_now(state: &mut OpState) -> std::result::Result<f64, JsErrorBox> {
    Ok(context(state)?.timestamp.unwrap_or_else(real_now))
}

#[allow(clippy::cast_precision_loss)]
fn real_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_millis() as f64)
}

#[op2(fast)]
#[allow(clippy::cast_precision_loss)]
fn op_rpp_random(state: &mut OpState) -> std::result::Result<f64, JsErrorBox> {
    let context = context(state)?;
    // SplitMix64 with the top 53 bits mapped exactly into [0, 1).
    context.random = context.random.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut value = context.random;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    Ok((value >> 11) as f64 / 9_007_199_254_740_992.0)
}

#[op2(fast)]
fn op_rpp_log(
    state: &mut OpState,
    #[string] level: &str,
    #[string] message: &str,
) -> std::result::Result<(), JsErrorBox> {
    let context = context(state)?;
    if context.logs.len() >= MAX_LOG_MESSAGES || context.log_bytes + message.len() > MAX_LOG_BYTES {
        return Err(JsErrorBox::range_error("console output limit exceeded"));
    }
    let level = match level {
        "debug" => LogLevel::Debug,
        "warn" => LogLevel::Warn,
        "error" => LogLevel::Error,
        _ => LogLevel::Info,
    };
    context.log_bytes += message.len();
    context.logs.push(Log {
        level,
        message: message.into(),
    });
    Ok(())
}

deno_core::extension!(rpp_profile, ops = [op_rpp_now, op_rpp_random, op_rpp_log]);
