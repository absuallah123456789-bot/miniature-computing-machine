import { api, listen } from './api.js';
import { openCropper } from './crop.js';
import { createLiveClassifier, runReview, CLASSES } from './review.js';
import { h, btn, toast, setAvatar, showModal, closeModal, isOpen, closeAllExcept } from './ui.js';

const $ = (id) => document.getElementById(id);
const PIECE_DIR = 'assets/pieces/'; // files: white|black + pawn|knight|bishop|rook|queen|king + .png
const pieceSrc = (color, kind) => `${PIECE_DIR}${color}${kind}.png`;
const PROMO_CHAR = { queen: 'q', rook: 'r', bishop: 'b', knight: 'n' };
const sqName = (sq) => 'abcdefgh'[sq & 7] + ((sq >> 3) + 1);
const MAX_CHAT = 500;
const reducedMotion = () => window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

const st = {
  profile: null,
  peers: [],
  selected: null,
  pending: null,        // cropped avatar {png, thumb, dataUrl} not yet saved
  session: null,
  sel: null,            // selected square on board
  drag: null,           // active drag {from, ghost, moved, wasSel, last:{x,y}}
  anim: null,           // animation hint for the next render
  premoveQueue: [],
  annot: { arrows: [], squares: [] }, // PC-only arrows / square highlights (right mouse button)
  arrowDraft: null,
  lowBuzzed: false,
  moveClasses: [],
  classifiedPly: 0,
  classifying: false,
  rewindRoutes: null,
  liveClassifier: null,
  classifierReady: null,
  classifierLoaded: false,
  classifierGame: null,
  chat: [],
  unread: 0,
  endShownFor: 0,
  popupOpen: false,
  clockSnap: null,
  viewPly: 0,
  checkAnimSq: null,
  connectionWasLost: false,
  settings: { type: '3+2', initial: 10, bonus: 0, color: 'random', boardTheme: 'classic', moveStyle: 'both', classifications: 'none', autoQueen: false },
};

// ---------------- game settings (time control + colour) ----------------
const TYPE_LABEL = { '1+0': 'Bullet 1', '1+1': 'Bullet 1+1', '2+1': 'Bullet 2+1', '3+0': 'Blitz 3', '3+2': 'Blitz 3+2', '5+0': 'Blitz 5', '10+0': 'Rapid 10', '10+5': 'Rapid 10+5', '15+10': 'Rapid 15+10' };
const COLOR_LABEL = { white: 'White', black: 'Black', random: 'Random' };
const BOARD_THEMES = {
  classic: { name: 'Classic', light: '#e6dcc3', dark: '#4f7a7d' },
  forest: { name: 'Forest', light: '#eeeed2', dark: '#769656' },
  ocean: { name: 'Ocean', light: '#dee3e6', dark: '#506d8a' },
  plum: { name: 'Plum', light: '#f0d9e8', dark: '#8b5a83' },
  slate: { name: 'Slate', light: '#c9d1d9', dark: '#596b7a' },
  walnut: { name: 'Walnut', light: '#f0d9b5', dark: '#b58863' },
  midnight: { name: 'Midnight', light: '#9aa5b8', dark: '#2f3b52' },
  sunset: { name: 'Sunset', light: '#fbe3c8', dark: '#d9776a' },
  mint: { name: 'Mint', light: '#e3f1ea', dark: '#5fa48a' },
  latte: { name: 'Catppuccin Latte', light: '#eff1f5', dark: '#7287fd' },
  frappe: { name: 'Catppuccin Frappé', light: '#c6d0f5', dark: '#626880' },
  macchiato: { name: 'Catppuccin Macchiato', light: '#cad3f5', dark: '#5b6078' },
  mocha: { name: 'Catppuccin Mocha', light: '#bac2de', dark: '#45475a' },
};
const MOVE_CLASS_META = {
  blunder: { label: 'Blunder' },
  miss: { label: 'Miss' },
  mistake: { label: 'Mistake' },
  inaccuracy: { label: 'Inaccuracy' },
  book: { label: 'Book' },
  excellent: { label: 'Excellent' },
  best: { label: 'Best' },
  good: { label: 'Good' },
  great: { label: 'Great' },
  brilliant: { label: 'Brilliant' },
  forced: { label: 'Forced' },
};

// ---------------- vibration (Android) ----------------
const IS_ANDROID = /Android/i.test(navigator.userAgent);
function buzz(pattern) {
  if (!IS_ANDROID || !navigator.vibrate) return;
  try { navigator.vibrate(pattern); } catch { /* ignore */ }
}

// ---------------- arrows & square highlights (PC / mouse only) ----------------
const ANNOT_COLORS = { green: '#15a02b', red: '#d9382b', blue: '#2f7de1', yellow: '#e0b21a' };
const SVGNS = 'http://www.w3.org/2000/svg';
const annotColor = (e) => (e.shiftKey ? 'red' : e.ctrlKey || e.metaKey ? 'blue' : e.altKey ? 'yellow' : 'green');
const emptyAnnot = () => ({ arrows: [], squares: [] });

function svgEl(tag, attrs) {
  const el = document.createElementNS(SVGNS, tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, String(v));
  return el;
}

function arrowOverlay(flip, arrows) {
  const svg = svgEl('svg', { class: 'board-overlay', viewBox: '0 0 8 8', 'aria-hidden': 'true' });
  for (const a of arrows) {
    const f = gridPos(a.from, flip), t = gridPos(a.to, flip);
    const x1 = f.col + 0.5, y1 = f.row + 0.5, x2 = t.col + 0.5, y2 = t.row + 0.5;
    const len = Math.hypot(x2 - x1, y2 - y1);
    if (!len) continue;
    const ux = (x2 - x1) / len, uy = (y2 - y1) / len;
    const tipX = x2 - ux * 0.1, tipY = y2 - uy * 0.1;
    const baseX = tipX - ux * 0.4, baseY = tipY - uy * 0.4;
    const color = ANNOT_COLORS[a.color] || ANNOT_COLORS.green;
    const g = svgEl('g', { opacity: '0.8', fill: color, stroke: color });
    g.append(svgEl('line', { x1: x1 + ux * 0.12, y1: y1 + uy * 0.12, x2: baseX + ux * 0.05, y2: baseY + uy * 0.05, 'stroke-width': 0.16, 'stroke-linecap': 'round' }));
    g.append(svgEl('polygon', { points: `${tipX},${tipY} ${baseX - uy * 0.22},${baseY + ux * 0.22} ${baseX + uy * 0.22},${baseY - ux * 0.22}`, 'stroke-width': 0.02, 'stroke-linejoin': 'round' }));
    svg.append(g);
  }
  return svg;
}

function overlayArrows() {
  const list = [...st.annot.arrows];
  const d = st.arrowDraft;
  if (d && d.to !== d.from) list.push({ from: d.from, to: d.to, color: d.color });
  return list;
}

function refreshOverlay() {
  const s = st.session;
  if (!s) return;
  const el = $('board');
  const next = arrowOverlay(isFlipped(s), overlayArrows());
  const old = el.querySelector('.board-overlay');
  if (old) old.replaceWith(next); else el.append(next);
}

function redrawBoard() { if (st.session) renderBoard(positionAt(st.session)); }

function startAnnot(e) {
  const s = st.session;
  if (!s) return;
  const sq = squareAtPoint(e.clientX, e.clientY);
  if (sq == null) return;
  st.arrowDraft = { from: sq, to: sq, color: annotColor(e), pointerId: e.pointerId };
  try { $('board').setPointerCapture(e.pointerId); } catch { /* ignore */ }
}

