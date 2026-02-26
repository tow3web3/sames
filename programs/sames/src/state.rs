use anchor_lang::prelude::*;

// ─────────────────────────────────────────────────────────────────────────────
// Launch status enum — 3-phase lifecycle
// ─────────────────────────────────────────────────────────────────────────────

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LaunchStatus {
    /// Phase 1: Presale window is open — buyers deposit SOL, get same price.
    Presale,
    /// Phase 2: Bonding curve trading — price floor enforced, can't sell below entry.
    BondingCurve,
    /// Phase 3: Graduated to Raydium — price floor removed, normal token.
    Graduated,
    /// Launch has been closed / cancelled.
    Closed,
}

// ─────────────────────────────────────────────────────────────────────────────
// Bonding curve math
// ─────────────────────────────────────────────────────────────────────────────
// We use a linear bonding curve for simplicity:
//   price = base_price + slope * tokens_sold
//
// For a buy of `amount` tokens starting at `tokens_sold`:
//   cost = integral from tokens_sold to tokens_sold + amount of (base + slope * x) dx
//        = base * amount + slope * (amount * tokens_sold + amount^2 / 2)
//
// This gives increasing price as more tokens are bought.

/// Calculate the cost in lamports to buy `amount` tokens on the bonding curve.
/// Uses integer math with scaling to avoid overflow.
/// base_price and slope are in lamports (slope is lamports per token, scaled by 1e9).
pub fn bonding_curve_cost(
    base_price: u64,
    slope_scaled: u64,  // slope * 1e9 (to handle fractional slopes)
    tokens_sold: u64,
    amount: u64,
) -> Option<u64> {
    // cost = base_price * amount + slope_scaled * amount * (2*tokens_sold + amount) / (2 * 1e9)
    let base_cost = (base_price as u128).checked_mul(amount as u128)?;
    let two_sold_plus_amount = (2u128)
        .checked_mul(tokens_sold as u128)?
        .checked_add(amount as u128)?;
    let slope_cost = (slope_scaled as u128)
        .checked_mul(amount as u128)?
        .checked_mul(two_sold_plus_amount)?
        .checked_div(2_000_000_000u128)?;  // divide by 2 * 1e9
    let total = base_cost.checked_add(slope_cost)?;
    if total > u64::MAX as u128 { return None; }
    Some(total as u64)
}

/// Calculate how many tokens you get for `sol_amount` lamports on the bonding curve.
/// Inverse of bonding_curve_cost using quadratic formula.
pub fn bonding_curve_tokens_for_sol(
    base_price: u64,
    slope_scaled: u64,
    tokens_sold: u64,
    sol_amount: u64,
) -> Option<u64> {
    if slope_scaled == 0 {
        // Linear pricing: tokens = sol_amount / base_price
        return Some(sol_amount.checked_div(base_price)?);
    }
    // Solving: slope_scaled * amount^2 / (2*1e9) + (base_price + slope_scaled * tokens_sold / 1e9) * amount = sol_amount
    // Using quadratic formula: a*x^2 + b*x - c = 0
    // a = slope_scaled / (2 * 1e9)
    // b = base_price + slope_scaled * tokens_sold / 1e9
    // c = sol_amount
    // x = (-b + sqrt(b^2 + 4ac)) / (2a)
    
    // Work in u128 to avoid overflow
    let a_num = slope_scaled as u128;  // numerator, will divide by 2e9 later
    let b = (base_price as u128)
        .checked_add(
            (slope_scaled as u128)
                .checked_mul(tokens_sold as u128)?
                .checked_div(1_000_000_000u128)?
        )?;
    let c = sol_amount as u128;
    
    // discriminant = b^2 + 4 * (a_num / 2e9) * c = b^2 + 2 * a_num * c / 1e9
    let b_squared = b.checked_mul(b)?;
    let four_ac = (2u128)
        .checked_mul(a_num)?
        .checked_mul(c)?
        .checked_div(1_000_000_000u128)?;
    let discriminant = b_squared.checked_add(four_ac)?;
    
    // Integer square root
    let sqrt_disc = isqrt_u128(discriminant);
    
    // amount = (-b + sqrt(disc)) / (2a) = (sqrt(disc) - b) * 1e9 / a_num
    if sqrt_disc <= b { return Some(0); }
    let numerator = (sqrt_disc - b).checked_mul(1_000_000_000u128)?;
    let result = numerator.checked_div(a_num)?;
    
    if result > u64::MAX as u128 { return None; }
    Some(result as u64)
}

/// Calculate the current spot price on the bonding curve.
pub fn bonding_curve_price(base_price: u64, slope_scaled: u64, tokens_sold: u64) -> u64 {
    let slope_component = (slope_scaled as u128)
        .checked_mul(tokens_sold as u128)
        .and_then(|v| v.checked_div(1_000_000_000u128))
        .unwrap_or(0);
    let price = (base_price as u128).saturating_add(slope_component);
    if price > u64::MAX as u128 { u64::MAX } else { price as u64 }
}

