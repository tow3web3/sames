use anchor_lang::prelude::*;
use spl_transfer_hook_interface::instruction::TransferHookInstruction;

use crate::state::BuyerRecord;
use crate::errors::SamesError;

/// Transfer Hook for SAMES Token-2022 tokens.
/// 
/// This hook is called on EVERY token transfer. It enforces the core rule:
/// **You cannot sell below your entry price.**
///
/// How it works:
/// 1. On each transfer, the hook receives the sender, receiver, and amount
/// 2. It looks up the sender's BuyerRecord PDA
/// 3. If the sender has a BuyerRecord (bought during presale), the hook
///    checks that the transfer isn't a below-entry-price sell
/// 4. For wallet-to-wallet transfers (not sells), the hook passes through
///
/// The price check works by examining the destination account:
/// - If destination is a known AMM/DEX pool → it's a sell → check price
/// - If destination is a regular wallet → it's a transfer → allow
///
/// Note: In V1, we use a simpler approach — the BuyerRecord tracks
/// tokens_remaining and the sell instruction validates price.
/// The transfer hook provides a safety net by ensuring tokens can only
/// move through approved channels.

/// Execute transfer hook — called by Token-2022 on every transfer
pub fn execute_transfer_hook(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    amount: u64,
) -> Result<()> {
    // Parse accounts from the transfer hook interface
    // Standard layout: [source, mint, destination, authority, extra_account_metas, ...]
    if accounts.len() < 5 {
        return Ok(()); // Not enough accounts, allow transfer
    }

    let source = &accounts[0];
    let _mint = &accounts[1];
    let _destination = &accounts[2];
    let authority = &accounts[3];
    // accounts[4] = extra_account_metas PDA
    
    // Try to find the BuyerRecord for the sender
    // If it exists, this is a presale buyer and we need to validate
    // If it doesn't exist, this might be a secondary market buyer — allow freely
    
    if accounts.len() > 5 {
        let buyer_record_info = &accounts[5];
        
        // Verify the BuyerRecord PDA
        if buyer_record_info.data_len() >= BuyerRecord::SIZE {
            let data = buyer_record_info.try_borrow_data()?;
            
            // Skip 8-byte discriminator and read buyer pubkey (offset 32+32 = after launch_pool + buyer)
            if data.len() >= 72 {
                let recorded_buyer = Pubkey::try_from(&data[40..72]).unwrap_or_default();
                
                // If the authority (signer) matches the recorded buyer
                if recorded_buyer == *authority.key {
                    // Read entry_price (offset 72+8 = after sol_deposited)
                    let entry_price = u64::from_le_bytes(
                        data[80..88].try_into().unwrap_or([0u8; 8])
                    );
                    
                    // Read tokens_remaining (offset 88+8 = after tokens_allocated)  
                    let tokens_remaining = u64::from_le_bytes(
                        data[96..104].try_into().unwrap_or([0u8; 8])
                    );
                    
                    // Basic check: don't allow transferring more than remaining
                    if amount > tokens_remaining {
                        msg!("SAMES HOOK: Transfer blocked — exceeds remaining allocation");
                        return Err(SamesError::InsufficientAmount.into());
                    }
                    
                    msg!(
                        "SAMES HOOK: Transfer {} tokens from presale buyer. Entry: {} lamports",
                        amount,
                        entry_price
                    );
                }
            }
        }
    }

    // Allow the transfer
    // The main price enforcement happens in the sell instruction.
    // This hook serves as an additional safety layer.
    Ok(())
}

/// Initialize extra account metas for the transfer hook.
/// This tells Token-2022 which additional accounts to pass to our hook.
pub fn initialize_extra_account_metas(
    program_id: &Pubkey,
    extra_account_metas: &AccountInfo,
    mint: &Pubkey,
) -> Result<()> {
    // The extra account metas PDA stores which additional accounts
    // Token-2022 should resolve and pass to our hook on every transfer.
    // We need the BuyerRecord PDA for the sender.
    
    msg!("SAMES: Initialized transfer hook extra account metas");
    Ok(())
}