function finishAnnot(e, cancelled) {
  const d = st.arrowDraft;
  st.arrowDraft = null;
  try { $('board').releasePointerCapture(d.pointerId); } catch { /* ignore */ }
  if (cancelled) { refreshOverlay(); return; }
  const to = squareAtPoint(e.clientX, e.clientY) ?? d.to;
  if (to === d.from) {
    if (st.premoveQueue.length) { cancelPremoves(); return; } // right-click still cancels queued premoves
    const i = st.annot.squares.findIndex((x) => x.sq === to);
    if (i >= 0 && st.annot.squares[i].color === d.color) st.annot.squares.splice(i, 1);
    else if (i >= 0) st.annot.squares[i].color = d.color;
    else st.annot.squares.push({ sq: to, color: d.color });
  } else {
    const i = st.annot.arrows.findIndex((x) => x.from === d.from && x.to === to);
    if (i >= 0 && st.annot.arrows[i].color === d.color) st.annot.arrows.splice(i, 1);
    else if (i >= 0) st.annot.arrows[i].color = d.color;
    else st.annot.arrows.push({ from: d.from, to, color: d.color });
  }
  redrawBoard();
}

function loadSettings() {
  try {
    Object.assign(st.settings, JSON.parse(localStorage.getItem('cheslan.settings') || '{}'));
    if (!BOARD_THEMES[st.settings.boardTheme]) st.settings.boardTheme = 'classic';
    if (!['drag', 'click', 'both'].includes(st.settings.moveStyle)) st.settings.moveStyle = 'both';
    if (!['none', 'you', 'opponent', 'both'].includes(st.settings.classifications)) st.settings.classifications = 'none';
    if (typeof st.settings.autoQueen !== 'boolean') st.settings.autoQueen = false;
  } catch { /* ignore */ }
}
function saveSettings() { try { localStorage.setItem('cheslan.settings', JSON.stringify(st.settings)); } catch { /* ignore */ } }

function applyPreferences() {
  const theme = BOARD_THEMES[st.settings.boardTheme] || BOARD_THEMES.classic;
  document.documentElement.style.setProperty('--sq-light', theme.light);
  document.documentElement.style.setProperty('--sq-dark', theme.dark);
  $('app').dataset.moveStyle = st.settings.moveStyle;
  saveSettings();
  if (st.settings.classifications !== 'none') void ensureLiveClassifier().catch((e) => console.error('Stockfish initialization failed:', e));
  if (st.session) { renderGame(); void classifyPendingMoves(); }
}

function ensureLiveClassifier() {
  // Stockfish is only loaded when classifications are enabled (default: off)
  if (!st.liveClassifier && st.settings.classifications === 'none') return Promise.resolve();
  if (!st.liveClassifier) {
    st.liveClassifier = createLiveClassifier();
    st.classifierReady = st.liveClassifier.ready.then(() => {
      st.classifierLoaded = true;
      void classifyPendingMoves();
    });
  }
  return st.classifierReady;
}

function setPreference(key, value) {
  st.settings[key] = value;
  applyPreferences();
}

/** -> {initial_ms, bonus_ms, color} as expected by the backend (initial_ms 0 = no timer) */
function getSettings() {
  const { type, initial, bonus, color } = st.settings;
  let i = 0, b = 0;
  if (type === 'custom') { i = initial * 60000; b = bonus * 1000; }
  else if (type !== 'none') { const [m, s] = type.split('+').map(Number); i = m * 60000; b = s * 1000; }
  const pgn = ($('in-pgn') && $('in-pgn').value.trim()) || null;
  return { initial_ms: i, bonus_ms: b, color, pgn };
}

function describeTc(initialMs, bonusMs) {
  if (!initialMs) return 'No timer';
  const m = initialMs / 60000;
  return bonusMs ? `${m} + ${bonusMs / 1000}` : `${m} min`;
}

function renderSettings() {
  const { type, initial, bonus, color } = st.settings;
  $('sel-type').value = type;
  $('custom-box').hidden = type !== 'custom';
  $('rng-init').value = initial; $('lbl-init').textContent = initial;
  $('rng-bonus').value = bonus; $('lbl-bonus').textContent = bonus;
  for (const b of document.querySelectorAll('.color-opt')) b.setAttribute('aria-checked', String(b.dataset.color === color));
  let label;
  if (type === 'none') label = 'No timer';
  else if (type === 'custom') label = `Custom ${describeTc(initial * 60000, bonus * 1000)}`;
  else label = TYPE_LABEL[type] || type;
  const hasPgn = $('in-pgn') && $('in-pgn').value.trim();
  $('settings-summary').textContent = `${label} · ${COLOR_LABEL[color]}${hasPgn ? ' · PGN' : ''}`;
}

function initSettings() {
  loadSettings();
  $('sel-type').addEventListener('change', (e) => { st.settings.type = e.target.value; saveSettings(); renderSettings(); });
  $('rng-init').addEventListener('input', (e) => { st.settings.initial = +e.target.value; saveSettings(); renderSettings(); });
  $('rng-bonus').addEventListener('input', (e) => { st.settings.bonus = +e.target.value; saveSettings(); renderSettings(); });
  for (const b of document.querySelectorAll('.color-opt')) b.addEventListener('click', () => { st.settings.color = b.dataset.color; saveSettings(); renderSettings(); });
  document.addEventListener('pointerdown', (e) => { const d = $('settings'); if (d.open && !d.contains(e.target)) d.open = false; });
  applyPreferences();
  void ensureLiveClassifier().catch((e) => console.error('Stockfish initialization failed:', e));
  renderSettings();
}

function openAppSettings() {
  const build = () => [
    h('header', { class: 'settings-modal-header' },
      h('h2', { text: 'Settings' }),
      btn('Close', () => closeModal('preferences'))),
    h('div', { class: 'field' }, h('span', { text: 'Board theme' }),
      h('div', { class: 'theme-options', role: 'radiogroup', 'aria-label': 'Board theme' },
        Object.entries(BOARD_THEMES).map(([id, theme]) =>
          h('button', {
            class: 'theme-option',
            type: 'button',
            role: 'radio',
            'aria-checked': String(st.settings.boardTheme === id),
            onclick: () => { setPreference('boardTheme', id); showModal('preferences', build); },
          },
          h('span', { class: 'theme-preview' },
            h('i', { style: `background:${theme.light}` }),
            h('i', { style: `background:${theme.dark}` })),
          h('span', { text: theme.name })))),
    ),
    h('div', { class: 'field' }, h('span', { text: 'Moving type' }),
      preferenceOptions('moveStyle', [['drag', 'Drag'], ['click', 'Click'], ['both', 'Both']], build)),
    h('div', { class: 'field' }, h('span', { text: 'Auto promote to queen' }),
      preferenceOptions('autoQueen', [[false, 'Off'], [true, 'On']], build),
      h('small', { class: 'muted', text: 'Skips the promotion popup and always promotes to a queen.' })),
    h('div', { class: 'field' }, h('span', { text: 'Move classifications' }),
      preferenceOptions('classifications', [['none', 'None'], ['you', 'You'], ['opponent', 'Opponent'], ['both', 'Both']], build),
      h('small', { class: 'muted', text: st.settings.classifications === 'none' ? 'Move analysis is off.' : st.classifierLoaded ? 'Stockfish is ready for move analysis.' : 'Stockfish loads in the background before a game starts.' })),
  ];
  showModal('preferences', build, { cardClass: 'preferences-modal', backdropClose: true });
}

function preferenceOptions(key, options, build) {
  return h('div', { class: 'preference-options', role: 'radiogroup' },
    options.map(([value, label]) => h('button', {
      class: 'btn',
      type: 'button',
      role: 'radio',
      'aria-checked': String(st.settings[key] === value),
      onclick: () => { setPreference(key, value); showModal('preferences', build); },
    }, label)));
}

// ---------------- screens ----------------
function showScreen(name) {
  $('screen-lobby').hidden = name !== 'lobby';
  $('screen-game').hidden = name !== 'game';
  $('app').classList.toggle('game-layout', name === 'game');
}

