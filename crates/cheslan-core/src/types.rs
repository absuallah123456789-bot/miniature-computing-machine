use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Color {
    White,
    Black,
}

impl Color {
    pub fn opp(self) -> Color {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Color::White => "white",
            Color::Black => "black",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Pawn => "pawn",
            Kind::Knight => "knight",
            Kind::Bishop => "bishop",
            Kind::Rook => "rook",
            Kind::Queen => "queen",
            Kind::King => "king",
        }
    }
    pub fn letter(self) -> char {
        match self {
            Kind::Pawn => 'P',
            Kind::Knight => 'N',
            Kind::Bishop => 'B',
            Kind::Rook => 'R',
            Kind::Queen => 'Q',
            Kind::King => 'K',
        }
    }
    pub fn from_char(c: char) -> Option<Kind> {
        match c.to_ascii_lowercase() {
            'p' => Some(Kind::Pawn),
            'n' => Some(Kind::Knight),
            'b' => Some(Kind::Bishop),
            'r' => Some(Kind::Rook),
            'q' => Some(Kind::Queen),
            'k' => Some(Kind::King),
            _ => None,
        }
    }
    /// Material value in centipawns (king = 0).
    pub fn value(self) -> i32 {
        match self {
            Kind::Pawn => 100,
            Kind::Knight => 300,
            Kind::Bishop => 300,
            Kind::Rook => 500,
            Kind::Queen => 900,
            Kind::King => 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Piece {
    pub color: Color,
    pub kind: Kind,
}

impl Piece {
    pub fn fen_char(self) -> char {
        let c = self.kind.letter();
        if self.color == Color::White {
            c
        } else {
            c.to_ascii_lowercase()
        }
    }
}

/// Square index: a1 = 0, h1 = 7, a8 = 56, h8 = 63.
pub type Sq = u8;

pub fn file(sq: Sq) -> u8 {
    sq & 7
}
pub fn rank(sq: Sq) -> u8 {
    sq >> 3
}
pub fn sq_name(sq: Sq) -> String {
    format!("{}{}", (b'a' + file(sq)) as char, rank(sq) + 1)
}
pub fn parse_sq(s: &str) -> Option<Sq> {
    let b = s.as_bytes();
    if b.len() != 2 || !(b'a'..=b'h').contains(&b[0]) || !(b'1'..=b'8').contains(&b[1]) {
        return None;
    }
    Some((b[1] - b'1') * 8 + (b[0] - b'a'))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Move {
    pub from: Sq,
    pub to: Sq,
    pub promo: Option<Kind>,
}

impl Move {
    pub fn uci(&self) -> String {
        let mut s = format!("{}{}", sq_name(self.from), sq_name(self.to));
        if let Some(k) = self.promo {
            s.push(k.letter().to_ascii_lowercase());
        }
        s
    }
    pub fn parse_uci(s: &str) -> Option<Move> {
        if !s.is_ascii() || (s.len() != 4 && s.len() != 5) {
            return None;
        }
        let from = parse_sq(&s[0..2])?;
        let to = parse_sq(&s[2..4])?;
        let promo = if s.len() == 5 {
            match Kind::from_char(s.as_bytes()[4] as char)? {
                k @ (Kind::Queen | Kind::Rook | Kind::Bishop | Kind::Knight) => Some(k),
                _ => return None,
            }
        } else {
            None
        };
        Some(Move { from, to, promo })
    }
}
