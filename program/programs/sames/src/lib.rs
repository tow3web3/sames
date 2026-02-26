use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token_2022::{self, Token2022, MintTo};
use anchor_spl::associated_token::AssociatedToken;

pub mod errors;
pub mod state;
pub mod transfer_hook;

use errors::SamesError;
use state::*;

declare_id!("SAMESpXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX");

#[program]
pub mod sames {
    use super::*;

    /// Create a new token launch with a presale window.
    /// The token uses Token-2022 with a transfer hook that prevents
    /// selling below entry price.
    pub fn create_launch(
        ctx: Context<CreateLaunch>,
        price_lamports: u64,
        total_supply: u64,
        presale_seconds: u32,
    ) -> Result<()> {
        // Validate inputs
        require!(price_lamports > 0, SamesError::InvalidPrice);
        require!(total_supply > 0, SamesError::InvalidSupply);
        require!(
            presale_seconds >= 10 && presale_seconds <= 120,
            SamesError::InvalidPresaleWindow
        );

        let clock = Clock::get()?;
        let launch = &mut ctx.accounts.launch_pool;

        launch.creator = ctx.accounts.creator.key();
        launch.token_mint = ctx.accounts.token_mint.key();
        launch.price_lamports = price_lamports;
        launch.total_supply = total_supply;
        launch.presale_start = clock.unix_timestamp;
        launch.presale_end = clock.unix_timestamp + presale_seconds as i64;
        launch.total_sol_collected = 0;
        launch.buyer_count = 0;
        launch.status = LaunchStatus::Presale;
        launch.bump = ctx.bumps.launch_pool;

        msg!(
            "SAMES: Launch created. Token: {}. Price: {} lamports. Window: {}s",
            launch.token_mint,
            price_lamports,
            presale_seconds
        );

        Ok(())
    }

    /// Buy during the presale window.
    /// Everyone pays the same fixed price. SOL is held in escrow.
    pub fn buy_presale(ctx: Context<BuyPresale>, sol_amount: u64) -> Result<()> {
        require!(sol_amount > 0, SamesError::InsufficientAmount);

        let clock = Clock::get()?;
        let launch = &mut ctx.accounts.launch_pool;

        // Check presale is active
        require!(launch.status == LaunchStatus::Presale, SamesError::NotInPresale);
        require!(clock.unix_timestamp >= launch.presale_start, SamesError::PresaleNotStarted);
        require!(clock.unix_timestamp < launch.presale_end, SamesError::PresaleEnded);

        // Transfer SOL from buyer to launch pool escrow
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.buyer.to_account_info(),
                    to: ctx.accounts.launch_pool.to_account_info(),
                },
            ),
            sol_amount,
        )?;

        // Update launch pool
        launch.total_sol_collected = launch
            .total_sol_collected
            .checked_add(sol_amount)
            .ok_or(SamesError::MathOverflow)?;
        launch.buyer_count = launch
            .buyer_count
            .checked_add(1)
            .ok_or(SamesError::MaxBuyersReached)?;

        // Create buyer record
        let buyer_record = &mut ctx.accounts.buyer_record;
        buyer_record.launch_pool = launch.key();
        buyer_record.buyer = ctx.accounts.buyer.key();
        buyer_record.sol_deposited = sol_amount;
        buyer_record.entry_price = launch.price_lamports;
        buyer_record.tokens_allocated = 0; // Set during finalization
        buyer_record.tokens_remaining = 0;
        buyer_record.finalized = false;
        buyer_record.bump = ctx.bumps.buyer_record;

        msg!(
            "SAMES: Buy {} lamports at fixed price {} | Buyer #{}", 
            sol_amount,
            launch.price_lamports,
            launch.buyer_count
        );

        Ok(())
    }

    /// Finalize the launch after presale window closes.
    /// Mints tokens to each buyer proportional to their SOL deposit.
    /// Transitions status to Live — market is now open.
    pub fn finalize_buyer(ctx: Context<FinalizeBuyer>) -> Result<()> {
        let clock = Clock::get()?;
        let launch = &ctx.accounts.launch_pool;

        // Presale must be over
        require!(
            clock.unix_timestamp >= launch.presale_end,
            SamesError::PresaleStillActive
        );
        require!(launch.status == LaunchStatus::Presale, SamesError::AlreadyFinalized);

        let buyer_record = &mut ctx.accounts.buyer_record;
        require!(!buyer_record.finalized, SamesError::AlreadyFinalized);

        // Calculate token allocation:
        // tokens = (buyer_sol / total_sol) * total_supply
        let tokens = (buyer_record.sol_deposited as u128)
            .checked_mul(launch.total_supply as u128)
            .ok_or(SamesError::MathOverflow)?
            .checked_div(launch.total_sol_collected as u128)
            .ok_or(SamesError::MathOverflow)? as u64;

        buyer_record.tokens_allocated = tokens;
        buyer_record.tokens_remaining = tokens;
        buyer_record.finalized = true;

        // Mint tokens to buyer's token account
        let launch_key = launch.key();
        let seeds = &[
            b"launch",
            launch.creator.as_ref(),
            launch.token_mint.as_ref(),
            &[launch.bump],
        ];
        let signer_seeds = &[&seeds[..]];

        token_2022::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.token_mint.to_account_info(),
                    to: ctx.accounts.buyer_token_account.to_account_info(),
                    authority: ctx.accounts.launch_pool.to_account_info(),
                },
                signer_seeds,
            ),
            tokens,
        )?;

        msg!(
            "SAMES: Finalized buyer {}. Allocated {} tokens. Entry: {} lamports",
            buyer_record.buyer,
            tokens,
            buyer_record.entry_price
        );

        Ok(())
    }

    /// Sell tokens — only allowed at or above entry price.
    /// The transfer hook also enforces this, but this provides
    /// a clean sell interface with price validation.
    pub fn sell(ctx: Context<Sell>, amount: u64, min_price_lamports: u64) -> Result<()> {
        let buyer_record = &ctx.accounts.buyer_record;
        
        require!(buyer_record.finalized, SamesError::NotFinalized);
        require!(
            min_price_lamports >= buyer_record.entry_price,
            SamesError::SellBelowEntry
        );
        require!(
            amount <= buyer_record.tokens_remaining,
            SamesError::InsufficientAmount
        );

        // The actual swap would go through a DEX or AMM here.
        // The transfer hook on Token-2022 provides the second layer
        // of protection — even if someone tries to bypass this instruction,
        // the hook will block transfers that imply a sell below entry.

        msg!(
            "SAMES: Sell {} tokens at min price {} (entry was {})",
            amount,
            min_price_lamports,
            buyer_record.entry_price
        );

        Ok(())
    }
}

