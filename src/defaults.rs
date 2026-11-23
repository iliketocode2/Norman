//! Defaults for model grants and live-mode reservations (design/09 §0, §3).
//!
//! A `(grant m (model))` with no fields means Claude Sonnet 5 at the prices
//! below. These are *defaults only*: every field can be overridden in the grant,
//! and this file is the one place to change them.
//! Prices are in dollars per million tokens, which is micro-dollars per
//! token: $2 per million = 2 µ$/token. A grant may state them with up to
//! three decimals (`[in 0.1]`), so the interpreter keeps them in nano-dollars.

/// The default model: Claude Sonnet 5.
pub const MODEL_ID: &str = "claude-sonnet-5";
/// µ$ per input token ($2 per million).
pub const IN_PRICE: i64 = 2;
/// µ$ per output token ($10 per million).
pub const OUT_PRICE: i64 = 10;
/// The model's maximum output, in tokens.
pub const CEILING: u64 = 128_000;
/// Thinking allowance, in tokens, added to `max_tokens` and the reservation (decision A).
pub const THINK: u64 = 2_000;

/// When an oracle's input count is an estimate (live `count_tokens`), the
/// reservation uses ⌈n × (1 + PCT/100)⌉ + ABS tokens (design/09 §3).
pub const COUNT_MARGIN_PCT: i64 = 5;
pub const COUNT_MARGIN_ABS: i64 = 32;

/// A cache read costs a tenth of the input price when the grant doesn't say
/// (design/09 §7a). That is the rate for most models, Claude Sonnet 5
/// included; Claude Opus 5.5 and Sonnet 5.5 charge a twentieth, and Claude
/// Fable 5.1 and 5.2 a fortieth, so a grant for one of those states
/// `[cache-read …]`.
pub const CACHE_READ_DIVISOR: i64 = 10;

/// The scripted tokenizer counts an answer type's JSON schema at this rate:
/// 478 tokens for 521 bytes, the first real call's measurement (design/09
/// §6a). The API adds instructions around the schema, so it costs more than
/// a token per byte of prose would suggest. One data point: refine it with
/// `tests/live_real.rs`'s calibration test.
pub const SCHEMA_TOKENS: i64 = 478;
pub const SCHEMA_BYTES: i64 = 521;

/// The shortest prefix a model caches, in tokens. A shorter one silently
/// isn't cached: no write is charged, and nothing is read later. A model this
/// table doesn't know gets the largest minimum, so scripted costs err high.
pub fn min_cacheable(model_id: &str) -> i64 {
    match model_id {
        "claude-opus-5-5" | "claude-opus-5" | "claude-sonnet-5-5" | "claude-haiku-5-5" | "claude-fable-5"
        | "claude-fable-5-1" | "claude-fable-5-2" | "claude-mythos-5" | "claude-mythos-5-1" | "claude-mythos-5-2" => {
            512
        }
        "claude-opus-4-8" | "claude-sonnet-5" | "claude-sonnet-4-6" | "claude-sonnet-4-5" | "claude-opus-4-1"
        | "claude-opus-4" | "claude-sonnet-4" => 1024,
        "claude-opus-4-7" | "claude-haiku-3-5" => 2048,
        _ => 4096,
    }
}
