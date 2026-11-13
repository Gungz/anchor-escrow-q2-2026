#[cfg(test)]
mod tests {

    use {
        anchor_lang::{
            prelude::{msg, Clock},
            solana_program::instruction::Instruction,
            solana_program::program_pack::Pack,
            system_program::ID as SYSTEM_PROGRAM_ID,
            AccountDeserialize, InstructionData, ToAccountMetas,
        },
        anchor_spl::{
            associated_token::{self, ID as ASSOCIATED_TOKEN_PROGRAM_ID},
            token::spl_token,
        },
        litesvm::LiteSVM,
        litesvm_token::{
            spl_token::ID as TOKEN_PROGRAM_ID, CreateAssociatedTokenAccount, CreateMint, MintTo,
        },
        solana_keypair::Keypair,
        solana_message::Message,
        solana_pubkey::Pubkey,
        solana_signer::Signer,
        solana_transaction::Transaction,
    };

    // Realistic unix time (2027-01-15) used as the base for the test clock,
    // instead of LiteSVM's default all-zero clock.
    const NOW: i64 = 1_800_000_000;
    const ONE_HOUR: i64 = 3_600;

    // Custom error codes from the program, in declaration order.
    const ESCROW_EXPIRED: u32 = 6000;
    const ESCROW_NOT_EXPIRED: u32 = 6001;
    const INVALID_EXPIRATION: u32 = 6002;

    // Setup function to initialize LiteSVM and create a payer keypair
    fn setup() -> (LiteSVM, Keypair) {
        let program_id = anchor_escrow_q2_2026::id();
        let payer = Keypair::new();
        let mut svm = LiteSVM::new();
        let bytes = include_bytes!("../../../target/deploy/anchor_escrow_q2_2026.so");
        svm.add_program(program_id, bytes).unwrap();
        svm.airdrop(&payer.pubkey(), 1_000_000_000).unwrap();
        // Start from a realistic clock value instead of all-zero.
        set_clock(&mut svm, NOW);

        // Return the LiteSVM instance and payer keypair
        (svm, payer)
    }

    fn set_clock(program: &mut LiteSVM, unix_timestamp: i64) {
        let mut clock = program.get_sysvar::<Clock>();
        clock.unix_timestamp = unix_timestamp;
        program.set_sysvar(&clock);
    }

    fn create_mints(program: &mut LiteSVM, payer: &Keypair, maker: &Pubkey) -> (Pubkey, Pubkey) {
        // Create two mints (Mint A and Mint B) with 6 decimal places and the maker as the authority
        let mint_a = CreateMint::new(program, payer)
            .decimals(6)
            .authority(maker)
            .send()
            .unwrap();
        msg!("Mint A: {}\n", mint_a);

        let mint_b = CreateMint::new(program, payer)
            .decimals(6)
            .authority(maker)
            .send()
            .unwrap();
        msg!("Mint B: {}\n", mint_b);

        (mint_a, mint_b)
    }

    fn create_ata(program: &mut LiteSVM, payer: &Keypair, mint: &Pubkey, owner: &Pubkey) -> Pubkey {
        // Create an associated token account using litesvm-token's CreateAssociatedTokenAccount utility
        CreateAssociatedTokenAccount::new(program, payer, mint)
            .owner(owner)
            .send()
            .unwrap()
    }

    fn mint_tokens(program: &mut LiteSVM, payer: &Keypair, mint: &Pubkey, ata: &Pubkey, amount: u64) {
        // Mint tokens to an associated token account using litesvm-token's MintTo utility
        MintTo::new(program, payer, mint, ata, amount)
            .send()
            .unwrap();
    }

    fn escrow_pdas(maker: &Pubkey, seed: u64, mint_a: &Pubkey) -> (Pubkey, Pubkey) {
        // Derive the PDA for the escrow account using the maker's public key and a seed value
        let escrow = Pubkey::find_program_address(
            &[b"escrow", maker.as_ref(), &seed.to_le_bytes()],
            &anchor_escrow_q2_2026::id(),
        )
        .0;
        msg!("Escrow PDA: {}\n", escrow);

        // Derive the associated token address for the vault using the escrow PDA and Mint A
        let vault = associated_token::get_associated_token_address(&escrow, mint_a);
        msg!("Vault PDA: {}\n", vault);

        (escrow, vault)
    }

