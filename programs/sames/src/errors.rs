use anchor_lang::prelude::*;

/// Custom error codes for the SAMES protocol.
/// Each variant maps to a unique u32 starting at 6000 (Anchor convention).
#[error_code]
pub enum SamesError {
    // ── Presale window ──────────────────────────────────────────────────
    #[msg("Presale window has not started yet")]
    PresaleNotStarted,

    #[msg("Presale window has already ended")]
    PresaleEnded,

    #[msg("Presale window is still active — cannot finalize yet")]
    PresaleStillActive,

    #[msg("Launch has already been finalized")]
    AlreadyFinalized,

    #[msg("Launch has not been finalized yet")]
    NotFinalized,

    // ── Amount / math ───────────────────────────────────────────────────
    #[msg("Deposit amount must be greater than zero")]
    ZeroDeposit,

    #[msg("Arithmetic overflow")]
    MathOverflow,

    #[msg("Insufficient token balance for this operation")]
    InsufficientBalance,

    #[msg("Sell amount must be greater than zero")]
    ZeroSellAmount,

    // ── Price enforcement ───────────────────────────────────────────────
    #[msg("Sell price is below entry price — transfer blocked")]
    SellBelowEntry,

    #[msg("No buyer record found — cannot verify entry price")]
    NoBuyerRecord,

    // ── Transfer hook ───────────────────────────────────────────────────
    #[msg("Transfer hook: sell price below recorded entry price")]
    HookSellBelowEntry,

    #[msg("Transfer hook: unable to derive price from extra account metas")]
    HookPriceDerivationFailed,

    // ── Authority ───────────────────────────────────────────────────────
    #[msg("Only the launch creator can call this instruction")]
    UnauthorizedCreator,

    #[msg("Invalid mint for this launch pool")]
    InvalidMint,

    #[msg("Invalid market account provided")]
    InvalidMarket,

    #[msg("Supply must be greater than zero")]
    ZeroSupply,

    #[msg("Price per token must be greater than zero")]
    ZeroPrice,

    #[msg("Token name too long (max 32 bytes)")]
    NameTooLong,

    #[msg("Token symbol too long (max 10 bytes)")]
    SymbolTooLong,
}
