const express = require('express');
const Database = require('better-sqlite3');
const multer = require('multer');
const cors = require('cors');
const path = require('path');
const fs = require('fs');

const app = express();
const PORT = 3001;

// ── Setup ──
app.use(cors());
app.use(express.json());
app.use('/uploads', express.static(path.join(__dirname, 'uploads')));
fs.mkdirSync(path.join(__dirname, 'uploads/pfp'), { recursive: true });

// ── Database ──
const db = new Database(path.join(__dirname, 'sames.db'));
db.pragma('journal_mode = WAL');

db.exec(`
  CREATE TABLE IF NOT EXISTS profiles (
    wallet TEXT PRIMARY KEY,
    username TEXT,
    pfp_url TEXT,
    bio TEXT DEFAULT '',
    website TEXT DEFAULT '',
    twitter TEXT DEFAULT '',
    telegram TEXT DEFAULT '',
    created_at INTEGER DEFAULT (unixepoch()),
    updated_at INTEGER DEFAULT (unixepoch())
  );

  CREATE TABLE IF NOT EXISTS chat_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    token_address TEXT NOT NULL,
    wallet TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at INTEGER DEFAULT (unixepoch())
  );

  CREATE INDEX IF NOT EXISTS idx_chat_token ON chat_messages(token_address, created_at DESC);
  CREATE INDEX IF NOT EXISTS idx_chat_wallet ON chat_messages(wallet);
`);

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
  limits: { fileSize: 2 * 1024 * 1024 }, // 2MB
  fileFilter: (req, file, cb) => {
    const allowed = ['image/jpeg', 'image/png', 'image/gif', 'image/webp'];
    cb(null, allowed.includes(file.mimetype));
  }
});

// ══════════════════════════════════════════
// PROFILES
// ══════════════════════════════════════════

// Get profile
app.get('/api/profile/:wallet', (req, res) => {
  const profile = db.prepare('SELECT * FROM profiles WHERE wallet = ?').get(req.params.wallet);
  if (!profile) return res.json({ wallet: req.params.wallet, username: null, pfp_url: null });
  res.json(profile);
});

// Get multiple profiles (for chat)
app.post('/api/profiles/batch', (req, res) => {
  const { wallets } = req.body;
  if (!wallets || !Array.isArray(wallets)) return res.json([]);
  const placeholders = wallets.map(() => '?').join(',');
  const profiles = db.prepare(`SELECT * FROM profiles WHERE wallet IN (${placeholders})`).all(...wallets);
  res.json(profiles);
});

// Update profile
app.post('/api/profile/:wallet', (req, res) => {
  const { username, bio, website, twitter, telegram } = req.body;
  const wallet = req.params.wallet;

  db.prepare(`
    INSERT INTO profiles (wallet, username, bio, website, twitter, telegram)
    VALUES (?, ?, ?, ?, ?, ?)
    ON CONFLICT(wallet) DO UPDATE SET
      username = COALESCE(?, username),
      bio = COALESCE(?, bio),
      website = COALESCE(?, website),
      twitter = COALESCE(?, twitter),
      telegram = COALESCE(?, telegram),
      updated_at = unixepoch()
  `).run(wallet, username, bio || '', website || '', twitter || '', telegram || '',
         username, bio, website, twitter, telegram);

  res.json({ ok: true });
});

// Upload PFP
app.post('/api/profile/:wallet/pfp', upload.single('pfp'), (req, res) => {
  if (!req.file) return res.status(400).json({ error: 'No file uploaded' });
  const pfp_url = `/uploads/pfp/${req.file.filename}`;

  db.prepare(`
    INSERT INTO profiles (wallet, pfp_url)
    VALUES (?, ?)
    ON CONFLICT(wallet) DO UPDATE SET pfp_url = ?, updated_at = unixepoch()
  `).run(req.params.wallet, pfp_url, pfp_url);

  res.json({ ok: true, pfp_url });
});

// ══════════════════════════════════════════
// CHAT
// ══════════════════════════════════════════

// Get messages for a token
app.get('/api/chat/:tokenAddress', (req, res) => {
  const limit = Math.min(parseInt(req.query.limit) || 50, 200);
  const before = req.query.before ? parseInt(req.query.before) : null;

  let messages;
  if (before) {
    messages = db.prepare(`
      SELECT m.*, p.username, p.pfp_url
      FROM chat_messages m
      LEFT JOIN profiles p ON m.wallet = p.wallet
      WHERE m.token_address = ? AND m.id < ?
      ORDER BY m.created_at DESC LIMIT ?
    `).all(req.params.tokenAddress, before, limit);
  } else {
    messages = db.prepare(`
      SELECT m.*, p.username, p.pfp_url
      FROM chat_messages m
      LEFT JOIN profiles p ON m.wallet = p.wallet
      WHERE m.token_address = ?
      ORDER BY m.created_at DESC LIMIT ?
    `).all(req.params.tokenAddress, limit);
  }

  res.json(messages.reverse()); // oldest first
});

// Post a message
app.post('/api/chat/:tokenAddress', (req, res) => {
  const { wallet, message } = req.body;
  if (!wallet || !message || !message.trim()) {
    return res.status(400).json({ error: 'wallet and message required' });
  }
  if (message.length > 500) {
    return res.status(400).json({ error: 'Message too long (max 500 chars)' });
  }

  const result = db.prepare(`
    INSERT INTO chat_messages (token_address, wallet, message)
    VALUES (?, ?, ?)
  `).run(req.params.tokenAddress, wallet, message.trim());

  const msg = db.prepare(`
    SELECT m.*, p.username, p.pfp_url
    FROM chat_messages m
    LEFT JOIN profiles p ON m.wallet = p.wallet
    WHERE m.id = ?
  `).get(result.lastInsertRowid);

  res.json(msg);
});

// ── Health ──
app.get('/api/health', (req, res) => {
  const stats = {
    profiles: db.prepare('SELECT COUNT(*) as count FROM profiles').get().count,
    messages: db.prepare('SELECT COUNT(*) as count FROM chat_messages').get().count,
  };
  res.json({ ok: true, ...stats });
});

// ── Start ──
app.listen(PORT, '0.0.0.0', () => {
  console.log(`SAMES API running on port ${PORT}`);
});
