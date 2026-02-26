use anchor_lang::prelude::*;

/// Status of a launch pool
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
pub enum LaunchStatus {
    /// Presale window is open — buyers can deposit SOL
    Presale,
    /// Presale ended, tokens distributed, market is live
    Live,
    /// Launch was cancelled
    Cancelled,
}

/// Main state account for each token launch
/// PDA seeds: ["launch", creator.key(), token_mint.key()]
#[account]
pub struct LaunchPool {
    /// Creator/authority of this launch
    pub creator: Pubkey,
    
    /// The token mint (Token-2022 with transfer hook)
    pub token_mint: Pubkey,
    
    /// Fixed price per token in lamports during presale
    pub price_lamports: u64,
    
    /// Total token supply to distribute
    pub total_supply: u64,
    
    /// Unix timestamp when presale window opens
    pub presale_start: i64,
    
    /// Unix timestamp when presale window closes (start + 30s)
    pub presale_end: i64,
    
    /// Total SOL (lamports) collected during presale
    pub total_sol_collected: u64,
    
    /// Number of unique buyers
    pub buyer_count: u32,
    
    /// Current status
    pub status: LaunchStatus,
    
    /// Bump seed for PDA
    pub bump: u8,
    
    /// Reserved for future use
    pub _reserved: [u8; 64],
}

impl LaunchPool {
    pub const SIZE: usize = 8  // discriminator
        + 32  // creator
        + 32  // token_mint
        + 8   // price_lamports
        + 8   // total_supply
        + 8   // presale_start
        + 8   // presale_end
        + 8   // total_sol_collected
        + 4   // buyer_count
        + 1   // status
        + 1   // bump
        + 64; // reserved
}

/// Per-buyer record tracking their entry price and allocation
/// PDA seeds: ["buyer", launch_pool.key(), buyer.key()]
#[account]
pub struct BuyerRecord {
    /// The launch pool this belongs to
    pub launch_pool: Pubkey,
    
    /// The buyer's wallet
    pub buyer: Pubkey,
    
    /// SOL deposited (lamports)
    pub sol_deposited: u64,
    
    /// Entry price in lamports per token (same for all buyers in a launch)
    pub entry_price: u64,
    
    /// Tokens allocated after finalization
    pub tokens_allocated: u64,
    
    /// Tokens remaining (decreases as they sell above entry)
    pub tokens_remaining: u64,
    
    /// Whether this buyer has been finalized (tokens distributed)
    pub finalized: bool,
    
    /// Bump seed for PDA
    pub bump: u8,
}

impl BuyerRecord {
    pub const SIZE: usize = 8  // discriminator
        + 32  // launch_pool
        + 32  // buyer
        + 8   // sol_deposited
        + 8   // entry_price
        + 8   // tokens_allocated
        + 8   // tokens_remaining
        + 1   // finalized
        + 1;  // bump
}