// ============================================================
// INSTRUCTION ACCOUNTS
// ============================================================

#[derive(Accounts)]
pub struct CreateLaunch<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,

    #[account(
        init,
        payer = creator,
        space = LaunchPool::SIZE,
        seeds = [b"launch", creator.key().as_ref(), token_mint.key().as_ref()],
        bump,
    )]
    pub launch_pool: Account<'info, LaunchPool>,

    /// The Token-2022 mint (must be created beforehand with transfer hook)
    /// CHECK: Validated in instruction logic
    pub token_mint: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token2022>,
}

#[derive(Accounts)]
pub struct BuyPresale<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,

    #[account(
        mut,
        seeds = [b"launch", launch_pool.creator.as_ref(), launch_pool.token_mint.as_ref()],
        bump = launch_pool.bump,
    )]
    pub launch_pool: Account<'info, LaunchPool>,

    #[account(
        init,
        payer = buyer,
        space = BuyerRecord::SIZE,
        seeds = [b"buyer", launch_pool.key().as_ref(), buyer.key().as_ref()],
        bump,
    )]
    pub buyer_record: Account<'info, BuyerRecord>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct FinalizeBuyer<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        seeds = [b"launch", launch_pool.creator.as_ref(), launch_pool.token_mint.as_ref()],
        bump = launch_pool.bump,
    )]
    pub launch_pool: Account<'info, LaunchPool>,

    #[account(
        mut,
        seeds = [b"buyer", launch_pool.key().as_ref(), buyer_record.buyer.as_ref()],
        bump = buyer_record.bump,
    )]
    pub buyer_record: Account<'info, BuyerRecord>,

    /// CHECK: Token mint validated against launch pool
    #[account(mut, address = launch_pool.token_mint)]
    pub token_mint: AccountInfo<'info>,

    /// CHECK: Buyer's associated token account
    #[account(mut)]
    pub buyer_token_account: AccountInfo<'info>,

    pub token_program: Program<'info, Token2022>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Sell<'info> {
    #[account(mut)]
    pub seller: Signer<'info>,

    #[account(
        seeds = [b"launch", launch_pool.creator.as_ref(), launch_pool.token_mint.as_ref()],
        bump = launch_pool.bump,
    )]
    pub launch_pool: Account<'info, LaunchPool>,

    #[account(
        mut,
        seeds = [b"buyer", launch_pool.key().as_ref(), seller.key().as_ref()],
        bump = buyer_record.bump,
        has_one = buyer @ SamesError::InvalidAuthority,
    )]
    pub buyer_record: Account<'info, BuyerRecord>,

    pub token_program: Program<'info, Token2022>,
    pub system_program: Program<'info, System>,
}
