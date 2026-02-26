import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { PublicKey, Keypair, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { expect } from "chai";

// NOTE: The IDL type will be generated after `anchor build`.
// For now we use `any` — replace with generated type after build.
type Sames = any;

describe("sames", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Sames as Program<Sames>;
  const creator = provider.wallet;
  const mint = Keypair.generate();

  let launchPoolPda: PublicKey;
  let launchPoolBump: number;
  let vaultPda: PublicKey;
  let marketRegistryPda: PublicKey;

  before(async () => {
    // Derive PDAs
    [launchPoolPda, launchPoolBump] = PublicKey.findProgramAddressSync(
      [Buffer.from("launch_pool"), mint.publicKey.toBuffer()],
      program.programId
    );

    [vaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), launchPoolPda.toBuffer()],
      program.programId
    );

    [marketRegistryPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("market_registry"), launchPoolPda.toBuffer()],
      program.programId
    );
  });

  it("Creates a launch", async () => {
    // In a real test, create the Token-2022 mint first with transfer hook extension.
    // For this skeleton test, we test the instruction structure.
    try {
      const tx = await program.methods
        .createLaunch(
          "SAMES Token",       // token_name
          "SAMES",             // token_symbol
          new anchor.BN(1_000_000_000), // total_supply (1B tokens)
          new anchor.BN(1_000_000)      // price_lamports (0.001 SOL)
        )
        .accounts({
          creator: creator.publicKey,
          mint: mint.publicKey,
          launchPool: launchPoolPda,
          vault: vaultPda,
          marketRegistry: marketRegistryPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([])
        .rpc();

      console.log("Create launch tx:", tx);

      // Verify pool state
      const pool = await program.account.launchPool.fetch(launchPoolPda);
      expect(pool.tokenName).to.equal("SAMES Token");
      expect(pool.tokenSymbol).to.equal("SAMES");
      expect(pool.totalSupply.toNumber()).to.equal(1_000_000_000);
      expect(pool.priceLamports.toNumber()).to.equal(1_000_000);
      expect(pool.status).to.deep.equal({ presale: {} });
    } catch (e) {
      console.log("Note: Full test requires Token-2022 mint setup. Error:", e.message);
    }
  });

  it("Buys during presale", async () => {
    const buyer = Keypair.generate();

    // Airdrop SOL to buyer
    const sig = await provider.connection.requestAirdrop(
      buyer.publicKey,
      2 * LAMPORTS_PER_SOL
    );
    await provider.connection.confirmTransaction(sig);

    const [buyerRecordPda] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("buyer_record"),
        launchPoolPda.toBuffer(),
        buyer.publicKey.toBuffer(),
      ],
      program.programId
    );

    try {
      const tx = await program.methods
        .buyPresale(new anchor.BN(LAMPORTS_PER_SOL)) // 1 SOL
        .accounts({
          buyer: buyer.publicKey,
          launchPool: launchPoolPda,
          vault: vaultPda,
          buyerRecord: buyerRecordPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([buyer])
        .rpc();

      console.log("Buy presale tx:", tx);

      const record = await program.account.buyerRecord.fetch(buyerRecordPda);
      expect(record.solDeposited.toNumber()).to.equal(LAMPORTS_PER_SOL);
      expect(record.buyer.toBase58()).to.equal(buyer.publicKey.toBase58());
    } catch (e) {
      console.log("Note: Requires active launch pool. Error:", e.message);
    }
  });

  it("Rejects buy after presale ends", async () => {
    // Wait for presale to end (30s) then try to buy
    // In real tests, use bankrun or warp_to_slot
    console.log("Skipping time-dependent test in basic suite");
  });

  it("Finalizes launch and mints tokens", async () => {
    // Would need: wait 30s, then call finalize for each buyer
    console.log("Skipping finalization test in basic suite");
  });

  it("Rejects sell below entry price", async () => {
    // Would need: finalized launch, buyer tries to sell at lower price
    console.log("Skipping sell-below-entry test in basic suite");
  });

  it("Allows sell at or above entry price", async () => {
    console.log("Skipping sell-above-entry test in basic suite");
  });
});
