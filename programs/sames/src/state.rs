use anchor_lang::prelude::*;

// ─────────────────────────────────────────────────────────────────────────────
// Launch status enum
// ─────────────────────────────────────────────────────────────────────────────

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LaunchStatus {
    /// Presale window is open — buyers can deposit SOL.
    Presale,
    /// Presale ended, tokens minted, trading is live.
    Live,
    /// Launch has been closed / wound down.
    Closed,
}

// ─────────────────────────────────────────────────────────────────────────────
// LaunchPool — one per token launch
// ─────────────────────────────────────────────────────────────────────────────
// PDA seeds: [b"launch_pool", mint.key().as_ref()]

#[account]
#[derive(Debug)]
pub struct LaunchPool {
    /// The wallet that created this launch (has admin rights).
    pub creator: Pubkey,

    /// SPL Token-2022 mint address for the launched token.
    pub mint: Pubkey,

    /// Human-readable token name (max 32 bytes, UTF-8).
    pub token_name: String,

    /// Token ticker symbol (max 10 bytes, UTF-8).
    pub token_symbol: String,

    /// Total token supply (in smallest units, i.e. raw amount with decimals).
    pub total_supply: u64,

    /// Fixed price per token in lamports during presale.
    /// e.g. 1_000_000 = 0.001 SOL per token.
    pub price_lamports: u64,

    /// Unix timestamp when the presale window opens.
    pub start_time: i64,

    /// Unix timestamp when the presale window closes (start_time + 30s).
    pub end_time: i64,

    /// Total SOL (lamports) collected during presale.
    pub total_sol_collected: u64,

    /// Number of unique buyers.
    pub buyer_count: u32,

    /// Current status of the launch.
    pub status: LaunchStatus,

    /// Bump seed for this PDA.
    pub bump: u8,

    /// Vault bump (SOL escrow PDA).
    pub vault_bump: u8,

    /// Reserved space for future upgrades (128 bytes).
    pub _reserved: [u8; 128],
}

impl LaunchPool {
    /// Account discriminator (8) + all fields.
    /// String fields use 4-byte length prefix + content.
    /// token_name: 4 + 32 = 36
    /// token_symbol: 4 + 10 = 14
    pub const MAX_SIZE: usize = 8  // discriminator
        + 32  // creator
        + 32  // mint
        + 36  // token_name (4 + 32)
        + 14  // token_symbol (4 + 10)
        + 8   // total_supply
        + 8   // price_lamports
        + 8   // start_time
        + 8   // end_time
        + 8   // total_sol_collected
        + 4   // buyer_count
        + 1   // status (enum)
        + 1   // bump
        + 1   // vault_bump
        + 128; // _reserved

    pub fn is_presale_active(&self, now: i64) -> bool {
        self.status == LaunchStatus::Presale && now >= self.start_time && now < self.end_time
    }

    pub fn is_presale_over(&self, now: i64) -> bool {
        now >= self.end_time
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// BuyerRecord — one per buyer per launch
// ─────────────────────────────────────────────────────────────────────────────
// PDA seeds: [b"buyer_record", launch_pool.key().as_ref(), buyer.key().as_ref()]

#[account]
#[derive(Debug)]
pub struct BuyerRecord {
    /// The launch pool this record belongs to.
    pub launch_pool: Pubkey,

    /// The buyer's wallet address.
    pub buyer: Pubkey,

    /// SOL deposited by this buyer (lamports).
    pub sol_deposited: u64,

    /// Entry price in lamports per token (copied from LaunchPool.price_lamports at time of buy).
    pub entry_price: u64,

    /// Number of tokens allocated to this buyer after finalization.
    /// Zero until `finalize_launch` is called.
    pub tokens_allocated: u64,

    /// Number of tokens this buyer has sold (tracked for price floor enforcement).
    pub tokens_sold: u64,

    /// Bump seed for this PDA.
    pub bump: u8,

    /// Reserved for future use.
    pub _reserved: [u8; 64],
}

impl BuyerRecord {
    pub const MAX_SIZE: usize = 8  // discriminator
        + 32  // launch_pool
        + 32  // buyer
        + 8   // sol_deposited
        + 8   // entry_price
        + 8   // tokens_allocated
        + 8   // tokens_sold
        + 1   // bump
        + 64; // _reserved
}

// ─────────────────────────────────────────────────────────────────────────────
// MarketRegistry — whitelisted DEX / market accounts for hook price checking
// ─────────────────────────────────────────────────────────────────────────────
// PDA seeds: [b"market_registry", launch_pool.key().as_ref()]

#[account]
#[derive(Debug)]
pub struct MarketRegistry {
    /// The launch pool this registry belongs to.
    pub launch_pool: Pubkey,

    /// Authority that can add/remove markets (usually the launch creator).
    pub authority: Pubkey,

    /// List of known DEX/market token accounts.
    /// Transfers TO these accounts are treated as sells and price-checked.
    pub market_accounts: Vec<Pubkey>,

    /// Bump seed.
    pub bump: u8,
}

impl MarketRegistry {
    /// Max 16 market accounts.
    pub const MAX_MARKETS: usize = 16;
    pub const MAX_SIZE: usize = 8  // discriminator
        + 32  // launch_pool
        + 32  // authority
        + 4 + (32 * Self::MAX_MARKETS)  // Vec<Pubkey>
        + 1;  // bump
}
