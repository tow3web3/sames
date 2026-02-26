const express = require('express');
const { Pool } = require('pg');
const multer = require('multer');
const cors = require('cors');
const path = require('path');
const fs = require('fs');

const app = express();
const PORT = 3001;
const DATABASE_URL = process.env.DATABASE_URL;
if (!DATABASE_URL) { console.error('DATABASE_URL not set'); process.exit(1); }

// ── Setup ──
app.use(cors());
app.use(express.json());
app.use('/uploads', express.static(path.join(__dirname, 'uploads')));
fs.mkdirSync(path.join(__dirname, 'uploads/pfp'), { recursive: true });

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

app.post('/api/profile/:wallet', async (req, res) => {
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

app.post('/api/profile/:wallet/pfp', upload.single('pfp'), async (req, res) => {
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

app.post('/api/chat/:tokenAddress', async (req, res) => {
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

// ── Health ──
app.get('/api/health', async (req, res) => {
  try {
    const profiles = (await pool.query('SELECT COUNT(*) FROM profiles')).rows[0].count;
    const messages = (await pool.query('SELECT COUNT(*) FROM chat_messages')).rows[0].count;
    res.json({ ok: true, db: 'neon-postgres', profiles: +profiles, messages: +messages });
  } catch(e) { res.status(500).json({ ok: false, error: e.message }); }
});

app.listen(PORT, '0.0.0.0', () => console.log(`SAMES API running on port ${PORT} (Neon PostgreSQL)`));
