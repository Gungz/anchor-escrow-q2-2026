use anchor_lang::prelude::*;

use crate::{ESCROW_SEED, error::ErrorCode, state::Escrow};

#[derive(Accounts)]
pub struct Update<'info> {
    pub maker: Signer<'info>,
    //Only the maker can update the terms, and only while the escrow is
    //still open: once the deadline passes the escrow can only be refunded.
    #[account(
        mut,
        has_one = maker,
        seeds = [ESCROW_SEED, maker.key().as_ref(), escrow.seed.to_le_bytes().as_ref()],
        bump = escrow.bump,
    )]
    pub escrow: Account<'info, Escrow>,
}

impl<'info> Update<'info> {
    //Update the deal terms (receive amount and expiration) of a live escrow.
    //The vault balance, mints and seed are immutable: to change those,
    //the maker has to refund and open a new escrow.
    pub fn update_terms(&mut self, receive: u64, expiration: i64) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;

        //The escrow can only be updated before the current deadline passes.
        require!(now < self.escrow.expiration, ErrorCode::EscrowExpired);
        //The new deadline must be a unix timestamp in the future.
        require!(expiration > now, ErrorCode::InvalidExpiration);
        //The maker must ask for at least one unit of mint_b.
        require!(receive > 0, ErrorCode::InvalidReceiveAmount);

        self.escrow.receive = receive;
        self.escrow.expiration = expiration;

        Ok(())
    }
}
