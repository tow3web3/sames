use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token_2022::{self, Token2022};
use anchor_spl::token_interface::{Mint as MintAccount, TokenAccount};

pub mod errors;
pub mod state;
pub mod hook;

use errors::SamesError;
use state::*;

declare_id!("H91AKWdUASAKjpGwq4NXzp8kyddLbZMj9N1rP8HFjCmw");

/// Presale window duration in seconds.
const PRESALE_DURATION: i64 = 30;

/// Default graduation threshold: 69 SOL.
const DEFAULT_GRADUATION_THRESHOLD: u64 = 69_000_000_000;

/// Default bonding curve slope (scaled by 1e9).
/// With base_price=1000 lamports and slope=100, price doubles after 10M tokens sold.
const DEFAULT_SLOPE: u64 = 100;

/// Platform fee: 1% (in basis points = 100).
const PLATFORM_FEE_BPS: u64 = 100;

#[program]
pub mod sames {
    use super::*;

    // ═════════════════════════════════════════════════════════════════════
    // 1. CREATE LAUNCH
    // ═════════════════════════════════════════════════════════════════════
    pub fn create_launch(
        ctx: Context<CreateLaunch>,
        token_name: String,
        token_symbol: String,
        total_supply: u64,
        price_lamports: u64,
    ) -> Result<()> {
        require!(token_name.len() <= 32, SamesError::NameTooLong);
        require!(token_symbol.len() <= 10, SamesError::SymbolTooLong);
        require!(total_supply > 0, SamesError::ZeroSupply);
        require!(price_lamports > 0, SamesError::ZeroPrice);

        let clock = Clock::get()?;
        let now = clock.unix_timestamp;

        let pool = &mut ctx.accounts.launch_pool;
        pool.creator = ctx.accounts.creator.key();
        pool.mint = ctx.accounts.mint.key();
        pool.token_name = token_name;
        pool.token_symbol = token_symbol;
        pool.total_supply = total_supply;
        pool.price_lamports = price_lamports;
        pool.slope_scaled = DEFAULT_SLOPE;
        pool.tokens_sold_curve = 0;
        pool.curve_sol_collected = 0;
        pool.start_time = now;
        pool.end_time = now
            .checked_add(PRESALE_DURATION)
            .ok_or(SamesError::MathOverflow)?;
        pool.total_sol_collected = 0;
        pool.buyer_count = 0;
        pool.graduation_threshold = DEFAULT_GRADUATION_THRESHOLD;
        pool.status = LaunchStatus::Presale;
        pool.bump = ctx.bumps.launch_pool;
        pool.vault_bump = ctx.bumps.vault;
        pool._reserved = [0u8; 64];

        let registry = &mut ctx.accounts.market_registry;
        registry.launch_pool = pool.key();
        registry.authority = ctx.accounts.creator.key();
        registry.market_accounts = Vec::new();
        registry.bump = ctx.bumps.market_registry;

        msg!("SAMES: Launch created. Presale {} to {}", pool.start_time, pool.end_time);
        Ok(())
    }