// ---------------- lobby / onboarding ----------------
function fillLobby() {
  if (st.profile) $('in-name').value = st.profile.name;
  setAvatar($('avatar-preview'), { name: $('in-name').value || '?', avatar: st.pending?.dataUrl ?? st.profile?.avatar });
}

function renderPeers() {
  const list = $('peer-list');
  list.replaceChildren();
  if (st.selected && !st.peers.some((p) => p.id === st.selected)) st.selected = null;
  for (const p of st.peers) {
    const av = h('div', { class: 'avatar md' });
    setAvatar(av, p);
    const li = h('li', {
      class: 'peer', role: 'option', tabindex: '0', 'aria-selected': String(p.id === st.selected),
      onclick: () => { st.selected = p.id; renderPeers(); },
      onkeydown: (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); st.selected = p.id; renderPeers(); } },
    }, av, h('span', { class: 'pn', text: p.name }), p.in_game && h('span', { class: 'os ingame', text: 'In game' }), h('span', { class: 'os', text: p.os }));
    list.append(li);
  }
  $('peer-empty').hidden = st.peers.length > 0;
}

async function saveProfile() {
  const prof = await api.saveProfile($('in-name').value, st.pending?.png ?? null, st.pending?.thumb ?? null);
  st.profile = prof;
  st.pending = null;
  $('in-name').value = prof.name;
  fillLobby();
  return prof;
}

async function onConfirm() {
  try {
    await saveProfile();
    if (!st.selected) { toast('Saved. Pick a player from the list to start a game.'); return; }
    $('settings').open = false;
    const target = st.peers.find((p) => p.id === st.selected);
    if (target && target.in_game) { askSpectate(target); return; }
    await ensureLiveClassifier();
    await api.challenge(st.selected, getSettings());
  } catch (e) { toast(e); }
}

function askSpectate(peer) {
  showModal('spectate-ask', () => [
    h('h2', { text: 'In a game' }),
    playerRow(peer),
    h('p', { class: 'sub', text: 'This person is in a game. Want to spectate?' }),
    h('div', { class: 'btn-grid' },
      btn('No', () => closeModal('spectate-ask')),
      btn('Yes', async () => {
        closeModal('spectate-ask');
        try { await ensureLiveClassifier(); await api.spectate(peer.id); } catch (e) { toast(e); }
      }, 'btn primary')),
  ], { backdropClose: true });
}

async function onPickFile(e) {
  const f = e.target.files && e.target.files[0];
  e.target.value = '';
  if (!f) return;
  const res = await openCropper(f);
  if (res) { st.pending = res; fillLobby(); } else toast('Could not use that image.');
}

async function onConnectIp() {
  try {
    await saveProfile();
    await ensureLiveClassifier();
    await api.challengeIp($('in-ip').value, getSettings());
  } catch (e) { toast(e); }
}

// ---------------- dialogs driven by phase ----------------
function playerRow(p) {
  const av = h('div', { class: 'avatar md' });
  setAvatar(av, p);
  return h('div', { class: 'who' }, av, h('div', {}, h('strong', { text: p.name }), h('div', { class: 'muted', text: p.os })));
}

function colorLineForIncoming(pref) {
  if (pref === 'white') return 'You play Black';
  if (pref === 'black') return 'You play White';
  return 'Colours are random';
}

function syncPhaseDialogs(ph) {
  if (ph.phase === 'incoming' && ph.peer) {
    const tcText = ph.settings ? describeTc(ph.settings.initial_ms, ph.settings.bonus_ms) : '';
    const colText = ph.settings ? colorLineForIncoming(ph.settings.color) : '';
    showModal('challenge-in', () => [
      h('h2', { text: 'Challenge!' }),
      playerRow(ph.peer),
      h('p', { class: 'sub', text: 'wants to play a game of chess.' }),
      ph.settings && h('p', { class: 'muted', text: `${tcText} · ${colText}` }),
      h('div', { class: 'btn-grid' },
        btn('Decline', () => api.respondChallenge(false).catch(toast)),
        btn('Accept', async () => { try { await ensureLiveClassifier(); await api.respondChallenge(true); } catch (e) { toast(e); } }, 'btn primary')),
    ]);
  } else closeModal('challenge-in');

  if (ph.phase === 'outgoing' && ph.peer) {
    showModal('challenge-out', () => [
      h('h2', { text: 'Waiting…' }),
      playerRow(ph.peer),
      h('p', { class: 'sub', text: 'Waiting for them to accept your challenge.' }),
      btn('Cancel', () => api.cancelChallenge().catch(toast)),
    ]);
  } else closeModal('challenge-out');
}

// ---------------- refresh / session ----------------
function setSession(next, hint) {
  const prev = st.session;
  st.session = next;
  const newGame = !prev || prev.game_no !== next.game_no;
  if (!next.spectator) {
    if (newGame) st.lowBuzzed = false;
    const inCheckNow = next.check_sq != null && next.turn === next.my_color && !next.end;
    if (inCheckNow && (newGame || !prev || prev.ply !== next.ply)) buzz([90, 60, 90]);
    if (next.end && (newGame || !prev || !prev.end)) buzz([250, 80, 250]);
  }
  if (newGame || !prev || prev.ply !== next.ply) st.annot = emptyAnnot();
  const rewind = st.premoveQueue.length && prev && !newGame
    && (next.end || !next.connected || next.check_sq != null);
  st.rewindRoutes = rewind ? premoveRoutes(prev) : null;
  if (newGame || next.end || !next.connected || next.check_sq != null) st.premoveQueue = [];
  if (newGame) {
    st.moveClasses = [];
    st.classifiedPly = 0;
    st.classifierGame = next.game_no;
    if (st.liveClassifier) st.liveClassifier.reset();
  }
  if (!prev || prev.game_no !== next.game_no) st.viewPly = next.ply;
  else if (st.viewPly === prev.ply) st.viewPly = next.ply;
  st.viewPly = Math.min(st.viewPly, next.ply);
  if (!prev || prev.ply !== next.ply || prev.game_no !== next.game_no) st.sel = null;
  st.checkAnimSq = prev && prev.game_no === next.game_no && next.ply > prev.ply ? next.check_sq : null;
  st.anim = prev && prev.game_no === next.game_no && next.ply === prev.ply + 1 ? { prev, hint: hint || null } : null;
  renderGame();
  st.anim = null;
  st.rewindRoutes = null;
  if (prev && !newGame && prev.turn !== next.my_color && next.turn === next.my_color && !next.end && next.check_sq == null) {
    void playQueuedPremove(next);
  }
  void classifyPendingMoves();
}

async function classifyPendingMoves() {
  if (st.classifying || !st.session || !st.classifierLoaded || st.settings.classifications === 'none') return;
  st.classifying = true;
  let failed = false;
  try {
    while (st.session && st.classifiedPly < st.session.ply) {
      const gameNo = st.session.game_no;
      const plies = await api.getReviewInput();
      if (!st.session || st.session.game_no !== gameNo || st.classifierGame !== gameNo) return;
      const index = st.classifiedPly;
      if (index >= plies.length) return;
      const result = await st.liveClassifier.classify(plies[index]);
      if (!st.session || st.session.game_no !== gameNo || st.classifierGame !== gameNo) return;
      st.moveClasses[index] = result;
      st.classifiedPly = index + 1;
      renderGame();
    }
  } catch (e) {
    failed = true;
    console.error('Move classification failed:', e);
  } finally {
    st.classifying = false;
    if (!failed && st.session && st.classifierLoaded && st.settings.classifications !== 'none' && st.classifiedPly < st.session.ply) void classifyPendingMoves();
  }
}

