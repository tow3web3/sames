const express = require('express');
const { Pool } = require('pg');
const multer = require('multer');
const cors = require('cors');
const path = require('path');
const fs = require('fs');
const nacl = require('tweetnacl');
const bs58 = require('bs58');
const rateLimit = require('express-rate-limit');

const app = express();
const PORT = 3001;
const DATABASE_URL = process.env.DATABASE_URL;
if (!DATABASE_URL) { console.error('DATABASE_URL not set'); process.exit(1); }

// ── Setup ──
app.use(cors());
app.use(express.json());

// ── Rate Limiting ──
const apiLimiter = rateLimit({ windowMs: 60 * 1000, max: 60, message: { error: 'Too many requests' } });
const chatLimiter = rateLimit({ windowMs: 60 * 1000, max: 20, message: { error: 'Too many messages' } });
app.use('/api/', apiLimiter);

// ── Wallet Signature Verification ──
function verifySignature(wallet, message, signature) {
  try {
    const pubkey = bs58.decode(wallet);
    const sig = bs58.decode(signature);
    const msg = new TextEncoder().encode(message);
    return nacl.sign.detached.verify(msg, sig, pubkey);
  } catch(e) { return false; }
}

// Middleware: verify wallet ownership for write operations
// Set REQUIRE_SIG=1 to enforce (disabled by default during dev)
const REQUIRE_SIG = process.env.REQUIRE_SIG === '1';
function requireWalletSig(req, res, next) {
  if (!REQUIRE_SIG) return next();
  const wallet = req.body?.wallet || req.params?.wallet;
  const signature = req.headers['x-wallet-signature'];
  const message = req.headers['x-wallet-message'];
  if (!wallet || !signature || !message) {
    return res.status(401).json({ error: 'Missing wallet signature. Include x-wallet-signature and x-wallet-message headers.' });
  }
  if (!message.includes(wallet)) {
    return res.status(401).json({ error: 'Signature message must contain wallet address' });
  }
  if (!verifySignature(wallet, message, signature)) {
    return res.status(401).json({ error: 'Invalid wallet signature' });
  }
  next();
}
app.use('/uploads', express.static(path.join(__dirname, 'uploads')));
// Serve frontend files from parent directory
app.use(express.static(path.join(__dirname, '..')));
fs.mkdirSync(path.join(__dirname, 'uploads/pfp'), { recursive: true });
fs.mkdirSync(path.join(__dirname, 'uploads/tokens'), { recursive: true });

// ── Database ──
const pool = new Pool({ connectionString: DATABASE_URL });

// ── Image Upload ──
const storage = multer.diskStorage({
  destination: (req, file, cb) => cb(null, path.join(__dirname, 'uploads/pfp')),
  filename: (req, file, cb) => {
    const ext = path.extname(file.originalname) || '.png';
    cb(null, req.params.wallet + ext);
  }
});
const upload = multer({
  storage,
  limits: { fileSize: 2 * 1024 * 1024 },
  fileFilter: (req, file, cb) => {
    cb(null, ['image/jpeg', 'image/png', 'image/gif', 'image/webp'].includes(file.mimetype));
  }
});

// ══════════════════════════════════════════
// PROFILES
// ══════════════════════════════════════════

app.get('/api/profile/:wallet', async (req, res) => {
  try {
    const { rows } = await pool.query('SELECT * FROM profiles WHERE wallet = $1', [req.params.wallet]);
    if (!rows.length) return res.json({ wallet: req.params.wallet, username: null, pfp_url: null });
    res.json(rows[0]);
  } catch(e) { res.status(500).json({ error: e.message }); }
});

app.post('/api/profiles/batch', async (req, res) => {
  const { wallets } = req.body;
  if (!wallets?.length) return res.json([]);
  try {
    const placeholders = wallets.map((_, i) => `$${i + 1}`).join(',');
    const { rows } = await pool.query(`SELECT * FROM profiles WHERE wallet IN (${placeholders})`, wallets);
    res.json(rows);
  } catch(e) { res.status(500).json({ error: e.message }); }
});