    // ═════════════════════════════════════════════════════════════════════
    // 2. BUY PRESALE (Phase 1)
    // ═════════════════════════════════════════════════════════════════════
    pub fn buy_presale(ctx: Context<BuyPresale>, sol_amount: u64) -> Result<()> {
        require!(sol_amount > 0, SamesError::ZeroDeposit);

        let clock = Clock::get()?;
        let now = clock.unix_timestamp;
        let pool = &mut ctx.accounts.launch_pool;

        require!(now >= pool.start_time, SamesError::PresaleNotStarted);
        require!(now < pool.end_time, SamesError::PresaleEnded);
        require!(pool.status == LaunchStatus::Presale, SamesError::AlreadyFinalized);

        // Transfer SOL to vault
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.buyer.to_account_info(),
                    to: ctx.accounts.vault.to_account_info(),
                },
            ),
            sol_amount,
        )?;

        pool.total_sol_collected = pool.total_sol_collected
            .checked_add(sol_amount).ok_or(SamesError::MathOverflow)?;

        let record = &mut ctx.accounts.buyer_record;
        if record.sol_deposited == 0 && record.curve_sol_spent == 0 {
            record.launch_pool = pool.key();
            record.buyer = ctx.accounts.buyer.key();
            record.entry_price = pool.price_lamports;
            record.tokens_allocated = 0;
            record.tokens_sold = 0;
            record.curve_sol_spent = 0;
            record.curve_tokens_bought = 0;
            record.bump = ctx.bumps.buyer_record;
            record._reserved = [0u8; 32];
            pool.buyer_count = pool.buyer_count.checked_add(1).ok_or(SamesError::MathOverflow)?;
        }

        record.sol_deposited = record.sol_deposited
            .checked_add(sol_amount).ok_or(SamesError::MathOverflow)?;

        msg!("SAMES: Presale buy {} lamports by {}", sol_amount, ctx.accounts.buyer.key());
        Ok(())
    }

    // ═════════════════════════════════════════════════════════════════════
    // 3. FINALIZE LAUNCH (Presale → BondingCurve)
    // ═════════════════════════════════════════════════════════════════════
    pub fn finalize_launch(ctx: Context<FinalizeLaunch>) -> Result<()> {
        let clock = Clock::get()?;
        let now = clock.unix_timestamp;
        let pool = &ctx.accounts.launch_pool;

        require!(pool.creator == ctx.accounts.creator.key(), SamesError::UnauthorizedCreator);
        require!(pool.is_presale_over(now), SamesError::PresaleStillActive);
        require!(pool.status == LaunchStatus::Presale, SamesError::AlreadyFinalized);

        // Calculate this buyer's token allocation
        let record = &mut ctx.accounts.buyer_record;
        require!(record.sol_deposited > 0, SamesError::ZeroDeposit);

        let tokens = (record.sol_deposited as u128)
            .checked_mul(pool.total_supply as u128)
            .ok_or(SamesError::MathOverflow)?
            .checked_div(pool.total_sol_collected as u128)
            .ok_or(SamesError::MathOverflow)? as u64;

        record.tokens_allocated = tokens;
        record.entry_price = pool.price_lamports;

        // Mint tokens to buyer
        let mint_key = pool.mint;
        let pool_seeds: &[&[u8]] = &[b"launch_pool", mint_key.as_ref(), &[pool.bump]];

        token_2022::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token_2022::MintTo {
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.buyer_token_account.to_account_info(),
                    authority: ctx.accounts.launch_pool.to_account_info(),
                },
                &[pool_seeds],
            ),
            tokens,
        )?;

        msg!("SAMES: Allocated {} tokens to {}", tokens, record.buyer);
        Ok(())
    }

    // ═════════════════════════════════════════════════════════════════════
    // 3b. SET BONDING CURVE LIVE
    // ═════════════════════════════════════════════════════════════════════
    pub fn start_bonding_curve(ctx: Context<StartBondingCurve>) -> Result<()> {
        let pool = &mut ctx.accounts.launch_pool;

        require!(pool.creator == ctx.accounts.creator.key(), SamesError::UnauthorizedCreator);
        require!(pool.status == LaunchStatus::Presale, SamesError::AlreadyFinalized);

        let clock = Clock::get()?;
        require!(pool.is_presale_over(clock.unix_timestamp), SamesError::PresaleStillActive);

        pool.status = LaunchStatus::BondingCurve;
        // Set the base price for the curve based on presale price
        // The curve starts where the presale ended
        msg!("SAMES: Bonding curve LIVE. Base price: {} lamports", pool.price_lamports);
        Ok(())
    }

    // ═════════════════════════════════════════════════════════════════════
    // 4. BUY ON BONDING CURVE (Phase 2)
    // ═════════════════════════════════════════════════════════════════════
    pub fn buy_curve(ctx: Context<BuyCurve>, sol_amount: u64) -> Result<()> {
        require!(sol_amount > 0, SamesError::ZeroDeposit);

        // Read values first to avoid borrow conflicts with CPI
        let pool_status = ctx.accounts.launch_pool.status;
        let base_price = ctx.accounts.launch_pool.price_lamports;
        let slope = ctx.accounts.launch_pool.slope_scaled;
        let cur_tokens_sold = ctx.accounts.launch_pool.tokens_sold_curve;
        let mint_key = ctx.accounts.launch_pool.mint;
        let pool_bump = ctx.accounts.launch_pool.bump;
        let graduation_threshold = ctx.accounts.launch_pool.graduation_threshold;

        require!(pool_status == LaunchStatus::BondingCurve, SamesError::NotBondingCurve);

        // Calculate tokens for this SOL amount
        let tokens = bonding_curve_tokens_for_sol(base_price, slope, cur_tokens_sold, sol_amount)
            .ok_or(SamesError::MathOverflow)?;
        require!(tokens > 0, SamesError::ZeroDeposit);

        let cost = bonding_curve_cost(base_price, slope, cur_tokens_sold, tokens)
            .ok_or(SamesError::MathOverflow)?;
        require!(cost <= sol_amount, SamesError::InsufficientBalance);

        // Transfer SOL to vault
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.buyer.to_account_info(),
                    to: ctx.accounts.vault.to_account_info(),
                },
            ),
            cost,
        )?;

        // Mint tokens to buyer
        let pool_seeds: &[&[u8]] = &[b"launch_pool", mint_key.as_ref(), &[pool_bump]];
        token_2022::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token_2022::MintTo {
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.buyer_token_account.to_account_info(),
                    authority: ctx.accounts.launch_pool.to_account_info(),
                },
                &[pool_seeds],
            ),
            tokens,
        )?;

        // Now do all mutable updates
        let pool = &mut ctx.accounts.launch_pool;
        pool.tokens_sold_curve = pool.tokens_sold_curve
            .checked_add(tokens).ok_or(SamesError::MathOverflow)?;
        pool.curve_sol_collected = pool.curve_sol_collected
            .checked_add(cost).ok_or(SamesError::MathOverflow)?;

        let record = &mut ctx.accounts.buyer_record;
        if record.sol_deposited == 0 && record.curve_sol_spent == 0 {
            record.launch_pool = pool.key();
            record.buyer = ctx.accounts.buyer.key();
            record.tokens_allocated = 0;
            record.tokens_sold = 0;
            record.bump = ctx.bumps.buyer_record;
            record._reserved = [0u8; 32];
            pool.buyer_count = pool.buyer_count.checked_add(1).ok_or(SamesError::MathOverflow)?;
        }

        record.curve_sol_spent = record.curve_sol_spent
            .checked_add(cost).ok_or(SamesError::MathOverflow)?;
        record.curve_tokens_bought = record.curve_tokens_bought
            .checked_add(tokens).ok_or(SamesError::MathOverflow)?;

        // Update entry price to weighted average
        let total_sol = record.sol_deposited.saturating_add(record.curve_sol_spent);
        let total_tkns = record.tokens_allocated.saturating_add(record.curve_tokens_bought);
        if total_tkns > 0 {
            record.entry_price = ((total_sol as u128)
                .checked_div(total_tkns as u128).unwrap_or(0)) as u64;
        }

        // Check graduation
        if pool.curve_sol_collected >= graduation_threshold {
            msg!("SAMES: Graduation threshold reached! {} lamports", pool.curve_sol_collected);
        }

        let new_price = bonding_curve_price(base_price, slope, pool.tokens_sold_curve);
        msg!("SAMES: Curve buy {} tokens for {} lamports. Price: {}", tokens, cost, new_price);
        Ok(())
    }

    // ═════════════════════════════════════════════════════════════════════
    // 5. SELL ON BONDING CURVE (Phase 2 — with price floor)
    // ═════════════════════════════════════════════════════════════════════
    pub fn sell_curve(ctx: Context<SellCurve>, token_amount: u64) -> Result<()> {
        require!(token_amount > 0, SamesError::ZeroSellAmount);

        // Read values first to avoid borrow issues
        let pool_status = ctx.accounts.launch_pool.status;
        let base_price = ctx.accounts.launch_pool.price_lamports;
        let slope = ctx.accounts.launch_pool.slope_scaled;
        let tokens_sold = ctx.accounts.launch_pool.tokens_sold_curve;
        let _vault_bump = ctx.accounts.launch_pool.vault_bump;
        let entry_price = ctx.accounts.buyer_record.entry_price;

        require!(pool_status == LaunchStatus::BondingCurve, SamesError::NotBondingCurve);

        // Check balance
        let total_tokens = ctx.accounts.buyer_record.tokens_allocated
            .saturating_add(ctx.accounts.buyer_record.curve_tokens_bought);
        let available = total_tokens.saturating_sub(ctx.accounts.buyer_record.tokens_sold);
        require!(token_amount <= available, SamesError::InsufficientBalance);

        // PRICE FLOOR CHECK
        let current_price = bonding_curve_price(base_price, slope, tokens_sold);
        require!(current_price >= entry_price, SamesError::SellBelowEntry);

        // Calculate SOL to return
        let sol_return_raw = bonding_curve_cost(
            base_price, slope,
            tokens_sold.saturating_sub(token_amount),
            token_amount,
        ).ok_or(SamesError::MathOverflow)?;

        // Apply 1% fee
        let fee = sol_return_raw.checked_mul(PLATFORM_FEE_BPS)
            .ok_or(SamesError::MathOverflow)?
            .checked_div(10_000)
            .ok_or(SamesError::MathOverflow)?;
        let sol_return = sol_return_raw.saturating_sub(fee);

        // Transfer SOL from vault to seller
        **ctx.accounts.vault.to_account_info().try_borrow_mut_lamports()? -= sol_return;
        **ctx.accounts.seller.to_account_info().try_borrow_mut_lamports()? += sol_return;

        // Update state
        let pool = &mut ctx.accounts.launch_pool;
        pool.tokens_sold_curve = pool.tokens_sold_curve.saturating_sub(token_amount);
        pool.curve_sol_collected = pool.curve_sol_collected.saturating_sub(sol_return_raw);
        let record = &mut ctx.accounts.buyer_record;
        record.tokens_sold = record.tokens_sold
            .checked_add(token_amount).ok_or(SamesError::MathOverflow)?;

        // Burn tokens from seller
        token_2022::burn(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                token_2022::Burn {
                    mint: ctx.accounts.mint.to_account_info(),
                    from: ctx.accounts.seller_token_account.to_account_info(),
                    authority: ctx.accounts.seller.to_account_info(),
                },
            ),
            token_amount,
        )?;

        msg!("SAMES: Curve sell {} tokens for {} lamports (fee: {})", token_amount, sol_return, fee);
        Ok(())
    }

    // ═════════════════════════════════════════════════════════════════════
    // 6. GRADUATE (Phase 2 → Phase 3)
    // ═════════════════════════════════════════════════════════════════════
    /// Anyone can call this once the graduation threshold is met.
    /// In production, this would create a Raydium LP and migrate liquidity.
    /// For now, it just flips the status.
    pub fn graduate(ctx: Context<Graduate>) -> Result<()> {
        let pool = &mut ctx.accounts.launch_pool;
        require!(pool.status == LaunchStatus::BondingCurve, SamesError::NotBondingCurve);
        require!(pool.curve_sol_collected >= pool.graduation_threshold, SamesError::NotReadyToGraduate);

        pool.status = LaunchStatus::Graduated;

        // TODO: In production:
        // 1. Create Raydium AMM pool
        // 2. Add liquidity from vault
        // 3. Burn LP tokens or send to creator
        // 4. Remaining vault SOL to creator as profit

        msg!("SAMES: 🎓 GRADUATED! Token is now on Raydium. Price floor removed.");
        Ok(())
    }

    // ═════════════════════════════════════════════════════════════════════
    // 7. UPDATE PRICE (oracle/cranker)
    // ═════════════════════════════════════════════════════════════════════
    pub fn update_price(ctx: Context<UpdatePrice>, new_price: u64) -> Result<()> {
        let pool = &mut ctx.accounts.launch_pool;
        require!(pool.creator == ctx.accounts.authority.key(), SamesError::UnauthorizedCreator);
        require!(new_price > 0, SamesError::ZeroPrice);
        pool.price_lamports = new_price;
        msg!("SAMES: Price updated to {}", new_price);
        Ok(())
    }

    // ═════════════════════════════════════════════════════════════════════
    // 8. REGISTER MARKET ACCOUNT
    // ═════════════════════════════════════════════════════════════════════
    pub fn register_market(ctx: Context<RegisterMarket>, market_account: Pubkey) -> Result<()> {
        let registry = &mut ctx.accounts.market_registry;
        require!(registry.authority == ctx.accounts.authority.key(), SamesError::UnauthorizedCreator);
        require!(registry.market_accounts.len() < MarketRegistry::MAX_MARKETS, SamesError::InvalidMarket);
        registry.market_accounts.push(market_account);
        msg!("SAMES: Registered market {}", market_account);
        Ok(())
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// ACCOUNT CONTEXTS
// ═════════════════════════════════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(token_name: String, token_symbol: String, total_supply: u64, price_lamports: u64)]
pub struct CreateLaunch<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    /// CHECK: Token-2022 mint.
    pub mint: UncheckedAccount<'info>,
    #[account(
        init, payer = creator, space = LaunchPool::MAX_SIZE,
        seeds = [b"launch_pool", mint.key().as_ref()], bump,
    )]
    pub launch_pool: Account<'info, LaunchPool>,
    /// CHECK: SOL vault PDA.
    #[account(mut, seeds = [b"vault", launch_pool.key().as_ref()], bump)]
    pub vault: SystemAccount<'info>,
    #[account(
        init, payer = creator, space = MarketRegistry::MAX_SIZE,
        seeds = [b"market_registry", launch_pool.key().as_ref()], bump,
    )]
    pub market_registry: Account<'info, MarketRegistry>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(sol_amount: u64)]