async function refresh() {
  const ph = await api.getPhase();
  if (ph.phase === 'playing' || ph.phase === 'spectating') {
    const s = await api.getSession();
    showScreen('game');
    if (s) setSession(s);
  } else {
    if (st.session) { st.session = null; st.chat = []; st.unread = 0; st.sel = null; st.drag = null; st.premoveQueue = []; st.endShownFor = 0; st.popupOpen = false; st.viewPly = 0; st.connectionWasLost = false; st.clockSnap = null; updateBadge(); }
    showScreen('lobby');
    $('subtitle').textContent = 'by MalikHw47';
    $('subtitle').removeAttribute('title');
    closeAllExcept(['challenge-in', 'challenge-out', 'crop']);
  }
  syncPhaseDialogs(ph);
}

// ---------------- clocks ----------------
function fmtClock(ms) {
  ms = Math.max(0, ms);
  if (ms < 10000) return `0:${(ms / 1000).toFixed(1).padStart(4, '0')}`;
  const t = Math.floor(ms / 1000), hh = Math.floor(t / 3600), mm = Math.floor((t % 3600) / 60), ss = t % 60;
  return hh ? `${hh}:${String(mm).padStart(2, '0')}:${String(ss).padStart(2, '0')}` : `${mm}:${String(ss).padStart(2, '0')}`;
}

function syncClockSnap(s) {
  st.clockSnap = s.clock ? { t0: performance.now(), white: s.clock.white_ms, black: s.clock.black_ms, running: s.clock.running } : null;
  updateClocks();
}

function updateClocks() {
  const s = st.session, snap = st.clockSnap;
  const bars = [[$('bar-opp'), s && (s.my_color === 'white' ? 'black' : 'white')], [$('bar-me'), s && s.my_color]];
  for (const [bar, color] of bars) {
    const el = bar.querySelector('[data-role=clock]');
    if (!s || !snap) { el.hidden = true; continue; }
    const active = snap.running === color;
    const ms = snap[color] - (active ? performance.now() - snap.t0 : 0);
    el.hidden = false;
    el.textContent = fmtClock(ms);
    el.classList.toggle('active', active);
    el.classList.toggle('low', ms < 10000);
    if (!s.spectator && !s.end && color === s.my_color) {
      if (active && ms > 0 && ms < 10000) { if (!st.lowBuzzed) { st.lowBuzzed = true; buzz([150, 80, 150]); } }
      else if (ms >= 10000) st.lowBuzzed = false;
    }
  }
}

// ---------------- game rendering ----------------
function fillPlayer(bar, player, caps, active) {
  setAvatar(bar.querySelector('[data-role=avatar]'), player);
  bar.querySelector('[data-role=name]').textContent = player.name;
  bar.querySelector('[data-role=os]').textContent = player.os;
  const c = bar.querySelector('[data-role=captured]');
  c.replaceChildren(...caps.map((p) => h('img', { src: pieceSrc(p.color, p.kind), alt: p.kind, draggable: 'false' })));
  bar.classList.toggle('turn', active);
}

function renderBanner(s) {
  const el = $('net-banner');
  if (s.connected) { el.hidden = true; return; }
  el.hidden = false;
  el.classList.toggle('info', s.reconnect.state === 'trying');
  if (s.opp_left) el.textContent = 'Opponent left the game.';
  else if (s.reconnect.state === 'trying') el.textContent = `Connection lost — reconnecting (attempt ${Math.max(1, s.reconnect.attempt)}/${s.reconnect.max})…`;
  else if (s.reconnect.state === 'gave_up') el.textContent = `Couldn't reconnect after ${s.reconnect.max} attempts. You can still review the game or go back to the main screen.`;
  else el.textContent = 'Opponent disconnected.';
}

function renderGame() {
  const s = st.session;
  if (!s) return;
  const view = positionAt(s);
  const sub = $('subtitle');
  if (s.opening) { sub.textContent = s.opening.name; sub.title = s.opening.eco ? `ECO ${s.opening.eco}` : ''; }
  else { sub.textContent = 'by MalikHw47'; sub.removeAttribute('title'); }

  const showTurn = st.viewPly < s.ply || !s.end;
  fillPlayer($('bar-opp'), s.opponent, s.captured_by_opp, showTurn && view.turn !== s.my_color);
  fillPlayer($('bar-me'), s.me, s.captured_by_me, showTurn && view.turn === s.my_color);
  syncClockSnap(s);
  renderBanner(s);
  renderBoard(view);
  st.checkAnimSq = null;

  if (s.end) {
    if (st.endShownFor !== s.game_no) { st.endShownFor = s.game_no; st.popupOpen = true; }
    st.connectionWasLost = false;
  } else if (!s.connected) {
    if (!st.connectionWasLost) st.popupOpen = true;
    st.connectionWasLost = true;
  } else {
    st.connectionWasLost = false;
    st.popupOpen = false;
    closeModal('end');
  }
  const showEnd = s.end || !s.connected;
  $('btn-popup').hidden = !(showEnd && !st.popupOpen);
  if (showEnd && st.popupOpen && !isOpen('review')) showEndModal(); else closeModal('end');
  $('btn-prev').disabled = st.viewPly === 0;
  $('btn-next').disabled = st.viewPly === s.ply;
  $('btn-cancel-premoves').hidden = st.premoveQueue.length === 0;
  const moveLabel = st.viewPly === 0 ? 'Start' : `${Math.floor((st.viewPly - 1) / 2) + 1}${st.viewPly % 2 ? '.' : '...'} ${s.history[st.viewPly - 1]}`;
  $('history-label').textContent = `${moveLabel} (${st.viewPly}/${s.ply})`;

  if (s.draw_in && !s.end) {
    showModal('draw-in', () => [
      h('h2', { text: 'Draw offered' }),
      h('p', { class: 'sub', text: `${s.opponent.name} offers a draw.` }),
      h('div', { class: 'btn-grid' },
        btn('Decline', () => api.respondDraw(false).catch(toast)),
        btn('Accept', () => api.respondDraw(true).catch(toast), 'btn primary')),
    ]);
  } else closeModal('draw-in');

  if (s.rematch_in && s.end) {
    showModal('rematch-in', () => [
      h('h2', { text: 'Rematch?' }),
      h('p', { class: 'sub', text: `${s.opponent.name} wants a rematch (colours swap).` }),
      h('div', { class: 'btn-grid' },
        btn('Decline', () => api.respondRematch(false).catch(toast)),
        btn('Accept', () => api.respondRematch(true).catch(toast), 'btn primary')),
    ]);
  } else closeModal('rematch-in');
}

function positionAt(s) {
  if (st.viewPly === s.ply) return s;
  const [placement, turn] = s.position_history[st.viewPly].split(' ');
  const pieces = [];
  const kinds = { p: 'pawn', n: 'knight', b: 'bishop', r: 'rook', q: 'queen', k: 'king' };
  const ranks = placement.split('/');
  for (let row = 0; row < ranks.length; row++) {
    let file = 0;
    for (const token of ranks[row]) {
      if (/\d/.test(token)) file += Number(token);
      else {
        pieces.push({ sq: (7 - row) * 8 + file, color: token === token.toUpperCase() ? 'white' : 'black', kind: kinds[token.toLowerCase()] });
        file++;
      }
    }
  }
  return {
    ...s,
    pieces,
    turn: turn === 'w' ? 'white' : 'black',
    legal: [],
    check_sq: null,
    last_move: st.viewPly ? s.move_history[st.viewPly - 1] : null,
    end: null,
  };
}

function setViewPly(ply) {
  const s = st.session;
  if (!s) return;
  const prevPly = st.viewPly;
  const prev = positionAt(s);
  st.viewPly = Math.max(0, Math.min(s.ply, ply));
  st.sel = null;
  const view = positionAt(s);
  if (Math.abs(st.viewPly - prevPly) === 1) {
    const [from, to] = s.move_history[Math.min(prevPly, st.viewPly)] || [];
    if (from != null && to != null) {
      st.anim = { prev, move: prevPly < st.viewPly ? [from, to] : [to, from], reverse: prevPly > st.viewPly };
    } else st.anim = null;
  } else st.anim = null;
  st.checkAnimSq = view.check_sq;
  renderGame();
  st.anim = null;
  st.checkAnimSq = null;
}

