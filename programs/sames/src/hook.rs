use anchor_lang::prelude::*;

use crate::errors::SamesError;
use crate::state::{BuyerRecord, LaunchPool, LaunchStatus, MarketRegistry};

// ─────────────────────────────────────────────────────────────────────────────
// Transfer Hook — enforces "no sell below entry price"
// ─────────────────────────────────────────────────────────────────────────────
//
// How it works:
// 1. Token-2022 calls our program on every transfer of SAMES tokens.
// 2. We look up the sender's BuyerRecord to get their entry_price.
// 3. We check if the destination is a known DEX/market account.
// 4. If it IS a market account, we derive the implied sell price and compare.
// 5. If sell price < entry price → REJECT the transfer.
// 6. If destination is NOT a market (wallet-to-wallet), we allow it.
//
// Price derivation:
// The implied sell price is passed via the extra_account_metas mechanism.
// In practice, a cranker or the DEX frontend sets the current market price
// in a PriceOracle PDA that we read during the hook.
//
// For V1 we use a simpler model: the LaunchPool tracks a `price_lamports`
// that represents the current market price (updated by an oracle/cranker).
// Transfers to market accounts are blocked if pool price < sender entry price.

/// Accounts required by the transfer hook.
/// These are resolved via the extra-account-metas pattern.
#[derive(Accounts)]
pub struct TransferHook<'info> {
    /// The source token account (sender).
    /// CHECK: validated by Token-2022 before calling hook.
    pub source_account: UncheckedAccount<'info>,

    /// The token mint.
    /// CHECK: validated by Token-2022.
    pub mint: UncheckedAccount<'info>,

    /// The destination token account (receiver).
    /// CHECK: validated by Token-2022.
    pub destination_account: UncheckedAccount<'info>,

    /// The source account's owner/delegate.
    /// CHECK: validated by Token-2022.
    pub owner: UncheckedAccount<'info>,

    /// Extra account meta list PDA (required by spl-transfer-hook-interface).
    /// CHECK: validated by the hook interface.
    #[account(
        seeds = [b"extra-account-metas", mint.key().as_ref()],
        bump,
    )]
    pub extra_account_meta_list: UncheckedAccount<'info>,

    /// The LaunchPool for this token.
    #[account(
        seeds = [b"launch_pool", mint.key().as_ref()],
        bump = launch_pool.bump,
    )]
    pub launch_pool: Account<'info, LaunchPool>,

    /// The sender's BuyerRecord (may not exist if they bought on market).
    /// If it doesn't exist, we allow the transfer (they have no price floor).
    /// CHECK: We try to deserialize; if it fails, transfer is allowed.
    #[account(
        seeds = [b"buyer_record", launch_pool.key().as_ref(), owner.key().as_ref()],
        bump,
    )]
    pub buyer_record: UncheckedAccount<'info>,

    /// MarketRegistry — list of known DEX accounts.
    #[account(
        seeds = [b"market_registry", launch_pool.key().as_ref()],
        bump = market_registry.bump,
    )]
    pub market_registry: Account<'info, MarketRegistry>,
}

/// Execute the transfer hook logic.
///
/// Called by Token-2022 on every transfer. We enforce price floor only
/// when the destination is a known market account AND the sender has a
/// BuyerRecord (original presale participant).
pub fn handler(ctx: Context<TransferHook>, amount: u64) -> Result<()> {
    let launch_pool = &ctx.accounts.launch_pool;
    let market_registry = &ctx.accounts.market_registry;
    let destination = ctx.accounts.destination_account.key();

    // ── 1. Only enforce on live launches ────────────────────────────────
    if launch_pool.status != LaunchStatus::BondingCurve {
        // Presale tokens shouldn't be transferable anyway; Closed = no restrictions
        return Ok(());
    }

    // ── 2. Check if destination is a known market/DEX account ───────────
    let is_market_transfer = market_registry
        .market_accounts
        .iter()
        .any(|m| *m == destination);

    if !is_market_transfer {
        // Wallet-to-wallet transfer — allowed without price check.
        // This means users can send tokens to friends freely.
        return Ok(());
    }

    // ── 3. Try to load sender's BuyerRecord ─────────────────────────────
    let buyer_record_info = &ctx.accounts.buyer_record;

    // If the account doesn't exist or has no data, this person bought on the
    // open market (not in presale) — no price floor applies to them.
    if buyer_record_info.data_is_empty() {
        return Ok(());
    }

    // Deserialize the BuyerRecord.
    let buyer_data = buyer_record_info.try_borrow_data()?;
    // Skip 8-byte Anchor discriminator
    if buyer_data.len() < 8 {
        return Ok(()); // Malformed — allow transfer (fail open for non-presale users)
    }

    let buyer_record = BuyerRecord::try_deserialize(&mut &buyer_data[..])
        .map_err(|_| SamesError::NoBuyerRecord)?;

    // ── 4. Price floor enforcement ──────────────────────────────────────
    // The current "market price" is stored in LaunchPool.price_lamports.
    // In production, this would be fed by an oracle or TWAP.
    // For V1, the creator/cranker updates it.
    let current_price = launch_pool.price_lamports;
    let entry_price = buyer_record.entry_price;

    if current_price < entry_price {
        msg!(
            "SAMES: Transfer BLOCKED. Market price {} < entry price {}",
            current_price,
            entry_price
        );
        return Err(SamesError::HookSellBelowEntry.into());
    }

    // ── 5. Passed all checks — transfer allowed ────────────────────────
    msg!(
        "SAMES: Transfer OK. amount={}, market_price={}, entry_price={}",
        amount,
        current_price,
        entry_price
    );

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Initialize extra-account-metas (called once at launch creation)
// ─────────────────────────────────────────────────────────────────────────────
// This sets up the additional accounts that Token-2022 will pass to our hook.

#[derive(Accounts)]
pub struct InitializeExtraAccountMetaList<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    /// CHECK: The extra-account-metas PDA.
    #[account(
        mut,
        seeds = [b"extra-account-metas", mint.key().as_ref()],
        bump,
    )]
    pub extra_account_meta_list: UncheckedAccount<'info>,

    /// The token mint.
    /// CHECK: validated externally.
    pub mint: UncheckedAccount<'info>,

    /// The launch pool.
    #[account(
        seeds = [b"launch_pool", mint.key().as_ref()],
        bump = launch_pool.bump,
    )]
    pub launch_pool: Account<'info, LaunchPool>,

    /// Market registry.
    #[account(
        seeds = [b"market_registry", launch_pool.key().as_ref()],
        bump = market_registry.bump,
    )]
    pub market_registry: Account<'info, MarketRegistry>,

    pub system_program: Program<'info, System>,
}
