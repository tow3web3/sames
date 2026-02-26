use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token_2022::{self, Token2022};
use anchor_spl::token_interface::{Mint as MintAccount, TokenAccount};

pub mod errors;
pub mod state;
pub mod hook;

use errors::SamesError;
use state::*;

declare_id!("3Sew3pFCTkvFZ8Ayj2CgrL2cr9FBTR5ChNytPmUMi5mu");

/// Presale window duration in seconds.
const PRESALE_DURATION: i64 = 30;

#[program]
pub mod sames {
    use super::*;

    // ═════════════════════════════════════════════════════════════════════
    // 1. CREATE LAUNCH
    // ═════════════════════════════════════════════════════════════════════
    /// Creator initializes a new token launch with a 30-second presale window.
    ///
    /// The mint should already be created as a Token-2022 mint with transfer
    /// hook extension pointing to this program. This instruction sets up
    /// the LaunchPool state and SOL vault.
    pub fn create_launch(
        ctx: Context<CreateLaunch>,
        token_name: String,
        token_symbol: String,
        total_supply: u64,
        price_lamports: u64,
    ) -> Result<()> {
        // ── Validation ──────────────────────────────────────────────────
        require!(token_name.len() <= 32, SamesError::NameTooLong);
        require!(token_symbol.len() <= 10, SamesError::SymbolTooLong);
        require!(total_supply > 0, SamesError::ZeroSupply);
        require!(price_lamports > 0, SamesError::ZeroPrice);

        let clock = Clock::get()?;
        let now = clock.unix_timestamp;

        // ── Initialize LaunchPool ───────────────────────────────────────
        let pool = &mut ctx.accounts.launch_pool;
        pool.creator = ctx.accounts.creator.key();
        pool.mint = ctx.accounts.mint.key();
        pool.token_name = token_name;
        pool.token_symbol = token_symbol;
        pool.total_supply = total_supply;
        pool.price_lamports = price_lamports;
        pool.start_time = now;
        pool.end_time = now
            .checked_add(PRESALE_DURATION)
            .ok_or(SamesError::MathOverflow)?;
        pool.total_sol_collected = 0;
        pool.buyer_count = 0;
        pool.status = LaunchStatus::Presale;
        pool.bump = ctx.bumps.launch_pool;
        pool.vault_bump = ctx.bumps.vault;
        pool._reserved = [0u8; 128];

        // ── Initialize MarketRegistry ───────────────────────────────────
        let registry = &mut ctx.accounts.market_registry;
        registry.launch_pool = pool.key();
        registry.authority = ctx.accounts.creator.key();
        registry.market_accounts = Vec::new();
        registry.bump = ctx.bumps.market_registry;

        msg!(
            "SAMES: Launch created. Presale open from {} to {}",
            pool.start_time,
            pool.end_time
        );

        Ok(())
    }

    // ═════════════════════════════════════════════════════════════════════
    // 2. BUY PRESALE
    // ═════════════════════════════════════════════════════════════════════
    /// User deposits SOL during the 30-second presale window.
    /// Creates a BuyerRecord PDA storing their entry price.
    /// Multiple buys by the same user accumulate into the same record.
    pub fn buy_presale(ctx: Context<BuyPresale>, sol_amount: u64) -> Result<()> {
        require!(sol_amount > 0, SamesError::ZeroDeposit);

        let clock = Clock::get()?;
        let now = clock.unix_timestamp;
        let pool = &mut ctx.accounts.launch_pool;

        // ── Time checks ─────────────────────────────────────────────────
        require!(now >= pool.start_time, SamesError::PresaleNotStarted);
        require!(now < pool.end_time, SamesError::PresaleEnded);
        require!(
            pool.status == LaunchStatus::Presale,
            SamesError::AlreadyFinalized
        );

        // ── Transfer SOL from buyer to vault ────────────────────────────
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

        // ── Update pool totals ──────────────────────────────────────────
        pool.total_sol_collected = pool
            .total_sol_collected
            .checked_add(sol_amount)
            .ok_or(SamesError::MathOverflow)?;

        // ── Initialize or update BuyerRecord ────────────────────────────
        let record = &mut ctx.accounts.buyer_record;
        if record.sol_deposited == 0 {
            // New buyer
            record.launch_pool = pool.key();
            record.buyer = ctx.accounts.buyer.key();
            record.entry_price = pool.price_lamports;
            record.tokens_allocated = 0;
            record.tokens_sold = 0;
            record.bump = ctx.bumps.buyer_record;
            record._reserved = [0u8; 64];

            pool.buyer_count = pool
                .buyer_count
                .checked_add(1)
                .ok_or(SamesError::MathOverflow)?;
        }

        record.sol_deposited = record
            .sol_deposited
            .checked_add(sol_amount)
            .ok_or(SamesError::MathOverflow)?;

        msg!(
            "SAMES: Buyer {} deposited {} lamports (total: {})",
            ctx.accounts.buyer.key(),
            sol_amount,
            record.sol_deposited
        );

        Ok(())
    }