app.post('/api/profile/:wallet', requireWalletSig, async (req, res) => {
  const { username, bio, website, twitter, telegram } = req.body;
  const wallet = req.params.wallet;
  try {
    await pool.query(`
      INSERT INTO profiles (wallet, username, bio, website, twitter, telegram)
      VALUES ($1, $2, $3, $4, $5, $6)
      ON CONFLICT(wallet) DO UPDATE SET
        username = COALESCE($2, profiles.username),
        bio = COALESCE($3, profiles.bio),
        website = COALESCE($4, profiles.website),
        twitter = COALESCE($5, profiles.twitter),
        telegram = COALESCE($6, profiles.telegram),
        updated_at = NOW()
    `, [wallet, username, bio || '', website || '', twitter || '', telegram || '']);
    res.json({ ok: true });
  } catch(e) { res.status(500).json({ error: e.message }); }
});

app.post('/api/profile/:wallet/pfp', requireWalletSig, upload.single('pfp'), async (req, res) => {
  if (!req.file) return res.status(400).json({ error: 'No file' });
  const pfp_url = `/uploads/pfp/${req.file.filename}`;
  try {
    await pool.query(`
      INSERT INTO profiles (wallet, pfp_url) VALUES ($1, $2)
      ON CONFLICT(wallet) DO UPDATE SET pfp_url = $2, updated_at = NOW()
    `, [req.params.wallet, pfp_url]);
    res.json({ ok: true, pfp_url });
  } catch(e) { res.status(500).json({ error: e.message }); }
});

// ══════════════════════════════════════════
// TOKEN METADATA
// ══════════════════════════════════════════

const tokenStorage = multer.diskStorage({
  destination: (req, file, cb) => cb(null, path.join(__dirname, 'uploads/tokens')),
  filename: (req, file, cb) => {
    const ext = path.extname(file.originalname) || '.png';
    cb(null, req.params.mint + ext);
  }
});
const tokenUpload = multer({
  storage: tokenStorage,
  limits: { fileSize: 5 * 1024 * 1024 },
  fileFilter: (req, file, cb) => {
    cb(null, ['image/jpeg', 'image/png', 'image/gif', 'image/webp'].includes(file.mimetype));
  }
});

app.get('/api/token/:mint', async (req, res) => {
  try {
    const { rows } = await pool.query('SELECT * FROM token_metadata WHERE mint = $1', [req.params.mint]);
    if (!rows.length) return res.json(null);
    res.json(rows[0]);
  } catch(e) { res.status(500).json({ error: e.message }); }
});

app.post('/api/tokens/batch', async (req, res) => {
  const { mints } = req.body;
  if (!mints?.length) return res.json([]);
  try {
    const placeholders = mints.map((_, i) => `$${i + 1}`).join(',');
    const { rows } = await pool.query(`SELECT * FROM token_metadata WHERE mint IN (${placeholders})`, mints);
    res.json(rows);
  } catch(e) { res.status(500).json({ error: e.message }); }
});

app.post('/api/token/:mint', tokenUpload.single('image'), async (req, res) => {
  const { launch_address, description, website, telegram, twitter, creator_wallet } = req.body;
  const image_url = req.file ? `/uploads/tokens/${req.file.filename}` : null;
  try {
    await pool.query(`
      INSERT INTO token_metadata (mint, launch_address, image_url, description, website, telegram, twitter, creator_wallet)
      VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
      ON CONFLICT(mint) DO UPDATE SET
        launch_address = COALESCE($2, token_metadata.launch_address),
        image_url = COALESCE($3, token_metadata.image_url),
        description = COALESCE($4, token_metadata.description),
        website = COALESCE($5, token_metadata.website),
        telegram = COALESCE($6, token_metadata.telegram),
        twitter = COALESCE($7, token_metadata.twitter),
        updated_at = NOW()
    `, [req.params.mint, launch_address || '', image_url, description || '', website || '', telegram || '', twitter || '', creator_wallet || '']);
    res.json({ ok: true, image_url });
  } catch(e) { res.status(500).json({ error: e.message }); }
});

// ══════════════════════════════════════════
// CHAT
// ══════════════════════════════════════════

app.get('/api/chat/:tokenAddress', async (req, res) => {
  const limit = Math.min(parseInt(req.query.limit) || 50, 200);
  const before = req.query.before ? parseInt(req.query.before) : null;
  try {
    let query, params;
    if (before) {
      query = `SELECT m.*, p.username, p.pfp_url FROM chat_messages m
               LEFT JOIN profiles p ON m.wallet = p.wallet
               WHERE m.token_address = $1 AND m.id < $2
               ORDER BY m.created_at DESC LIMIT $3`;
      params = [req.params.tokenAddress, before, limit];
    } else {
      query = `SELECT m.*, p.username, p.pfp_url FROM chat_messages m
               LEFT JOIN profiles p ON m.wallet = p.wallet
               WHERE m.token_address = $1
               ORDER BY m.created_at DESC LIMIT $2`;
      params = [req.params.tokenAddress, limit];
    }
    const { rows } = await pool.query(query, params);
    res.json(rows.reverse());
  } catch(e) { res.status(500).json({ error: e.message }); }
});