function pieceAt(s, sq) { return s.pieces.find((p) => p.sq === sq); }
const isFlipped = (s) => s.my_color === 'black'; // opponent plays white => flip pieces AND coordinates
const gridPos = (sq, flip) => ({ col: flip ? 7 - (sq & 7) : sq & 7, row: flip ? sq >> 3 : 7 - (sq >> 3) });

function renderBoard(s) {
  const flip = isFlipped(s);
  const el = $('board');
  el.replaceChildren();
  const premoveView = st.premoveQueue.length && s.connected && !s.end && s.check_sq == null && st.viewPly === s.ply
    ? premovePosition(s)
    : s.pieces;
  const pm = new Map(s.pieces.map((p) => [p.sq, p])); // real pieces never move for premoves
  const ghosts = new Map(premoveView.filter((p) => p.origin !== p.sq).map((p) => [p.sq, p]));
  const targets = new Set();
  if (st.sel != null && !canQueuePremove(s)) { // premove ghosts may go anywhere, so no target dots
    const moves = s.legal.filter((m) => m.from === st.sel);
    for (const m of moves) targets.add(m.to);
  }
  const mated = s.end && s.end.kind === 'checkmate';
  for (let r = 0; r < 8; r++) {
    for (let c = 0; c < 8; c++) {
      const file = flip ? 7 - c : c;
      const rank = flip ? r : 7 - r;
      const sq = rank * 8 + file;
      const cls = ['sq', (file + rank) % 2 === 0 ? 'dark' : 'light'];
      if (s.last_move && (sq === s.last_move[0] || sq === s.last_move[1])) cls.push('last');
      if (sq === st.sel) cls.push('sel');
      if (sq === s.check_sq) cls.push(mated ? 'mate' : 'check'); // mate: solid red, no flicker
      if (st.premoveQueue.some((m) => m.from === sq || m.to === sq)) cls.push('premove');
      if (targets.has(sq)) cls.push(pm.has(sq) ? 'cap' : 'dot');
      const d = h('div', { class: cls.join(' '), role: 'gridcell', 'data-sq': sq });
      const hl = st.annot.squares.find((x) => x.sq === sq);
      if (hl) d.append(h('div', { class: `hl hl-${hl.color}` }));
      if (c === 0) d.append(h('span', { class: 'coord rank', text: String(rank + 1) }));
      if (r === 7) d.append(h('span', { class: 'coord file', text: 'abcdefgh'[file] }));
      const p = pm.get(sq);
      if (p) {
        const pieceClass = ['piece'];
        if (st.drag && st.drag.from === sq && !ghosts.has(sq)) pieceClass.push('lifted');
        if (sq === st.checkAnimSq && p.kind === 'king') pieceClass.push('check-anim');
        const moveClass = st.viewPly > 0 ? st.moveClasses[st.viewPly - 1] : null;
        const selectedSide = st.settings.classifications !== 'none' && (st.settings.classifications === 'both'
          || (st.settings.classifications === 'you' && p.color === s.my_color)
          || (st.settings.classifications === 'opponent' && p.color !== s.my_color));
        const annotatedMove = s.move_history[st.viewPly - 1];
        const meta = moveClass && MOVE_CLASS_META[moveClass];
        const annotated = selectedSide && meta && annotatedMove && annotatedMove[1] === sq;
        const piece = h('img', { class: pieceClass.join(' '), src: pieceSrc(p.color, p.kind), alt: `${p.color} ${p.kind}`, draggable: 'false' });
        d.append(piece);
        if (annotated) d.append(h('img', { class: 'move-class-badge', src: `assets/moves/${moveClass}.svg`, alt: meta.label, title: meta.label, draggable: 'false' }));
      }
      const g = ghosts.get(sq);
      if (g) d.append(h('img', { class: 'piece ghost' + (st.drag && st.drag.from === sq ? ' lifted' : ''), src: pieceSrc(g.color, g.kind), alt: `ghost ${g.color} ${g.kind}`, draggable: 'false' }));
      el.append(d);
    }
  }
  el.append(arrowOverlay(flip, overlayArrows()));
  if (st.anim) runMoveAnimation(el, st.anim.virtual ? { ...s, pieces: premoveView } : s, flip, st.anim);
  if (st.rewindRoutes) {
    for (const route of st.rewindRoutes) slidePiece(el, flip, route.from, route.to, null, 220);
  }
}

// ---------------- smooth piece movement (FLIP) ----------------
function slidePiece(el, flip, toSq, fromSq, fromPoint, ms) {
  const img = el.querySelector(`[data-sq="${toSq}"] .piece`);
  if (!img) return;
  const size = el.clientWidth / 8;
  const t = gridPos(toSq, flip);
  let dx, dy;
  if (fromPoint) {
    const rect = el.getBoundingClientRect();
    dx = fromPoint.x - (rect.left + (t.col + 0.5) * size);
    dy = fromPoint.y - (rect.top + (t.row + 0.5) * size);
  } else {
    const f = gridPos(fromSq, flip);
    dx = (f.col - t.col) * size;
    dy = (f.row - t.row) * size;
  }
  img.classList.add('moving');
  img.style.transition = 'none';
  img.style.transform = `translate(${dx}px, ${dy}px)`;
  void img.offsetWidth; // commit start position
  img.style.transition = `transform ${ms}ms cubic-bezier(.22,.8,.3,1)`;
  img.style.transform = 'translate(0, 0)';
  const done = () => { img.classList.remove('moving'); img.style.transition = ''; img.style.transform = ''; };
  img.addEventListener('transitionend', done, { once: true });
  setTimeout(done, ms + 80);
}

function runMoveAnimation(el, s, flip, { prev, hint, move, reverse }) {
  if (reducedMotion()) return;
  const [from, to] = move || s.last_move || [];
  if (from == null || to == null) return;
  const moved = pieceAt(s, to);
  if (!moved) return;
  if (reverse) {
    const restored = s.pieces.find((p) => p.color !== moved.color && !prev.pieces.some((old) => old.sq === p.sq && old.color === p.color && old.kind === p.kind));
    if (restored) {
      const restoredImg = el.querySelector(`[data-sq="${restored.sq}"] .piece`);
      if (restoredImg) {
        restoredImg.style.transition = 'none';
        restoredImg.style.opacity = '0';
        restoredImg.style.transform = 'scale(.75)';
        void restoredImg.offsetWidth;
        restoredImg.style.transition = 'opacity 220ms ease-out, transform 220ms ease-out';
        requestAnimationFrame(() => {
          restoredImg.style.opacity = '1';
          restoredImg.style.transform = 'scale(1)';
        });
        setTimeout(() => { restoredImg.style.transition = ''; restoredImg.style.opacity = ''; restoredImg.style.transform = ''; }, 300);
      }
    }
  } else {
    // fade out a captured piece (including en passant)
    let capSq = null;
    const before = prev.pieces.find((p) => p.sq === to);
    if (before && before.color !== moved.color) capSq = to;
    else if (!before && moved.kind === 'pawn' && (from & 7) !== (to & 7)) capSq = (from & ~7) | (to & 7);
    const cap = capSq != null ? prev.pieces.find((p) => p.sq === capSq && p.color !== moved.color) : null;
    if (cap) {
      const holder = el.querySelector(`[data-sq="${capSq}"]`);
      if (holder) {
        const ghost = h('img', { class: 'captured-fade', src: pieceSrc(cap.color, cap.kind), alt: '', draggable: 'false' });
        holder.prepend(ghost);
        requestAnimationFrame(() => requestAnimationFrame(() => {
          ghost.style.opacity = '0';
          ghost.style.transform = 'scale(1.25)';
        }));
        setTimeout(() => ghost.remove(), 300);
      }
    }
  }
  const dropped = hint && hint.point;
  slidePiece(el, flip, to, from, dropped ? hint.point : null, dropped ? 130 : 230);
  // castling: slide the rook too
  if (moved.kind === 'king' && Math.abs((from & 7) - (to & 7)) === 2) {
    const rank0 = from & ~7;
    const kingSide = (to & 7) === 6;
    slidePiece(el, flip, rank0 | (kingSide ? 5 : 3), rank0 | (kingSide ? 7 : 0), null, 230);
  }
}