    // ═════════════════════════════════════════════════════════════════════
    // 3. FINALIZE LAUNCH
    // ═════════════════════════════════════════════════════════════════════
    /// Called after the 30-second window ends.
    /// Calculates token allocations for each buyer.
    /// In production, this would also create an LP / enable trading.
    ///
    /// NOTE: Because Solana transactions have compute limits, finalization
    /// for many buyers should be done in batches. This instruction handles
    /// a single buyer at a time — call it once per buyer.
    pub fn finalize_launch(ctx: Context<FinalizeLaunch>) -> Result<()> {
        let clock = Clock::get()?;
        let now = clock.unix_timestamp;
        let pool = &ctx.accounts.launch_pool;

        // ── Checks ──────────────────────────────────────────────────────
        require!(
            pool.creator == ctx.accounts.creator.key(),
            SamesError::UnauthorizedCreator
        );
        require!(pool.is_presale_over(now), SamesError::PresaleStillActive);
        require!(
            pool.status == LaunchStatus::Presale,
            SamesError::AlreadyFinalized
        );

        // ── Calculate this buyer's token allocation ─────────────────────
        // tokens = (buyer_sol / total_sol) * total_supply
        let record = &mut ctx.accounts.buyer_record;
        require!(record.sol_deposited > 0, SamesError::ZeroDeposit);

        let tokens = (record.sol_deposited as u128)
            .checked_mul(pool.total_supply as u128)
            .ok_or(SamesError::MathOverflow)?
            .checked_div(pool.total_sol_collected as u128)
            .ok_or(SamesError::MathOverflow)? as u64;

        record.tokens_allocated = tokens;

        // ── Mint tokens to buyer's token account ────────────────────────
        // The mint authority is the LaunchPool PDA.
        let mint_key = pool.mint;
        let pool_seeds: &[&[u8]] = &[
            b"launch_pool",
            mint_key.as_ref(),
            &[pool.bump],
        ];

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

        msg!(
            "SAMES: Allocated {} tokens to buyer {}",
            tokens,
            record.buyer
        );

        Ok(())
    }

    // ═════════════════════════════════════════════════════════════════════
    // 3b. SET LAUNCH LIVE
    // ═════════════════════════════════════════════════════════════════════
    /// After all buyers are finalized, creator sets the pool to Live.
    /// This enables the transfer hook price enforcement.
    pub fn set_launch_live(ctx: Context<SetLaunchLive>) -> Result<()> {
        let pool = &mut ctx.accounts.launch_pool;

        require!(
            pool.creator == ctx.accounts.creator.key(),
            SamesError::UnauthorizedCreator
        );
        require!(
            pool.status == LaunchStatus::Presale,
            SamesError::AlreadyFinalized
        );

        let clock = Clock::get()?;
        require!(
            pool.is_presale_over(clock.unix_timestamp),
            SamesError::PresaleStillActive
        );

        pool.status = LaunchStatus::Live;
        msg!("SAMES: Launch is now LIVE. Transfer hooks active.");

        Ok(())
    }