/// Integer square root for u128 (Newton's method).
fn isqrt_u128(n: u128) -> u128 {
    if n == 0 { return 0; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

// ─────────────────────────────────────────────────────────────────────────────
// LaunchPool — one per token launch
// ─────────────────────────────────────────────────────────────────────────────

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

    /// Total token supply (in smallest units).
    pub total_supply: u64,

    /// Base price per token in lamports (starting price on bonding curve).
    pub price_lamports: u64,

    /// Bonding curve slope (scaled by 1e9). 0 = flat price.
    pub slope_scaled: u64,

    /// Number of tokens sold on the bonding curve so far.
    pub tokens_sold_curve: u64,

    /// SOL collected in the bonding curve vault (post-presale trading).
    pub curve_sol_collected: u64,

    /// Unix timestamp when the presale window opens.
    pub start_time: i64,

    /// Unix timestamp when the presale window closes (start_time + 30s).
    pub end_time: i64,

    /// Total SOL (lamports) collected during presale.
    pub total_sol_collected: u64,

    /// Number of unique buyers (presale + curve).
    pub buyer_count: u32,

    /// Market cap threshold in lamports for graduation to Raydium.
    /// Default: 69 SOL (69_000_000_000 lamports).
    pub graduation_threshold: u64,

    /// Current status of the launch.
    pub status: LaunchStatus,

    /// Bump seed for this PDA.
    pub bump: u8,

    /// Vault bump (SOL escrow PDA).
    pub vault_bump: u8,

    /// Reserved space for future upgrades.
    pub _reserved: [u8; 64],
}

impl LaunchPool {
    pub const MAX_SIZE: usize = 8  // discriminator
        + 32  // creator
        + 32  // mint
        + 36  // token_name (4 + 32)
        + 14  // token_symbol (4 + 10)
        + 8   // total_supply
        + 8   // price_lamports
        + 8   // slope_scaled
        + 8   // tokens_sold_curve
        + 8   // curve_sol_collected
        + 8   // start_time
        + 8   // end_time
        + 8   // total_sol_collected
        + 4   // buyer_count
        + 8   // graduation_threshold
        + 1   // status (enum)
        + 1   // bump
        + 1   // vault_bump
        + 64; // _reserved

    pub fn is_presale_active(&self, now: i64) -> bool {
        self.status == LaunchStatus::Presale && now >= self.start_time && now < self.end_time
    }

    pub fn is_presale_over(&self, now: i64) -> bool {
        now >= self.end_time
    }

    /// Current market cap = current_price * total_supply (in lamports).
    pub fn market_cap(&self) -> u128 {
        let price = bonding_curve_price(self.price_lamports, self.slope_scaled, self.tokens_sold_curve);
        (price as u128) * (self.total_supply as u128)
    }

    /// Check if the bonding curve has hit graduation threshold.
    pub fn should_graduate(&self) -> bool {
        self.status == LaunchStatus::BondingCurve
            && self.curve_sol_collected >= self.graduation_threshold
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// BuyerRecord — one per buyer per launch
// ─────────────────────────────────────────────────────────────────────────────

#[account]
#[derive(Debug)]
pub struct BuyerRecord {
    /// The launch pool this record belongs to.
    pub launch_pool: Pubkey,

    /// The buyer's wallet address.
    pub buyer: Pubkey,

    /// SOL deposited by this buyer during presale (lamports).
    pub sol_deposited: u64,

    /// Entry price in lamports per token.
    /// For presale buyers: the presale price.
    /// For curve buyers: their average purchase price.
    pub entry_price: u64,

    /// Number of tokens allocated/purchased by this buyer.
    pub tokens_allocated: u64,

    /// Number of tokens this buyer has sold.
    pub tokens_sold: u64,

    /// Total SOL spent on bonding curve buys (for avg price calculation).
    pub curve_sol_spent: u64,

    /// Total tokens bought on bonding curve (for avg price calculation).
    pub curve_tokens_bought: u64,

    /// Bump seed for this PDA.
    pub bump: u8,

    /// Reserved for future use.
    pub _reserved: [u8; 32],
}

impl BuyerRecord {
    pub const MAX_SIZE: usize = 8  // discriminator
        + 32  // launch_pool
        + 32  // buyer
        + 8   // sol_deposited
        + 8   // entry_price
        + 8   // tokens_allocated
        + 8   // tokens_sold
        + 8   // curve_sol_spent
        + 8   // curve_tokens_bought
        + 1   // bump
        + 32; // _reserved

    /// Calculate average entry price across presale + curve buys.
    pub fn average_entry_price(&self) -> u64 {
        let total_sol = self.sol_deposited.saturating_add(self.curve_sol_spent);
        let total_tokens = self.tokens_allocated.saturating_add(self.curve_tokens_bought);
        if total_tokens == 0 { return 0; }
        // avg_price = total_sol / total_tokens
        ((total_sol as u128) * 1_000_000_000 / (total_tokens as u128) / 1_000_000_000) as u64
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// MarketRegistry — whitelisted DEX / market accounts
// ─────────────────────────────────────────────────────────────────────────────

#[account]
#[derive(Debug)]
pub struct MarketRegistry {
    pub launch_pool: Pubkey,
    pub authority: Pubkey,
    pub market_accounts: Vec<Pubkey>,
    pub bump: u8,
}

impl MarketRegistry {
    pub const MAX_MARKETS: usize = 16;
    pub const MAX_SIZE: usize = 8 + 32 + 32 + 4 + (32 * Self::MAX_MARKETS) + 1;
}