app.post('/api/chat/:tokenAddress', chatLimiter, requireWalletSig, async (req, res) => {
  const { wallet, message } = req.body;
  if (!wallet || !message?.trim()) return res.status(400).json({ error: 'wallet and message required' });
  if (message.length > 500) return res.status(400).json({ error: 'Max 500 chars' });
  try {
    const { rows } = await pool.query(`
      WITH inserted AS (
        INSERT INTO chat_messages (token_address, wallet, message)
        VALUES ($1, $2, $3) RETURNING *
      )
      SELECT i.*, p.username, p.pfp_url FROM inserted i
      LEFT JOIN profiles p ON i.wallet = p.wallet
    `, [req.params.tokenAddress, wallet, message.trim()]);
    res.json(rows[0]);
  } catch(e) { res.status(500).json({ error: e.message }); }
});

// ══════════════════════════════════════════
// TRADES & PRICE HISTORY
// ══════════════════════════════════════════

// Record a trade
app.post('/api/trade/:tokenAddress', async (req, res) => {
  const { wallet, tx_sig, trade_type, sol_amount, token_amount, price_lamports } = req.body;
  if (!wallet || !tx_sig || !trade_type) return res.status(400).json({ error: 'Missing fields' });
  try {
    await pool.query(`
      INSERT INTO trades (token_address, tx_sig, wallet, trade_type, sol_amount, token_amount, price_lamports)
      VALUES ($1, $2, $3, $4, $5, $6, $7)
      ON CONFLICT(tx_sig) DO NOTHING
    `, [req.params.tokenAddress, tx_sig, wallet, trade_type, sol_amount || 0, token_amount || 0, price_lamports || 0]);
    res.json({ ok: true });
  } catch(e) { res.status(500).json({ error: e.message }); }
});

// Get trades for a token
app.get('/api/trades/:tokenAddress', async (req, res) => {
  const limit = Math.min(parseInt(req.query.limit) || 100, 500);
  try {
    const { rows } = await pool.query(`
      SELECT t.*, p.username, p.pfp_url FROM trades t
      LEFT JOIN profiles p ON t.wallet = p.wallet
      WHERE t.token_address = $1
      ORDER BY t.created_at ASC LIMIT $2
    `, [req.params.tokenAddress, limit]);
    res.json(rows);
  } catch(e) { res.status(500).json({ error: e.message }); }
});

// Record price snapshot
app.post('/api/snapshot/:tokenAddress', async (req, res) => {
  const { price_lamports, tokens_sold, sol_collected } = req.body;
  try {
    await pool.query(`
      INSERT INTO price_snapshots (token_address, price_lamports, tokens_sold, sol_collected)
      VALUES ($1, $2, $3, $4)
    `, [req.params.tokenAddress, price_lamports, tokens_sold || 0, sol_collected || 0]);
    res.json({ ok: true });
  } catch(e) { res.status(500).json({ error: e.message }); }
});

// Get price history for a token
app.get('/api/prices/:tokenAddress', async (req, res) => {
  const limit = Math.min(parseInt(req.query.limit) || 200, 1000);
  try {
    const { rows } = await pool.query(`
      SELECT * FROM price_snapshots
      WHERE token_address = $1
      ORDER BY created_at ASC LIMIT $2
    `, [req.params.tokenAddress, limit]);
    res.json(rows);
  } catch(e) { res.status(500).json({ error: e.message }); }
});

// ── Health ──
app.get('/api/health', async (req, res) => {
  try {
    const profiles = (await pool.query('SELECT COUNT(*) FROM profiles')).rows[0].count;
    const messages = (await pool.query('SELECT COUNT(*) FROM chat_messages')).rows[0].count;
    res.json({ ok: true, db: 'neon-postgres', profiles: +profiles, messages: +messages });
  } catch(e) { res.status(500).json({ ok: false, error: e.message }); }
});

app.listen(PORT, '0.0.0.0', () => console.log(`SAMES API running on port ${PORT} (Neon PostgreSQL)`));
