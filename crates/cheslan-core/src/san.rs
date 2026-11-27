use crate::position::Position;
use crate::types::*;

/// SAN without check/mate suffix.
pub fn san_core(pos: &Position, m: Move, legal: &[Move]) -> String {
    let p = pos.board[m.from as usize].expect("no piece");
    if p.kind == Kind::King && (file(m.from) as i8 - file(m.to) as i8).abs() == 2 {
        return if file(m.to) == 6 { "O-O".into() } else { "O-O-O".into() };
    }
    let mut s = String::new();
    let capture = pos.is_capture(m);
    if p.kind == Kind::Pawn {
        if capture {
            s.push((b'a' + file(m.from)) as char);
        }
    } else {
        s.push(p.kind.letter());
        let others: Vec<&Move> = legal
            .iter()
            .filter(|o| o.to == m.to && o.from != m.from && pos.board[o.from as usize].map(|q| q.kind) == Some(p.kind))
            .collect();
        if !others.is_empty() {
            let same_file = others.iter().any(|o| file(o.from) == file(m.from));
            let same_rank = others.iter().any(|o| rank(o.from) == rank(m.from));
            if !same_file {
                s.push((b'a' + file(m.from)) as char);
            } else if !same_rank {
                s.push((b'1' + rank(m.from)) as char);
            } else {
                s.push_str(&sq_name(m.from));
            }
        }
    }
    if capture {
        s.push('x');
    }
    s.push_str(&sq_name(m.to));
    if let Some(k) = m.promo {
        s.push('=');
        s.push(k.letter());
    }
    s
}

pub fn to_san(pos: &Position, m: Move, legal: &[Move]) -> String {
    let mut s = san_core(pos, m, legal);
    let mut c = pos.clone();
    c.make(m);
    if c.in_check() {
        s.push(if c.legal_moves().is_empty() { '#' } else { '+' });
    }
    s
}

/// Parse a SAN token (suffixes `+#!?` ignored, `0-0` accepted) into a legal move.
pub fn from_san(pos: &Position, san: &str) -> Option<Move> {
    let want: String = san.chars().filter(|c| !"+#!?".contains(*c)).collect::<String>().replace('0', "O");
    let legal = pos.legal_moves();
    legal.iter().copied().find(|&m| san_core(pos, m, &legal) == want)
}
