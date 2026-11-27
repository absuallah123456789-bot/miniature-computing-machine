// Game review: runs Stockfish 19 Lite (WASM, single-threaded) in a Web Worker and classifies moves.
const ENGINE_URL = 'vendor/stockfish/stockfish-19-lite-single.js';
const DEPTH = 12;
const MOVETIME_MS = 2500; // safety cap per position (slow phones)

class Engine {
  constructor(url) {
    this.w = new Worker(url);
    this.subs = new Set();
    this.error = null;
    this.w.onmessage = (e) => { const line = typeof e.data === 'string' ? e.data : String(e.data); for (const f of [...this.subs]) f(line); };
    this.w.onerror = (e) => {
      this.error = new Error(e.message || 'Stockfish worker failed to load.');
      for (const f of [...this.subs]) f('');
    };
  }
  send(c) { this.w.postMessage(c); }
  waitFor(pred) {
    return new Promise((res, rej) => {
      if (this.error) { rej(this.error); return; }
      const f = (l) => {
        if (this.error) { this.subs.delete(f); rej(this.error); }
        else if (pred(l)) { this.subs.delete(f); res(l); }
      };
      this.subs.add(f);
    });
  }
  async init() {
    this.send('uci'); await this.waitFor((l) => l === 'uciok');
    this.send('isready'); await this.waitFor((l) => l === 'readyok');
  }
  async analyse(fen, depth = DEPTH, movetimeMs = MOVETIME_MS) {
    let last = null;
    const track = (l) => { if (l.startsWith('info') && l.includes(' score ') && !l.includes('lowerbound') && !l.includes('upperbound')) last = l; };
    this.subs.add(track);
    this.send('position fen ' + fen);
    this.send(`go depth ${depth} movetime ${movetimeMs}`);
    const bm = await this.waitFor((l) => l.startsWith('bestmove'));
    this.subs.delete(track);
    const best = (bm.split(/\s+/)[1] || '').trim();
    let cp = 0;
    if (last) {
      const m = last.match(/score (cp|mate) (-?\d+)/);
      if (m) cp = m[1] === 'cp' ? parseInt(m[2], 10) : (parseInt(m[2], 10) > 0 ? 10000 : -10000);
    }
    const stmWhite = fen.split(' ')[1] === 'w';
    return { whiteCp: stmWhite ? cp : -cp, best };
  }
  terminate() { this.w.terminate(); }
}

const winPct = (cp) => 50 + 50 * (2 / (1 + Math.exp(-0.00368208 * Math.max(-10000, Math.min(10000, cp)))) - 1);
const moveAccuracy = (loss) => Math.max(0, Math.min(100, 103.1668 * Math.exp(-0.04354 * loss) - 3.1669));

export const CLASSES = ['brilliant', 'best', 'excellent', 'good', 'book', 'inaccuracy', 'miss', 'mistake', 'blunder'];

// loss = win-probability points lost by the mover (0..100).
function classify({ loss, isBest, wpBefore, wpAfter, sacCp, book, prevBad }) {
  if (sacCp <= -200 && loss < 2 && wpAfter >= 45 && wpAfter < 95 && wpBefore < 92) return 'brilliant';
  if (prevBad && wpBefore >= 55 && loss >= 10) return 'miss';       // failed to punish the opponent's error
  if (loss >= 20) return 'blunder';
  if (loss >= 10) return 'mistake';
  if (loss >= 5) return 'inaccuracy';
  if (book) return 'book';
  if (isBest || loss <= 0.5) return 'best';
  if (loss < 2) return 'excellent';
  return 'good';
}

export function createLiveClassifier() {
  const engine = new Engine(ENGINE_URL);
  const ready = engine.init();
  let previousFen = null;
  let previousEval = null;
  let prevBad = false;
  return {
    ready,
    reset() { previousFen = null; previousEval = null; prevBad = false; },
    async classify(ply) {
      await ready;
      const before = previousFen === ply.fen_before
        ? previousEval
        : await engine.analyse(ply.fen_before, 10, 450);
      const after = await engine.analyse(ply.fen_after, 10, 450);
      const sign = ply.mover === 'white' ? 1 : -1;
      const cpBefore = before.whiteCp * sign;
      const cpAfter = ply.after_state === 'mate' ? 10000
        : ply.after_state === 'draw' ? 0
          : after.whiteCp * sign;
      const wpBefore = winPct(cpBefore), wpAfter = winPct(cpAfter);
      const loss = Math.max(0, wpBefore - wpAfter);
      const result = classify({
        loss,
        isBest: ply.uci === before.best,
        wpBefore,
        wpAfter,
        sacCp: ply.sac_cp,
        book: ply.book,
        prevBad,
      });
      prevBad = result === 'mistake' || result === 'blunder';
      previousFen = ply.fen_after;
      previousEval = after;
      return result;
    },
    terminate() { engine.terminate(); },
  };
}

/**
 * @param plies  output of get_review_input
 * @param onProgress (done,total)
 * @param token  {cancelled:boolean}
 * @returns {white:{counts,accuracy}, black:{counts,accuracy}} or null if cancelled
 */
export async function runReview(plies, onProgress, token) {
  const eng = new Engine(ENGINE_URL);
  try {
    await eng.init();
    const n = plies.length;
    const evals = [];
    for (let i = 0; i < n; i++) {
      if (token.cancelled) return null;
      evals.push(await eng.analyse(plies[i].fen_before));
      onProgress(i + 1, n);
    }
    const res = {};
    for (const c of ['white', 'black']) res[c] = { counts: Object.fromEntries(CLASSES.map((k) => [k, 0])), accSum: 0, moves: 0 };
    let prevBad = false;
    for (let i = 0; i < n; i++) {
      const p = plies[i];
      const sign = p.mover === 'white' ? 1 : -1;
      const cpBefore = evals[i].whiteCp * sign;
      let cpAfter;
      if (p.after_state === 'mate') cpAfter = 10000;
      else if (p.after_state === 'draw') cpAfter = 0;
      else if (i + 1 < n) cpAfter = evals[i + 1].whiteCp * sign;
      else cpAfter = (await eng.analyse(p.fen_after)).whiteCp * sign;
      const wpBefore = winPct(cpBefore), wpAfter = winPct(cpAfter);
      const loss = Math.max(0, wpBefore - wpAfter);
      const cls = classify({ loss, isBest: p.uci === evals[i].best, wpBefore, wpAfter, sacCp: p.sac_cp, book: p.book, prevBad });
      prevBad = cls === 'mistake' || cls === 'blunder';
      const r = res[p.mover];
      r.counts[cls]++; r.accSum += moveAccuracy(loss); r.moves++;
    }
    for (const c of ['white', 'black']) {
      res[c].accuracy = res[c].moves ? Math.round((res[c].accSum / res[c].moves) * 10) / 10 : null;
      delete res[c].accSum;
    }
    return res;
  } finally {
    eng.terminate();
  }
}
