//! Defaults for model grants and live-mode reservations (design/09 §0, §3).
//!
//! A `(grant m (model))` with no fields means Claude Sonnet 5 at the prices
//! below. These are *defaults only*: every field can be overridden in the grant,
//! and this file is the one place to change them.
//! Prices are integer micro-dollars per token: $2 per million tokens = 2 µ$/token.

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
