use anchor_lang::prelude::*;

#[error_code]
pub enum SamesError {
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
    #[msg("Deposit amount must be greater than zero")]
    ZeroDeposit,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("Insufficient token balance")]
    InsufficientBalance,
    #[msg("Sell amount must be greater than zero")]
    ZeroSellAmount,
    #[msg("Sell price is below entry price — blocked by price floor")]
    SellBelowEntry,
    #[msg("No buyer record found")]
    NoBuyerRecord,
    #[msg("Transfer hook: sell price below entry")]
    HookSellBelowEntry,
    #[msg("Transfer hook: price derivation failed")]
    HookPriceDerivationFailed,
    #[msg("Only the launch creator can call this")]
    UnauthorizedCreator,
    #[msg("Invalid mint for this launch pool")]
    InvalidMint,
    #[msg("Invalid market account")]
    InvalidMarket,
    #[msg("Supply must be greater than zero")]
    ZeroSupply,
    #[msg("Price must be greater than zero")]
    ZeroPrice,
    #[msg("Token name too long (max 32 bytes)")]
    NameTooLong,
    #[msg("Token symbol too long (max 10 bytes)")]
    SymbolTooLong,
    #[msg("Not in bonding curve phase")]
    NotBondingCurve,
    #[msg("Graduation threshold not reached yet")]
    NotReadyToGraduate,
}