    fn make_ix(
        maker: Pubkey,
        mint_a: Pubkey,
        mint_b: Pubkey,
        maker_ata_a: Pubkey,
        escrow: Pubkey,
        vault: Pubkey,
        seed: u64,
        deposit: u64,
        receive: u64,
        expiration: i64,
    ) -> Instruction {
        // Create the "Make" instruction to deposit tokens into the escrow
        Instruction {
            program_id: anchor_escrow_q2_2026::id(),
            accounts: anchor_escrow_q2_2026::accounts::Make {
                maker,
                mint_a,
                mint_b,
                maker_ata_a,
                escrow,
                vault,
                associated_token_program: ASSOCIATED_TOKEN_PROGRAM_ID,
                token_program: TOKEN_PROGRAM_ID,
                system_program: SYSTEM_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: anchor_escrow_q2_2026::instruction::Make {
                deposit,
                seed,
                receive,
                expiration,
            }
            .data(),
        }
    }

    fn take_ix(
        taker: Pubkey,
        maker: Pubkey,
        mint_a: Pubkey,
        mint_b: Pubkey,
        taker_ata_a: Pubkey,
        taker_ata_b: Pubkey,
        maker_ata_b: Pubkey,
        escrow: Pubkey,
        vault: Pubkey,
    ) -> Instruction {
        // Create the "Take" instruction to execute the trade and close the escrow
        Instruction {
            program_id: anchor_escrow_q2_2026::id(),
            accounts: anchor_escrow_q2_2026::accounts::Take {
                taker,
                maker,
                mint_a,
                mint_b,
                taker_ata_a,
                taker_ata_b,
                maker_ata_b,
                escrow,
                vault,
                associated_token_program: ASSOCIATED_TOKEN_PROGRAM_ID,
                token_program: TOKEN_PROGRAM_ID,
                system_program: SYSTEM_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: anchor_escrow_q2_2026::instruction::Take {}.data(),
        }
    }

    fn refund_ix(
        maker: Pubkey,
        mint_a: Pubkey,
        maker_ata_a: Pubkey,
        escrow: Pubkey,
        vault: Pubkey,
    ) -> Instruction {
        // Create the "Refund" instruction to refund tokens back to the maker
        Instruction {
            program_id: anchor_escrow_q2_2026::id(),
            accounts: anchor_escrow_q2_2026::accounts::Refund {
                maker,
                mint_a,
                maker_ata_a,
                escrow,
                vault,
                token_program: TOKEN_PROGRAM_ID,
                system_program: SYSTEM_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: anchor_escrow_q2_2026::instruction::Refund {}.data(),
        }
    }

    fn update_ix(maker: Pubkey, escrow: Pubkey, receive: u64, expiration: i64) -> Instruction {
        // Create the "Update" instruction to change the deal terms of a live escrow
        Instruction {
            program_id: anchor_escrow_q2_2026::id(),
            accounts: anchor_escrow_q2_2026::accounts::Update { maker, escrow }
                .to_account_metas(None),
            data: anchor_escrow_q2_2026::instruction::Update { receive, expiration }.data(),
        }
    }

    // Send a transaction with a single instruction signed by `fee_payer` plus any extra signers
    fn send_ix(
        program: &mut LiteSVM,
        fee_payer: &Keypair,
        extra_signers: &[&Keypair],
        ix: &Instruction,
    ) -> litesvm::types::TransactionResult {
        // Derive a fresh blockhash so that re-sending an identical instruction does not collide
        program.expire_blockhash();
        let message = Message::new(std::slice::from_ref(ix), Some(&fee_payer.pubkey()));
        let recent_blockhash = program.latest_blockhash();

        let mut signers: Vec<&Keypair> = vec![fee_payer];
        signers.extend_from_slice(extra_signers);

        let transaction = Transaction::new(&signers, message, recent_blockhash);

        program.send_transaction(transaction)
    }

    fn assert_custom_error(
        program: &mut LiteSVM,
        fee_payer: &Keypair,
        extra_signers: &[&Keypair],
        ix: &Instruction,
        expected_code: u32,
    ) {
        let result = send_ix(program, fee_payer, extra_signers, ix);
        let err = result.err().expect("expected the transaction to fail");
        let debug = format!("{:?}", err.err);
        assert!(
            debug.contains(&format!("Custom({})", expected_code)),
            "expected custom error {}, got: {}",
            expected_code,
            debug
        );
        msg!("Transaction failed as expected with error {}", debug);
    }

    fn read_escrow(program: &LiteSVM, escrow: &Pubkey) -> anchor_escrow_q2_2026::state::Escrow {
        let escrow_account = program.get_account(escrow).unwrap();
        anchor_escrow_q2_2026::state::Escrow::try_deserialize(&mut escrow_account.data.as_ref())
            .unwrap()
    }

    fn token_amount(program: &LiteSVM, ata: &Pubkey) -> u64 {
        let account = program.get_account(ata).unwrap();
        spl_token::state::Account::unpack(&account.data).unwrap().amount
    }

    #[test]
    fn test_make_and_refund() {
        // Setup the test environment by initializing LiteSVM and creating a payer keypair
        let (mut program, payer) = setup();

        // Get the maker's public key from the payer keypair
        let maker = payer.pubkey();

        // Create two mints (Mint A and Mint B) with 6 decimal places and the maker as the authority
        let (mint_a, mint_b) = create_mints(&mut program, &payer, &maker);

        // Create the maker's associated token account for Mint A
        let maker_ata_a = create_ata(&mut program, &payer, &mint_a, &maker);
        msg!("Maker ATA A: {}\n", maker_ata_a);

        // Derive the PDA for the escrow account using the maker's public key and a seed value
        let seed = 123u64;
        let (escrow, vault) = escrow_pdas(&maker, seed, &mint_a);

        // Mint 1,000 tokens (with 6 decimal places) of Mint A to the maker's associated token account
        mint_tokens(&mut program, &payer, &mint_a, &maker_ata_a, 1000_000_000);

        // Create the "Make" instruction to deposit tokens into the escrow
        let deposit = 10_000_000u64;
        let expiration = NOW + ONE_HOUR;
        let make_ix = make_ix(
            maker,
            mint_a,
            mint_b,
            maker_ata_a,
            escrow,
            vault,
            seed,
            deposit,
            10_000_000,
            expiration,
        );

        // Send the transaction and capture the result
        let tx = send_ix(&mut program, &payer, &[], &make_ix).unwrap();

        // Log transaction details
        msg!("\n\nMake transaction sucessfull");
        msg!("CUs Consumed: {}", tx.compute_units_consumed);
        msg!("Tx Signature: {}", tx.signature);

        // Verify the vault account and escrow account data after the "Make" instruction
        assert_eq!(token_amount(&program, &vault), deposit);
        let vault_account = program.get_account(&vault).unwrap();
        let vault_data = spl_token::state::Account::unpack(&vault_account.data).unwrap();
        assert_eq!(vault_data.owner, escrow);
        assert_eq!(vault_data.mint, mint_a);

        let escrow_data = read_escrow(&program, &escrow);
        assert_eq!(escrow_data.seed, seed);
        assert_eq!(escrow_data.maker, maker);
        assert_eq!(escrow_data.mint_a, mint_a);
        assert_eq!(escrow_data.mint_b, mint_b);
        assert_eq!(escrow_data.receive, 10_000_000);
        assert_eq!(escrow_data.expiration, expiration);

        // The refund must fail before the deadline has passed
        assert_custom_error(
            &mut program,
            &payer,
            &[],
            &refund_ix(maker, mint_a, maker_ata_a, escrow, vault),
            ESCROW_NOT_EXPIRED,
        );

        // Advance the clock past the deadline, then the refund must succeed
        set_clock(&mut program, expiration);

        let refund_ix = refund_ix(maker, mint_a, maker_ata_a, escrow, vault);

        // Send the transaction and capture the result
        let tx = send_ix(&mut program, &payer, &[], &refund_ix).unwrap();

        // Log transaction details
        msg!("\n\nRefund transaction sucessful");
        msg!("CUs Consumed: {}", tx.compute_units_consumed);
        msg!("Tx Signature: {}", tx.signature);

        // The maker got all the tokens back
        assert_eq!(token_amount(&program, &maker_ata_a), 1000_000_000);
        assert!(program.get_account(&escrow).is_none());
        assert!(program.get_account(&vault).is_none());
    }

    #[test]
    fn test_make_and_take() {
        // Setup the test environment by initializing LiteSVM and creating a payer keypair
        let (mut program, payer) = setup();

        // Get the maker's public key from the payer keypair
        let maker = payer.pubkey();
        let taker = Keypair::new();
        program.airdrop(&taker.pubkey(), 1_000_000_000).unwrap();

        // Create two mints (Mint A and Mint B) with 6 decimal places and the maker as the authority
        let (mint_a, mint_b) = create_mints(&mut program, &payer, &maker);

        // Create the associated token accounts for maker and taker
        let maker_ata_a = create_ata(&mut program, &payer, &mint_a, &maker);
        msg!("Maker ATA A: {}\n", maker_ata_a);
        let maker_ata_b = create_ata(&mut program, &payer, &mint_b, &maker);
        msg!("Maker ATA B: {}\n", maker_ata_b);
        let taker_ata_a = create_ata(&mut program, &taker, &mint_a, &taker.pubkey());
        msg!("Taker ATA A: {}\n", taker_ata_a);
        let taker_ata_b = create_ata(&mut program, &taker, &mint_b, &taker.pubkey());
        msg!("Taker ATA B: {}\n", taker_ata_b);

        // Derive the PDA for the escrow account using the maker's public key and a seed value
        let seed = 123u64;
        let (escrow, vault) = escrow_pdas(&maker, seed, &mint_a);

        // Mint 1,000 tokens (with 6 decimal places) of Mint A to the maker's associated token account
        mint_tokens(&mut program, &payer, &mint_a, &maker_ata_a, 1000_000_000);
        // Mint 1,000 tokens (with 6 decimal places) of Mint B to the takers's associated token account
        mint_tokens(&mut program, &payer, &mint_b, &taker_ata_b, 1000_000_000);

        // Create the "Make" instruction to deposit tokens into the escrow
        let deposit = 10_000_000u64;
        let receive = 10_000_000u64;
        let expiration = NOW + ONE_HOUR;
        let make_ix = make_ix(
            maker,
            mint_a,
            mint_b,
            maker_ata_a,
            escrow,
            vault,
            seed,
            deposit,
            receive,
            expiration,
        );

        // Send the transaction and capture the result
        let tx = send_ix(&mut program, &payer, &[], &make_ix).unwrap();

        // Log transaction details
        msg!("\n\nMake transaction sucessfull");
        msg!("CUs Consumed: {}", tx.compute_units_consumed);
        msg!("Tx Signature: {}", tx.signature);

        // Verify the vault account and escrow account data after the "Make" instruction
        assert_eq!(token_amount(&program, &vault), deposit);
        let vault_account = program.get_account(&vault).unwrap();
        let vault_data = spl_token::state::Account::unpack(&vault_account.data).unwrap();
        assert_eq!(vault_data.owner, escrow);
        assert_eq!(vault_data.mint, mint_a);

        let escrow_data = read_escrow(&program, &escrow);
        assert_eq!(escrow_data.seed, seed);
        assert_eq!(escrow_data.maker, maker);
        assert_eq!(escrow_data.mint_a, mint_a);
        assert_eq!(escrow_data.mint_b, mint_b);
        assert_eq!(escrow_data.receive, receive);
        assert_eq!(escrow_data.expiration, expiration);

        // Jump to 30 minutes before the deadline; the take must succeed
        set_clock(&mut program, NOW + ONE_HOUR / 2);

        // Create the "Take" instruction to execute the trade and close the escrow
        let take_ix = take_ix(
            taker.pubkey(),
            maker,
            mint_a,
            mint_b,
            taker_ata_a,
            taker_ata_b,
            maker_ata_b,
            escrow,
            vault,
        );

        // Send the transaction and capture the result
        let tx = send_ix(&mut program, &taker, &[], &take_ix).unwrap();

        // Log transaction details
        msg!("\n\nTake transaction sucessful");
        msg!("CUs Consumed: {}", tx.compute_units_consumed);
        msg!("Tx Signature: {}", tx.signature);

        // The taker received the vault tokens, the maker received the payment
        assert_eq!(token_amount(&program, &taker_ata_a), deposit);
        assert_eq!(token_amount(&program, &maker_ata_b), receive);
        assert!(program.get_account(&escrow).is_none());
        assert!(program.get_account(&vault).is_none());
    }

    #[test]
    fn test_take_fails_after_expiration() {
        // Setup the test environment by initializing LiteSVM and creating a payer keypair
        let (mut program, payer) = setup();

        // Get the maker's public key from the payer keypair
        let maker = payer.pubkey();
        let taker = Keypair::new();
        program.airdrop(&taker.pubkey(), 1_000_000_000).unwrap();

        // Create two mints (Mint A and Mint B) with 6 decimal places and the maker as the authority
        let (mint_a, mint_b) = create_mints(&mut program, &payer, &maker);

        let maker_ata_a = create_ata(&mut program, &payer, &mint_a, &maker);
        let maker_ata_b = create_ata(&mut program, &payer, &mint_b, &maker);
        let taker_ata_a = create_ata(&mut program, &taker, &mint_a, &taker.pubkey());
        let taker_ata_b = create_ata(&mut program, &taker, &mint_b, &taker.pubkey());

        // Derive the PDA for the escrow account using the maker's public key and a seed value
        let seed = 123u64;
        let (escrow, vault) = escrow_pdas(&maker, seed, &mint_a);

        mint_tokens(&mut program, &payer, &mint_a, &maker_ata_a, 1000_000_000);
        mint_tokens(&mut program, &payer, &mint_b, &taker_ata_b, 1000_000_000);

        // Create and send the "Make" transaction
        let expiration = NOW + ONE_HOUR;
        let make_ix = make_ix(
            maker,
            mint_a,
            mint_b,
            maker_ata_a,
            escrow,
            vault,
            seed,
            10_000_000,
            10_000_000,
            expiration,
        );
        send_ix(&mut program, &payer, &[], &make_ix).unwrap();

        // Advance the clock right to the deadline; taking must now fail
        set_clock(&mut program, expiration);

        // Create the "Take" instruction and check it fails with EscrowExpired
        let take_ix = take_ix(
            taker.pubkey(),
            maker,
            mint_a,
            mint_b,
            taker_ata_a,
            taker_ata_b,
            maker_ata_b,
            escrow,
            vault,
        );

        assert_custom_error(
            &mut program,
            &taker,
            &[],
            &take_ix,
            ESCROW_EXPIRED,
        );

        // The escrow and the vault are still intact
        assert!(program.get_account(&escrow).is_some());
        assert_eq!(token_amount(&program, &vault), 10_000_000);
    }

    #[test]
    fn test_update_and_take() {
        // Setup the test environment by initializing LiteSVM and creating a payer keypair
        let (mut program, payer) = setup();

        // Get the maker's public key from the payer keypair
        let maker = payer.pubkey();
        let taker = Keypair::new();
        program.airdrop(&taker.pubkey(), 1_000_000_000).unwrap();

        // Create two mints (Mint A and Mint B) with 6 decimal places and the maker as the authority
        let (mint_a, mint_b) = create_mints(&mut program, &payer, &maker);

        let maker_ata_a = create_ata(&mut program, &payer, &mint_a, &maker);
        let maker_ata_b = create_ata(&mut program, &payer, &mint_b, &maker);
        let taker_ata_a = create_ata(&mut program, &taker, &mint_a, &taker.pubkey());
        let taker_ata_b = create_ata(&mut program, &taker, &mint_b, &taker.pubkey());

        // Derive the PDA for the escrow account using the maker's public key and a seed value
        let seed = 123u64;
        let (escrow, vault) = escrow_pdas(&maker, seed, &mint_a);

        mint_tokens(&mut program, &payer, &mint_a, &maker_ata_a, 1000_000_000);
        mint_tokens(&mut program, &payer, &mint_b, &taker_ata_b, 1000_000_000);

        // Create and send the "Make" transaction
        let expiration = NOW + ONE_HOUR;
        let make_ix = make_ix(
            maker,
            mint_a,
            mint_b,
            maker_ata_a,
            escrow,
            vault,
            seed,
            10_000_000,
            10_000_000,
            expiration,
        );
        send_ix(&mut program, &payer, &[], &make_ix).unwrap();

        // Update the deal terms: new receive amount and a later deadline
        let new_receive = 25_000_000u64;
        let new_expiration = NOW + 2 * ONE_HOUR;
        let update_ix = update_ix(maker, escrow, new_receive, new_expiration);
        let tx = send_ix(&mut program, &payer, &[], &update_ix).unwrap();

        msg!("\n\nUpdate transaction sucessful");
        msg!("CUs Consumed: {}", tx.compute_units_consumed);
        msg!("Tx Signature: {}", tx.signature);

        // Verify the escrow state was updated
        let escrow_data = read_escrow(&program, &escrow);
        assert_eq!(escrow_data.receive, new_receive);
        assert_eq!(escrow_data.expiration, new_expiration);
        assert_eq!(escrow_data.maker, maker);
        assert_eq!(escrow_data.seed, seed);

        // Take with the updated terms: the maker must receive the new amount
        set_clock(&mut program, NOW + 90 * 60);
        let take_ix = take_ix(
            taker.pubkey(),
            maker,
            mint_a,
            mint_b,
            taker_ata_a,
            taker_ata_b,
            maker_ata_b,
            escrow,
            vault,
        );
        send_ix(&mut program, &taker, &[], &take_ix).unwrap();

        assert_eq!(token_amount(&program, &maker_ata_b), new_receive);
        assert_eq!(token_amount(&program, &taker_ata_a), 10_000_000);
        assert!(program.get_account(&escrow).is_none());
        assert!(program.get_account(&vault).is_none());
    }

    #[test]
    fn test_update_fails_after_expiration() {
        // Setup the test environment by initializing LiteSVM and creating a payer keypair
        let (mut program, payer) = setup();

        // Get the maker's public key from the payer keypair
        let maker = payer.pubkey();

        // Create two mints (Mint A and Mint B) with 6 decimal places and the maker as the authority
        let (mint_a, mint_b) = create_mints(&mut program, &payer, &maker);

        let maker_ata_a = create_ata(&mut program, &payer, &mint_a, &maker);

        // Derive the PDA for the escrow account using the maker's public key and a seed value
        let seed = 123u64;
        let (escrow, vault) = escrow_pdas(&maker, seed, &mint_a);

        mint_tokens(&mut program, &payer, &mint_a, &maker_ata_a, 1000_000_000);

        // Create and send the "Make" transaction
        let expiration = NOW + ONE_HOUR;
        let make_ix = make_ix(
            maker,
            mint_a,
            mint_b,
            maker_ata_a,
            escrow,
            vault,
            seed,
            10_000_000,
            10_000_000,
            expiration,
        );
        send_ix(&mut program, &payer, &[], &make_ix).unwrap();

        // Advance the clock to the deadline; updating must now fail
        set_clock(&mut program, expiration);

        let update_ix = update_ix(maker, escrow, 20_000_000, expiration + ONE_HOUR);
        assert_custom_error(
            &mut program,
            &payer,
            &[],
            &update_ix,
            ESCROW_EXPIRED,
        );
    }

    #[test]
    fn test_update_fails_with_past_expiration() {
        // Setup the test environment by initializing LiteSVM and creating a payer keypair
        let (mut program, payer) = setup();

        // Get the maker's public key from the payer keypair
        let maker = payer.pubkey();

        // Create two mints (Mint A and Mint B) with 6 decimal places and the maker as the authority
        let (mint_a, mint_b) = create_mints(&mut program, &payer, &maker);

        let maker_ata_a = create_ata(&mut program, &payer, &mint_a, &maker);

        // Derive the PDA for the escrow account using the maker's public key and a seed value
        let seed = 123u64;
        let (escrow, vault) = escrow_pdas(&maker, seed, &mint_a);

        mint_tokens(&mut program, &payer, &mint_a, &maker_ata_a, 1000_000_000);

        // Create and send the "Make" transaction
        let expiration = NOW + ONE_HOUR;
        let make_ix = make_ix(
            maker,
            mint_a,
            mint_b,
            maker_ata_a,
            escrow,
            vault,
            seed,
            10_000_000,
            10_000_000,
            expiration,
        );
        send_ix(&mut program, &payer, &[], &make_ix).unwrap();

        // The new deadline must be in the future: setting it to the past must fail
        let update_ix = update_ix(maker, escrow, 20_000_000, NOW - 1);
        assert_custom_error(
            &mut program,
            &payer,
            &[],
            &update_ix,
            INVALID_EXPIRATION,
        );
    }

    #[test]
    fn test_make_fails_with_past_expiration() {
        // Setup the test environment by initializing LiteSVM and creating a payer keypair
        let (mut program, payer) = setup();

        // Get the maker's public key from the payer keypair
        let maker = payer.pubkey();

        // Create two mints (Mint A and Mint B) with 6 decimal places and the maker as the authority
        let (mint_a, mint_b) = create_mints(&mut program, &payer, &maker);

        let maker_ata_a = create_ata(&mut program, &payer, &mint_a, &maker);

        // Derive the PDA for the escrow account using the maker's public key and a seed value
        let seed = 123u64;
        let (escrow, vault) = escrow_pdas(&maker, seed, &mint_a);

        mint_tokens(&mut program, &payer, &mint_a, &maker_ata_a, 1000_000_000);

        // Creating an escrow with an expiration in the past must fail
        let make_ix = make_ix(
            maker,
            mint_a,
            mint_b,
            maker_ata_a,
            escrow,
            vault,
            seed,
            10_000_000,
            10_000_000,
            NOW - 1,
        );

        assert_custom_error(
            &mut program,
            &payer,
            &[],
            &make_ix,
            INVALID_EXPIRATION,
        );

        // No escrow or vault was created
        assert!(program.get_account(&escrow).is_none());
        assert!(program.get_account(&vault).is_none());
    }
}
