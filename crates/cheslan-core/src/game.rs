use crate::position::Position;
use crate::san;
use crate::types::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Ongoing,
    Checkmate { winner: Color },
    Stalemate,
    DrawFiftyMove,
    DrawThreefold,
    DrawInsufficient,
}

#[derive(Clone, Debug)]
pub struct MoveRecord {
    pub mv: Move,
    pub san: String,
    pub mover: Color,
    pub captured: Option<Piece>,
    pub fen_before: String,
}

#[derive(Clone, Debug)]
pub struct Game {
    pub start: Position,
    pub pos: Position,
    pub moves: Vec<MoveRecord>,
    keys: Vec<String>,
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

impl Game {
    pub fn new() -> Game {
        Game::from_position(Position::startpos())
    }

    pub fn from_position(p: Position) -> Game {
        let k = p.key();
        Game { start: p.clone(), pos: p, moves: vec![], keys: vec![k] }
    }

    pub fn from_fen(fen: &str) -> Result<Game, String> {
        Ok(Game::from_position(Position::from_fen(fen)?))
    }

    /// Position keys for every ply (index 0 = starting position).
    pub fn keys(&self) -> &[String] {
        &self.keys
    }

    pub fn play(&mut self, m: Move) -> Result<(), String> {
        let legal = self.pos.legal_moves();
        if !legal.contains(&m) {
            return Err(format!("illegal move {}", m.uci()));
        }
        let san = san::to_san(&self.pos, m, &legal);
        let fen_before = self.pos.to_fen();
        let mover = self.pos.side;
        let captured = self.pos.make(m);
        self.keys.push(self.pos.key());
        self.moves.push(MoveRecord { mv: m, san, mover, captured, fen_before });
        Ok(())
    }

    pub fn play_uci(&mut self, uci: &str) -> Result<(), String> {
        let m = Move::parse_uci(uci).ok_or_else(|| "bad move syntax".to_string())?;
        self.play(m)
    }

    pub fn outcome(&self) -> Outcome {
        if self.pos.legal_moves().is_empty() {
            return if self.pos.in_check() {
                Outcome::Checkmate { winner: self.pos.side.opp() }
            } else {
                Outcome::Stalemate
            };
        }
        if self.pos.insufficient_material() {
            return Outcome::DrawInsufficient;
        }
        if self.pos.halfmove >= 100 {
            return Outcome::DrawFiftyMove;
        }
        let last = self.keys.last().unwrap();
        if self.keys.iter().filter(|k| *k == last).count() >= 3 {
            return Outcome::DrawThreefold;
        }
        Outcome::Ongoing
    }

    pub fn pgn(&self, white: &str, black: &str, result: &str) -> String {
        let (y, m, d) = ymd_today();
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "'");
        let mut out = String::new();
        out.push_str("[Event \"ChesLan LAN game\"]\n[Site \"LAN\"]\n");
        out.push_str(&format!("[Date \"{y:04}.{m:02}.{d:02}\"]\n[Round \"-\"]\n"));
        out.push_str(&format!("[White \"{}\"]\n[Black \"{}\"]\n[Result \"{}\"]\n", esc(white), esc(black), result));
        if self.start != Position::startpos() {
            out.push_str(&format!("[SetUp \"1\"]\n[FEN \"{}\"]\n", self.start.to_fen()));
        }
        out.push('\n');
        let mut num = self.start.fullmove;
        for (i, r) in self.moves.iter().enumerate() {
            if r.mover == Color::White {
                out.push_str(&format!("{}. {} ", num, r.san));
            } else {
                if i == 0 {
                    out.push_str(&format!("{}... ", num));
                }
                out.push_str(&format!("{} ", r.san));
                num += 1;
            }
        }
        out.push_str(result);
        out.push('\n');
        out
    }
}

fn ymd_today() -> (i64, u32, u32) {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let z = secs.div_euclid(86400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}