pub struct BuyPresale<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(mut, seeds = [b"launch_pool", launch_pool.mint.as_ref()], bump = launch_pool.bump)]
    pub launch_pool: Account<'info, LaunchPool>,
    /// CHECK: SOL vault PDA.
    #[account(mut, seeds = [b"vault", launch_pool.key().as_ref()], bump = launch_pool.vault_bump)]
    pub vault: SystemAccount<'info>,
    #[account(
        init_if_needed, payer = buyer, space = BuyerRecord::MAX_SIZE,
        seeds = [b"buyer_record", launch_pool.key().as_ref(), buyer.key().as_ref()], bump,
    )]
    pub buyer_record: Account<'info, BuyerRecord>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct FinalizeLaunch<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(mut, seeds = [b"launch_pool", launch_pool.mint.as_ref()], bump = launch_pool.bump)]
    pub launch_pool: Account<'info, LaunchPool>,
    #[account(mut, constraint = mint.key() == launch_pool.mint @ SamesError::InvalidMint)]
    pub mint: InterfaceAccount<'info, MintAccount>,
    #[account(
        mut, seeds = [b"buyer_record", launch_pool.key().as_ref(), buyer_record.buyer.as_ref()],
        bump = buyer_record.bump,
    )]
    pub buyer_record: Account<'info, BuyerRecord>,
    #[account(mut)]
    pub buyer_token_account: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Program<'info, Token2022>,
}

