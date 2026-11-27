//! Helpers for game review: static material "sacrifice" detection used for Brilliant moves.
use crate::{position::Position, types::*};

fn material(pos: &Position, c: Color) -> i32 {
    pos.board.iter().flatten().filter(|p| p.color == c).map(|p| p.kind.value()).sum()
}

fn eval_stm(pos: &Position) -> i32 {
    material(pos, pos.side) - material(pos, pos.side.opp())
}

fn qsearch(pos: &Position, mut alpha: i32, beta: i32, depth: u8) -> i32 {
    let stand = eval_stm(pos);
    if stand >= beta {
        return beta;
    }
    if depth == 0 {
        return stand;
    }
    if stand > alpha {
        alpha = stand;
    }
    for m in pos.legal_moves() {
        if !pos.is_capture(m) {
            continue;
        }
        let mut c = pos.clone();
        c.make(m);
        let score = -qsearch(&c, -beta, -alpha, depth - 1);
        if score >= beta {
            return beta;
        }
        if score > alpha {
            alpha = score;
        }
    }
    alpha
}

/// Net material change (centipawns, mover's view) after the move and the forced capture
/// exchanges that follow. Strongly negative (<= -200) means the move gives up material.
pub fn sacrifice_cp(fen: &str, uci: &str) -> i32 {
    let (Ok(pos), Some(m)) = (Position::from_fen(fen), Move::parse_uci(uci)) else { return 0 };
    let before = eval_stm(&pos);
    let mut c = pos.clone();
    c.make(m);
    let q = qsearch(&c, -100_000, 100_000, 8);
    -q - before
}