// ---------------- input: click-to-move + drag-to-place ----------------
function canPlay(s) { return s && !s.spectator && !s.end && s.connected && st.viewPly === s.ply && s.turn === s.my_color; }
function canQueuePremove(s) {
  return s && !s.spectator && !s.end && s.connected && st.viewPly === s.ply && s.turn !== s.my_color && s.check_sq == null;
}

function premovePosition(s) {
  const pieces = new Map(s.pieces.map((p) => [p.sq, { ...p, origin: p.sq }]));
  for (const move of st.premoveQueue) {
    const piece = pieces.get(move.from);
    if (!piece || piece.color !== s.my_color) break;
    pieces.delete(move.from);
    pieces.delete(move.to);
    if (piece.kind === 'king' && Math.abs((move.from & 7) - (move.to & 7)) === 2) {
      const rank = move.from & ~7;
      const kingSide = (move.to & 7) === 6;
      const rookFrom = rank | (kingSide ? 7 : 0);
      const rook = pieces.get(rookFrom);
      if (rook) {
        pieces.delete(rookFrom);
        rook.sq = rank | (kingSide ? 5 : 3);
        pieces.set(rook.sq, rook);
      }
    }
    if (move.promo) piece.kind = move.promo;
    piece.sq = move.to;
    pieces.set(move.to, piece);
  }
  return [...pieces.values()];
}

function premoveRoutes() {
  return []; // ghosts: real pieces never move, so nothing needs to slide back
}

/** Ghost premoves may go to ANY square; legality is checked when the move is actually played (illegal => all premoves cancel). */
function premoveTargets(s, from) {
  const piece = premovePosition(s).find((p) => p.sq === from);
  if (!piece || piece.color !== s.my_color) return [];
  const moves = [];
  for (let to = 0; to < 64; to++) if (to !== from) moves.push({ from, to });
  return moves;
}

function cancelPremoves() {
  if (!st.premoveQueue.length && st.sel == null) return;
  st.rewindRoutes = st.session && premoveRoutes(st.session);
  st.premoveQueue = [];
  st.sel = null;
  renderGame();
  st.rewindRoutes = null;
}

function queuePremove(from, to) {
  const s = st.session;
  if (!canQueuePremove(s)) return;
  const before = premovePosition(s);
  const piece = before.find((p) => p.sq === from);
  const promoRank = piece && piece.kind === 'pawn' && (to >> 3) === (piece.color === 'white' ? 7 : 0);
  const move = { from, to, promo: promoRank ? 'queen' : null };
  st.premoveQueue.push(move);
  st.sel = null;
  renderGame();
}

async function playQueuedPremove(s) {
  if (!st.premoveQueue.length) return;
  const queued = st.premoveQueue[0];
  const targets = s.legal.filter((m) => m.from === queued.from && m.to === queued.to && (!queued.promo || m.promo === queued.promo));
  if (!targets.length) {
    cancelPremoves();
    return;
  }
  await playMove(queued.from, queued.to, targets, null, true);
}

function squareAtPoint(x, y) {
  const s = st.session;
  const r = $('board').getBoundingClientRect();
  if (x < r.left || x >= r.right || y < r.top || y >= r.bottom) return null;
  const col = Math.floor(((x - r.left) / r.width) * 8), row = Math.floor(((y - r.top) / r.height) * 8);
  const flip = isFlipped(s);
  const file = flip ? 7 - col : col, rank = flip ? row : 7 - row;
  return rank * 8 + file;
}

function startBoardDrag(e, s, sq, p) {
  const size = $('board').clientWidth / 8;
  const ghost = h('img', { class: 'drag-ghost', src: pieceSrc(p.color, p.kind), alt: '', draggable: 'false', style: `width:${size}px;height:${size}px;left:${e.clientX}px;top:${e.clientY}px` });
  document.body.append(ghost);
  try { $('board').setPointerCapture(e.pointerId); } catch { /* ignore */ }
  st.drag = { from: sq, ghost, moved: false, wasSel: st.sel === sq, start: { x: e.clientX, y: e.clientY }, last: { x: e.clientX, y: e.clientY } };
  st.sel = sq;
  renderBoard(s);
}

function onBoardDown(e) {
  if (e.pointerType === 'mouse' && e.button === 2) { e.preventDefault(); startAnnot(e); return; }
  if (e.pointerType === 'mouse' && e.button !== 0) return;
  if (st.annot.arrows.length || st.annot.squares.length) { // left click clears arrows / highlights
    st.annot = emptyAnnot();
    queueMicrotask(redrawBoard);
  }
  const s = st.session;
  if ((!canPlay(s) && !canQueuePremove(s)) || st.drag) return;
  const cell = e.target.closest('.sq');
  if (!cell) return;
  const sq = Number(cell.dataset.sq);
  const clickMoves = st.settings.moveStyle !== 'drag';
  const dragMoves = st.settings.moveStyle !== 'click';
  if (canQueuePremove(s)) {
    const projected = premovePosition(s);
    const ownHere = projected.find((p) => p.sq === sq && p.color === s.my_color);
    if (clickMoves && st.sel != null && sq !== st.sel && !ownHere) {
      e.preventDefault();
      queuePremove(st.sel, sq);
      return;
    }
    const piece = projected.find((p) => p.sq === sq);
    if (piece && piece.color === s.my_color) {
      if (clickMoves) {
        st.sel = sq;
        if (!dragMoves) { renderBoard(s); return; }
      }
      if (dragMoves) { e.preventDefault(); startBoardDrag(e, s, sq, piece); return; }
    }
    if (clickMoves) st.sel = piece && piece.color === s.my_color ? sq : null;
    else st.sel = null;
    renderBoard(s);
    return;
  }
  // second tap on a highlighted target => move (click-to-move)
  if (clickMoves && st.sel != null) {
    const t = s.legal.filter((m) => m.from === st.sel && m.to === sq);
    if (t.length) { e.preventDefault(); playMove(st.sel, sq, t, null); return; }
  }
  const p = pieceAt(s, sq);
  if (p && p.color === s.my_color && s.legal.some((m) => m.from === sq)) {
    if (dragMoves) { e.preventDefault(); startBoardDrag(e, s, sq, p); }
    else { st.sel = sq; renderBoard(s); }
  } else {
    st.sel = null;
    renderBoard(s);
  }
}

function onBoardMove(e) {
  if (st.arrowDraft) {
    const sq = squareAtPoint(e.clientX, e.clientY);
    if (sq != null && sq !== st.arrowDraft.to) { st.arrowDraft.to = sq; refreshOverlay(); }
    return;
  }
  const d = st.drag;
  if (!d) return;
  d.last = { x: e.clientX, y: e.clientY };
  if (!d.moved && Math.hypot(e.clientX - d.start.x, e.clientY - d.start.y) > 5) d.moved = true;
  d.ghost.style.left = e.clientX + 'px';
  d.ghost.style.top = e.clientY + 'px';
}