#[derive(Accounts)]
pub struct StartBondingCurve<'info> {
    pub creator: Signer<'info>,
    #[account(mut, seeds = [b"launch_pool", launch_pool.mint.as_ref()], bump = launch_pool.bump)]
    pub launch_pool: Account<'info, LaunchPool>,
}

#[derive(Accounts)]
#[instruction(sol_amount: u64)]
pub struct BuyCurve<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(mut, seeds = [b"launch_pool", launch_pool.mint.as_ref()], bump = launch_pool.bump)]
    pub launch_pool: Account<'info, LaunchPool>,
    #[account(mut, constraint = mint.key() == launch_pool.mint @ SamesError::InvalidMint)]
    pub mint: InterfaceAccount<'info, MintAccount>,
    /// CHECK: SOL vault PDA.
    #[account(mut, seeds = [b"vault", launch_pool.key().as_ref()], bump = launch_pool.vault_bump)]
    pub vault: SystemAccount<'info>,
    #[account(
        init_if_needed, payer = buyer, space = BuyerRecord::MAX_SIZE,
        seeds = [b"buyer_record", launch_pool.key().as_ref(), buyer.key().as_ref()], bump,
    )]
    pub buyer_record: Account<'info, BuyerRecord>,
    #[account(mut)]
    pub buyer_token_account: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Program<'info, Token2022>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(token_amount: u64)]