    // ═════════════════════════════════════════════════════════════════════
    // 4. SELL (explicit sell instruction with price check)
    // ═════════════════════════════════════════════════════════════════════
    /// User sells tokens. The program verifies that the current price
    /// is at or above their entry price before allowing the transfer.
    ///
    /// This is the explicit sell path. The transfer hook provides
    /// an additional enforcement layer for DEX transfers.
    pub fn sell(ctx: Context<Sell>, token_amount: u64, current_price: u64) -> Result<()> {
        require!(token_amount > 0, SamesError::ZeroSellAmount);

        let pool = &ctx.accounts.launch_pool;
        require!(
            pool.status == LaunchStatus::Live,
            SamesError::NotFinalized
        );

        let record = &mut ctx.accounts.buyer_record;

        // ── Price floor check ───────────────────────────────────────────
        require!(
            current_price >= record.entry_price,
            SamesError::SellBelowEntry
        );

        // ── Balance check ───────────────────────────────────────────────
        let remaining = record
            .tokens_allocated
            .checked_sub(record.tokens_sold)
            .ok_or(SamesError::MathOverflow)?;
        require!(token_amount <= remaining, SamesError::InsufficientBalance);

        // ── Update sold amount ──────────────────────────────────────────
        record.tokens_sold = record
            .tokens_sold
            .checked_add(token_amount)
            .ok_or(SamesError::MathOverflow)?;

        // ── Transfer tokens from seller to destination ──────────────────
        // In production this would go to a DEX pool / market maker.
        // For now we transfer to the provided destination account.
        let mint_key = pool.mint;
        let _pool_key = ctx.accounts.launch_pool.key();
        let _buyer_key = record.buyer;

        token_2022::transfer_checked(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                token_2022::TransferChecked {
                    from: ctx.accounts.seller_token_account.to_account_info(),
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.destination_token_account.to_account_info(),
                    authority: ctx.accounts.seller.to_account_info(),
                },
            ),
            token_amount,
            ctx.accounts.mint.decimals,
        )?;

        msg!(
            "SAMES: Sell OK. {} tokens at price {} (entry: {})",
            token_amount,
            current_price,
            record.entry_price
        );

        Ok(())
    }

    // ═════════════════════════════════════════════════════════════════════
    // 5. UPDATE MARKET PRICE (oracle/cranker)
    // ═════════════════════════════════════════════════════════════════════
    /// Updates the current market price in the LaunchPool.
    /// Only callable by the creator (in production, use a proper oracle).
    pub fn update_price(ctx: Context<UpdatePrice>, new_price: u64) -> Result<()> {
        let pool = &mut ctx.accounts.launch_pool;
        require!(
            pool.creator == ctx.accounts.authority.key(),
            SamesError::UnauthorizedCreator
        );
        require!(new_price > 0, SamesError::ZeroPrice);

        pool.price_lamports = new_price;
        msg!("SAMES: Market price updated to {} lamports", new_price);
        Ok(())
    }

    // ═════════════════════════════════════════════════════════════════════
    // 6. REGISTER MARKET ACCOUNT
    // ═════════════════════════════════════════════════════════════════════
    /// Adds a DEX/market token account to the registry.
    /// Transfers to registered accounts trigger price floor enforcement.
    pub fn register_market(ctx: Context<RegisterMarket>, market_account: Pubkey) -> Result<()> {
        let registry = &mut ctx.accounts.market_registry;
        require!(
            registry.authority == ctx.accounts.authority.key(),
            SamesError::UnauthorizedCreator
        );
        require!(
            registry.market_accounts.len() < MarketRegistry::MAX_MARKETS,
            SamesError::InvalidMarket
        );

        registry.market_accounts.push(market_account);
        msg!("SAMES: Registered market account {}", market_account);
        Ok(())
    }

    // Transfer hook instructions temporarily removed for devnet build.
    // Will be re-added when Token-2022 hook is properly configured.
}

// ═════════════════════════════════════════════════════════════════════════════
// ACCOUNT CONTEXTS
// ═════════════════════════════════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(token_name: String, token_symbol: String, total_supply: u64, price_lamports: u64)]
pub struct CreateLaunch<'info> {
    /// The launch creator, pays for account creation.
    #[account(mut)]
    pub creator: Signer<'info>,

    /// The Token-2022 mint (must already exist with transfer hook extension).
    /// CHECK: We validate it's a valid mint via the Token-2022 program.
    pub mint: UncheckedAccount<'info>,

    /// LaunchPool PDA — stores all launch state.
    #[account(
        init,
        payer = creator,
        space = LaunchPool::MAX_SIZE,
        seeds = [b"launch_pool", mint.key().as_ref()],
        bump,
    )]
    pub launch_pool: Account<'info, LaunchPool>,

    /// SOL vault PDA — holds deposited SOL during presale.
    /// CHECK: This is a PDA that holds SOL, not a data account.
    #[account(
        mut,
        seeds = [b"vault", launch_pool.key().as_ref()],
        bump,
    )]
    pub vault: SystemAccount<'info>,

    /// Market registry PDA — stores whitelisted DEX accounts.
    #[account(
        init,
        payer = creator,
        space = MarketRegistry::MAX_SIZE,
        seeds = [b"market_registry", launch_pool.key().as_ref()],
        bump,
    )]
    pub market_registry: Account<'info, MarketRegistry>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(sol_amount: u64)]