function onBoardUp(e) {
  if (st.arrowDraft) { finishAnnot(e, e.type === 'pointercancel'); return; }
  const d = st.drag;
  if (!d) return;
  st.drag = null;
  d.ghost.remove();
  try { $('board').releasePointerCapture(e.pointerId); } catch { /* ignore */ }
  const s = st.session;
  if (!s) return;
  if (d.moved) {
    const to = squareAtPoint(e.clientX, e.clientY);
    if (to != null && to !== d.from && canQueuePremove(s)) {
      if (premoveTargets(s, d.from).some((m) => m.to === to)) { queuePremove(d.from, to); return; }
    } else {
      const t = to != null && to !== d.from ? s.legal.filter((m) => m.from === d.from && m.to === to) : [];
      if (t.length) { playMove(d.from, to, t, { point: { x: e.clientX, y: e.clientY } }); return; }
    }
    if (st.settings.moveStyle === 'drag') st.sel = null;
  } else if (d.wasSel) {
    st.sel = null; // tapped an already-selected piece: deselect
  } else if (st.settings.moveStyle === 'drag') {
    st.sel = null;
  }
  renderBoard(s);
}

async function playMove(from, to, targets, hint, premove = false) {
  const s = st.session;
  let promo = '';
  if (targets.some((m) => m.promo)) {
    if (premove || st.settings.autoQueen) promo = 'q';
    else {
      const choice = await askPromotion(s.my_color);
      if (!choice) { st.sel = null; renderBoard(s); return; }
      promo = PROMO_CHAR[choice];
      if (hint) hint = null; // piece snaps from its original square behind the dialog
    }
  }
  try {
    st.sel = null;
    const next = await api.makeMove(sqName(from) + sqName(to) + promo);
    if (premove) st.premoveQueue.shift();
    setSession(next, hint);
  } catch (e) {
    if (premove) cancelPremoves();
    toast(e);
    await refresh();
  }
}

function askPromotion(color) {
  return new Promise((resolve) => {
    let done = false;
    const pick = (k) => { if (done) return; done = true; closeModal('promo'); resolve(k); };
    showModal('promo', () => [
      h('h2', { text: 'Promote to…' }),
      h('div', { class: 'promo-row' }, ['queen', 'rook', 'bishop', 'knight'].map((k) =>
        h('button', { type: 'button', 'aria-label': k, onclick: () => pick(k) }, h('img', { src: pieceSrc(color, k), alt: k })))),
      btn('Cancel', () => pick(null)),
    ]);
  });
}

// ---------------- end-of-game modal ----------------
function endText(s) {
  const e = s.end;
  if (!e && !s.connected) return ['Connection Lost', 'Connection to the other player was lost.'];
  if (s.spectator) {
    const nameOf = (c) => (c === 'white' ? s.me.name : s.opponent.name);
    const winner = e.winner ? nameOf(e.winner) : '';
    const loser = e.winner ? nameOf(e.winner === 'white' ? 'black' : 'white') : '';
    switch (e.kind) {
      case 'checkmate': return [`${winner} wins`, 'Checkmate.'];
      case 'resigned': return [`${winner} wins`, `${loser} resigned.`];
      case 'timeout': return [`${winner} wins`, `${loser} ran out of time.`];
      case 'stalemate': return ['Draw', 'Stalemate.'];
      case 'draw_agreed': return ['Draw', 'Draw by agreement.'];
      case 'fifty_move': return ['Draw', 'Fifty-move rule.'];
      case 'threefold': return ['Draw', 'Threefold repetition.'];
      default: return ['Draw', 'Insufficient material.'];
    }
  }
  const won = e.winner === s.my_color;
  switch (e.kind) {
    case 'checkmate': return won ? ['GGs checkmate', 'You delivered checkmate.'] : ['GGs you lost', 'Checkmate.'];
    case 'stalemate': return ['GGs stalemate', 'No legal moves, but not in check.'];
    case 'resigned': return won ? ['GGs you won', `${s.opponent.name} resigned.`] : ['GGs you lost', 'You resigned.'];
    case 'timeout': return won ? ['GGs you won', `${s.opponent.name} ran out of time.`] : ['GGs you lost', 'You ran out of time.'];
    case 'draw_agreed': return ['GGs draw', 'Draw by agreement.'];
    case 'fifty_move': return ['GGs draw', 'Fifty-move rule.'];
    case 'threefold': return ['GGs draw', 'Threefold repetition.'];
    default: return ['GGs draw', 'Insufficient material.'];
  }
}

function showEndModal() {
  showModal('end', () => {
    const s = st.session;
    if (!s || (!s.end && s.connected)) return [];
    const [title, sub] = endText(s);
    const rm = s.spectator ? null : s.rematch_out ? btn('Waiting…', null, 'btn', { disabled: true })
      : btn(s.connected ? 'Rematch' : 'Opponent offline', () => api.requestRematch().then(() => toast('Rematch request sent.')).catch(toast), 'btn primary', { disabled: !s.connected });
    return [
      h('h2', { text: title }),
      h('p', { class: 'sub', text: sub }),
      h('div', { class: 'btn-grid' },
        rm,
        btn('Main screen', () => api.leaveGame().catch(toast)),
        btn('Game review', openReview),
        btn('Keep board', () => { st.popupOpen = false; closeModal('end'); $('btn-popup').hidden = false; })),
    ];
  });
}

// ---------------- game review ----------------
const ROW_LABELS = { brilliant: 'Brilliant', best: 'Best', excellent: 'Excellent', good: 'Good', book: 'Book', inaccuracy: 'Inaccuracy', miss: 'Miss', mistake: 'Mistake', blunder: 'Blunder' };
const ROW_ORDER = ['blunder', 'miss', 'mistake', 'inaccuracy', 'good', 'excellent', 'best', 'brilliant', 'book'];

async function openReview() {
  const s = st.session;
  closeModal('end');
  st.popupOpen = false;
  $('btn-popup').hidden = false;
  const token = { cancelled: false };
  const bar = h('div', {});
  const label = h('p', { class: 'sub', text: 'Starting Stockfish 19 Lite…' });
  const body = h('div', {}, label, h('div', { class: 'progress' }, bar));
  const close = () => { token.cancelled = true; closeModal('review'); };
  showModal('review', () => [h('h2', { text: 'Game review' }), body, btn('Close', close)], { onClose: () => { token.cancelled = true; } });
  try {
    const plies = await api.getReviewInput();
    if (!plies.length) { label.textContent = 'No moves to review.'; return; }
    const res = await runReview(plies, (d, n) => { bar.style.width = `${Math.round((d / n) * 100)}%`; label.textContent = `Analysing position ${d} of ${n}…`; }, token);
    if (!res) return;
    const mine = res[s.my_color], theirs = res[s.my_color === 'white' ? 'black' : 'white'];
    // Opponent on the left, local player ALWAYS on the right.
    const table = h('table', { class: 'review-table' },
      h('thead', {}, h('tr', {}, h('th', { text: '' }), h('th', { text: s.opponent.name }), h('th', { class: 'me', text: s.spectator ? s.me.name : `${s.me.name} (you)` }))),
      h('tbody', {},
        h('tr', {}, h('td', { text: 'Accuracy' }), h('td', { class: 'num', text: theirs.accuracy == null ? '–' : `${theirs.accuracy}%` }), h('td', { class: 'num', text: mine.accuracy == null ? '–' : `${mine.accuracy}%` })),
        ROW_ORDER.filter((k) => CLASSES.includes(k)).map((k) =>
          h('tr', {},
            h('td', {}, h('span', { class: 'dot-label' }, h('i', { class: 'c-' + k }), ROW_LABELS[k])),
            h('td', { class: 'num', text: String(theirs.counts[k]) }),
            h('td', { class: 'num', text: String(mine.counts[k]) })))));
    body.replaceChildren(h('div', { style: 'overflow:auto' }, table));
  } catch (e) {
    label.textContent = 'Review failed: ' + (e && e.message ? e.message : e);
  }
}

