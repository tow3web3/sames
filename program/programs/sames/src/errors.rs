use anchor_lang::prelude::*;

#[error_code]
pub enum SamesError {
    #[msg("Presale window has not started yet")]
    PresaleNotStarted,

    #[msg("Presale window has ended")]
    PresaleEnded,

    #[msg("Presale window is still active, cannot finalize")]
    PresaleStillActive,

    #[msg("Launch has already been finalized")]
    AlreadyFinalized,

    #[msg("Launch has not been finalized yet")]
    NotFinalized,

    #[msg("Cannot sell below your entry price")]
    SellBelowEntry,

    #[msg("Insufficient SOL amount")]
    InsufficientAmount,

    #[msg("Arithmetic overflow")]
    MathOverflow,

    #[msg("Invalid token mint")]
    InvalidMint,

    #[msg("Invalid authority")]
    InvalidAuthority,

    #[msg("Launch pool is not in presale state")]
    NotInPresale,

    #[msg("Buyer record not found")]
    BuyerNotFound,

    #[msg("Maximum buyers reached for this launch")]
    MaxBuyersReached,

    #[msg("Presale window must be between 10 and 120 seconds")]
    InvalidPresaleWindow,

    #[msg("Price must be greater than zero")]
    InvalidPrice,

    #[msg("Supply must be greater than zero")]
    InvalidSupply,
}