pub struct BuyPresale<'info> {
    /// The buyer depositing SOL.
    #[account(mut)]
    pub buyer: Signer<'info>,

    /// The LaunchPool for this token.
    #[account(
        mut,
        seeds = [b"launch_pool", launch_pool.mint.as_ref()],
        bump = launch_pool.bump,
    )]
    pub launch_pool: Account<'info, LaunchPool>,

    /// SOL vault PDA.
    /// CHECK: PDA holding SOL.
    #[account(
        mut,
        seeds = [b"vault", launch_pool.key().as_ref()],
        bump = launch_pool.vault_bump,
    )]
    pub vault: SystemAccount<'info>,

    /// BuyerRecord PDA — created on first buy, updated on subsequent buys.
    #[account(
        init_if_needed,
        payer = buyer,
        space = BuyerRecord::MAX_SIZE,
        seeds = [b"buyer_record", launch_pool.key().as_ref(), buyer.key().as_ref()],
        bump,
    )]
    pub buyer_record: Account<'info, BuyerRecord>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct FinalizeLaunch<'info> {
    /// The launch creator.
    #[account(mut)]
    pub creator: Signer<'info>,

    /// The LaunchPool.
    #[account(
        mut,
        seeds = [b"launch_pool", launch_pool.mint.as_ref()],
        bump = launch_pool.bump,
    )]
    pub launch_pool: Account<'info, LaunchPool>,

    /// The Token-2022 mint — LaunchPool PDA is the mint authority.
    #[account(
        mut,
        constraint = mint.key() == launch_pool.mint @ SamesError::InvalidMint,
    )]
    pub mint: InterfaceAccount<'info, MintAccount>,

    /// The buyer's BuyerRecord.
    #[account(
        mut,
        seeds = [b"buyer_record", launch_pool.key().as_ref(), buyer_record.buyer.as_ref()],
        bump = buyer_record.bump,
    )]
    pub buyer_record: Account<'info, BuyerRecord>,

    /// The buyer's token account (Token-2022).
    #[account(mut)]
    pub buyer_token_account: InterfaceAccount<'info, TokenAccount>,

    /// Token-2022 program.
    pub token_program: Program<'info, Token2022>,
}

#[derive(Accounts)]
pub struct SetLaunchLive<'info> {
    pub creator: Signer<'info>,

    #[account(
        mut,
        seeds = [b"launch_pool", launch_pool.mint.as_ref()],
        bump = launch_pool.bump,
    )]
    pub launch_pool: Account<'info, LaunchPool>,
}

#[derive(Accounts)]
pub struct Sell<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    #[account(
        seeds = [b"launch_pool", launch_pool.mint.as_ref()],
        bump = launch_pool.bump,
    )]
    pub launch_pool: Account<'info, LaunchPool>,

    #[account(
        mut,
        constraint = mint.key() == launch_pool.mint @ SamesError::InvalidMint,
    )]
    pub mint: InterfaceAccount<'info, MintAccount>,

    #[account(
        mut,
        seeds = [b"buyer_record", launch_pool.key().as_ref(), seller.key().as_ref()],
        bump = buyer_record.bump,
        constraint = buyer_record.buyer == seller.key() @ SamesError::NoBuyerRecord,
    )]
    pub buyer_record: Account<'info, BuyerRecord>,

    /// Seller's token account.
    #[account(mut)]
    pub seller_token_account: InterfaceAccount<'info, TokenAccount>,

    /// Destination token account (e.g., DEX pool).
    #[account(mut)]
    pub destination_token_account: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Program<'info, Token2022>,
}

#[derive(Accounts)]
pub struct UpdatePrice<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"launch_pool", launch_pool.mint.as_ref()],
        bump = launch_pool.bump,
    )]
    pub launch_pool: Account<'info, LaunchPool>,
}

#[derive(Accounts)]
pub struct RegisterMarket<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"market_registry", market_registry.launch_pool.as_ref()],
        bump = market_registry.bump,
    )]
    pub market_registry: Account<'info, MarketRegistry>,
}