pub struct SellCurve<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,
    #[account(mut, seeds = [b"launch_pool", launch_pool.mint.as_ref()], bump = launch_pool.bump)]
    pub launch_pool: Account<'info, LaunchPool>,
    #[account(mut, constraint = mint.key() == launch_pool.mint @ SamesError::InvalidMint)]
    pub mint: InterfaceAccount<'info, MintAccount>,
    /// CHECK: SOL vault PDA.
    #[account(mut, seeds = [b"vault", launch_pool.key().as_ref()], bump = launch_pool.vault_bump)]
    pub vault: SystemAccount<'info>,
    #[account(
        mut,
        seeds = [b"buyer_record", launch_pool.key().as_ref(), seller.key().as_ref()],
        bump = buyer_record.bump,
        constraint = buyer_record.buyer == seller.key() @ SamesError::NoBuyerRecord,
    )]
    pub buyer_record: Account<'info, BuyerRecord>,
    #[account(mut)]
    pub seller_token_account: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Program<'info, Token2022>,
}

#[derive(Accounts)]
pub struct Graduate<'info> {
    #[account(mut)]
    pub caller: Signer<'info>,
    #[account(mut, seeds = [b"launch_pool", launch_pool.mint.as_ref()], bump = launch_pool.bump)]
    pub launch_pool: Account<'info, LaunchPool>,
    /// CHECK: SOL vault PDA.
    #[account(mut, seeds = [b"vault", launch_pool.key().as_ref()], bump = launch_pool.vault_bump)]
    pub vault: SystemAccount<'info>,
}

#[derive(Accounts)]
pub struct UpdatePrice<'info> {
    pub authority: Signer<'info>,
    #[account(mut, seeds = [b"launch_pool", launch_pool.mint.as_ref()], bump = launch_pool.bump)]
    pub launch_pool: Account<'info, LaunchPool>,
}

#[derive(Accounts)]
pub struct RegisterMarket<'info> {
    pub authority: Signer<'info>,
    #[account(mut, seeds = [b"market_registry", market_registry.launch_pool.as_ref()], bump = market_registry.bump)]
    pub market_registry: Account<'info, MarketRegistry>,
}