// ---------------- menu / chat ----------------
function openMenu() {
  const s = st.session;
  if (!s) return;
  if (s.spectator) {
    showModal('menu', () => [
      h('h2', { text: 'Spectating' }),
      h('div', { class: 'btn-col' },
        btn('Exit to main menu', () => { closeModal('menu'); api.leaveGame().catch(toast); }, 'btn danger'),
        btn('Copy PGN', async () => { closeModal('menu'); await copyPgn(); }),
        btn('Close', () => closeModal('menu'))),
    ], { backdropClose: true });
    return;
  }
  const live = !s.end && s.connected && st.viewPly === s.ply;
  const abandon = s.ply <= 1;
  showModal('menu', () => [
    h('h2', { text: 'Game' }),
    h('div', { class: 'btn-col' },
      btn(s.draw_out ? 'Draw offered…' : 'Offer draw', async () => { closeModal('menu'); try { await api.offerDraw(); toast('Draw offered.'); } catch (e) { toast(e); } }, 'btn', { disabled: !live || s.draw_out }),
      btn(abandon ? 'Abandon Game' : 'Resign', () => { closeModal('menu'); confirmResign(abandon); }, 'btn danger', { disabled: !live }),
      btn('Copy PGN', async () => { closeModal('menu'); await copyPgn(); }),
      btn('Close', () => closeModal('menu'))),
  ], { backdropClose: true });
}

function confirmResign(abandon = false) {
  showModal('resign', () => [
    h('h2', { text: abandon ? 'Abandon game?' : 'Resign?' }),
    h('p', { class: 'sub', text: abandon ? 'You will leave this game.' : 'You will lose this game.' }),
    h('div', { class: 'btn-grid' }, btn('Keep playing', () => closeModal('resign')),
      btn(abandon ? 'Abandon Game' : 'Resign', async () => { closeModal('resign'); try { await api.resign(); } catch (e) { toast(e); } }, 'btn danger')),
  ]);
}

async function copyPgn() {
  try {
    const pgn = await api.getPgn();
    try { await navigator.clipboard.writeText(pgn); }
    catch {
      const ta = h('textarea', { style: 'position:fixed;opacity:0' }); ta.value = pgn; document.body.append(ta); ta.select();
      const ok = document.execCommand('copy'); ta.remove();
      if (!ok) throw new Error('clipboard unavailable');
    }
    toast('PGN copied.');
  } catch (e) { toast('Could not copy PGN: ' + (e.message || e)); }
}

// 1–4 emoji and nothing else (whitespace ignored): shown without a bubble, 20% larger.
const EMOJI_UNIT = '(?:\\p{Regional_Indicator}{2}|[#*0-9]\\uFE0F?\\u20E3|\\p{Extended_Pictographic}(?:\\uFE0F|\\p{Emoji_Modifier})?(?:\\u200D\\p{Extended_Pictographic}(?:\\uFE0F|\\p{Emoji_Modifier})?)*)';
const EMOJI_ONLY = new RegExp(`^${EMOJI_UNIT}{1,4}$`, 'u');
export const isEmojiOnly = (t) => EMOJI_ONLY.test(t.replace(/\s+/gu, ''));

function addBubble(list, text, mine, from) {
  const cls = 'bubble' + (mine ? ' me' : '') + (isEmojiOnly(text) ? ' emoji' : '');
  const el = h('div', { class: cls }); // text nodes only, never HTML
  if (from && !mine) el.append(h('span', { class: 'bubble-from', text: from }));
  el.append(document.createTextNode(text));
  list.append(el);
  list.scrollTop = list.scrollHeight;
}

let chatList = null;
function openChat() {
  st.unread = 0;
  updateBadge();
  const input = h('input', { type: 'text', maxlength: MAX_CHAT, placeholder: 'Message…', autocomplete: 'off', 'aria-label': 'Message' });
  const list = h('div', { class: 'chat-list' });
  chatList = list;
  for (const m of st.chat) addBubble(list, m.text, m.mine, m.from);
  const send = async () => {
    const text = input.value.slice(0, MAX_CHAT);
    if (!text.trim()) return;
    try { const clean = await api.sendChat(text); st.chat.push({ text: clean, mine: true }); addBubble(list, clean, true); input.value = ''; }
    catch (e) { toast(e); }
  };
  input.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); send(); } });
  showModal('chat', () => [
    h('header', {}, h('strong', { text: 'Chat' }), btn('Close', () => closeModal('chat'))),
    list,
    h('div', { class: 'chat-input' }, input, btn('Send', send, 'btn primary')),
  ], { cardClass: 'chat', onClose: () => { chatList = null; } });
  setTimeout(() => { list.scrollTop = list.scrollHeight; input.focus(); }, 0);
}

function updateBadge() {
  const b = $('chat-badge');
  b.hidden = st.unread === 0;
  b.textContent = String(Math.min(st.unread, 99));
}

// ---------------- init ----------------
async function init() {
  initSettings();
  $('btn-pick').addEventListener('click', () => $('in-file').click());
  $('in-file').addEventListener('change', onPickFile);
  $('btn-confirm').addEventListener('click', onConfirm);
  $('in-pgn').addEventListener('input', renderSettings);
  $('btn-ip').addEventListener('click', onConnectIp);
  $('in-name').addEventListener('input', () => {
    if (!st.pending && !st.profile?.avatar) setAvatar($('avatar-preview'), { name: $('in-name').value || '?', avatar: null });
  });
  $('btn-prev').addEventListener('click', () => setViewPly(st.viewPly - 1));
  $('btn-next').addEventListener('click', () => setViewPly(st.viewPly + 1));
  $('btn-app-settings').addEventListener('click', openAppSettings);
  $('btn-gear').addEventListener('click', openMenu);
  $('btn-chat').addEventListener('click', openChat);
  $('btn-cancel-premoves').addEventListener('click', cancelPremoves);
  $('btn-popup').addEventListener('click', () => { st.popupOpen = true; showEndModal(); $('btn-popup').hidden = true; });

  const board = $('board');
  board.addEventListener('pointerdown', onBoardDown);
  board.addEventListener('pointermove', onBoardMove);
  board.addEventListener('pointerup', onBoardUp);
  board.addEventListener('pointercancel', onBoardUp);
  board.addEventListener('contextmenu', (e) => e.preventDefault());

  setInterval(updateClocks, 100);

  await listen('peers-changed', (p) => { st.peers = p; renderPeers(); });
  await listen('state-changed', () => refresh().catch(console.error));
  await listen('challenge-declined', () => toast('Challenge declined or player unavailable.'));
  await listen('challenge-cancelled', () => toast('Challenge cancelled.'));
  await listen('draw-declined', () => toast('Draw offer declined.'));
  await listen('rematch-declined', () => toast('Rematch declined.'));
  await listen('peer-disconnected', () => toast('Connection lost — trying to reconnect…'));
  await listen('peer-reconnected', () => toast('Reconnected!'));
  await listen('reconnect-failed', () => toast('Could not reconnect.'));
  await listen('net-error', (m) => toast(m));
  await listen('spectator-joined', (name) => toast(`Spectator ${name} came in!`));
  await listen('spectate-ended', () => toast('The game you were watching has ended.'));
  await listen('chat', (p) => {
    const text = typeof p === 'string' ? p : p.text;
    const from = typeof p === 'string' ? null : p.from;
    st.chat.push({ text, mine: false, from });
    if (chatList) addBubble(chatList, text, false, from);
    else { st.unread++; updateBadge(); toast('New chat message'); }
  });

  st.profile = await api.getProfile();
  fillLobby();
  st.peers = await api.listPeers();
  renderPeers();
  await refresh();
}

init().catch((e) => { console.error(e); toast('Startup error: ' + (e.message || e)); });
