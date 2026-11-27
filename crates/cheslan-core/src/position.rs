use crate::types::*;

const KNIGHT: [(i8, i8); 8] = [(1, 2), (2, 1), (2, -1), (1, -2), (-1, -2), (-2, -1), (-2, 1), (-1, 2)];
const KING: [(i8, i8); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];
const BISHOP_DIRS: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];
const ROOK_DIRS: [(i8, i8); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
const QUEEN_DIRS: [(i8, i8); 8] = [(1, 1), (1, -1), (-1, 1), (-1, -1), (1, 0), (-1, 0), (0, 1), (0, -1)];

fn offs(sq: Sq, df: i8, dr: i8) -> Option<Sq> {
    let f = (sq & 7) as i8 + df;
    let r = (sq >> 3) as i8 + dr;
    if (0..8).contains(&f) && (0..8).contains(&r) {
        Some((r * 8 + f) as u8)
    } else {
        None
    }
}

fn push_pawn(v: &mut Vec<Move>, from: Sq, to: Sq, promo_rank: u8) {
    if rank(to) == promo_rank {
        for k in [Kind::Queen, Kind::Rook, Kind::Bishop, Kind::Knight] {
            v.push(Move { from, to, promo: Some(k) });
        }
    } else {
        v.push(Move { from, to, promo: None });
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Position {
    pub board: [Option<Piece>; 64],
    pub side: Color,
    /// Castling rights: [white king-side, white queen-side, black king-side, black queen-side]
    pub castling: [bool; 4],
    pub ep: Option<Sq>,
    pub halfmove: u32,
    pub fullmove: u32,
}

impl Position {
    pub fn startpos() -> Position {
        Position::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap()
    }

    pub fn from_fen(fen: &str) -> Result<Position, String> {
        let mut it = fen.split_whitespace();
        let placement = it.next().ok_or("empty FEN")?;
        let side = it.next().unwrap_or("w");
        let castling = it.next().unwrap_or("-");
        let ep = it.next().unwrap_or("-");
        let half = it.next().unwrap_or("0");
        let full = it.next().unwrap_or("1");
        let mut board = [None; 64];
        let ranks: Vec<&str> = placement.split('/').collect();
        if ranks.len() != 8 {
            return Err("FEN needs 8 ranks".into());
        }
        for (i, r) in ranks.iter().enumerate() {
            let rk = 7 - i as u8;
            let mut f = 0u8;
            for ch in r.chars() {
                if let Some(d) = ch.to_digit(10) {
                    f += d as u8;
                } else {
                    let kind = Kind::from_char(ch).ok_or("bad piece in FEN")?;
                    let color = if ch.is_ascii_uppercase() { Color::White } else { Color::Black };
                    if f > 7 {
                        return Err("rank too long".into());
                    }
                    board[(rk * 8 + f) as usize] = Some(Piece { color, kind });
                    f += 1;
                }
            }
            if f != 8 {
                return Err("rank length != 8".into());
            }
        }
        let side = match side {
            "w" => Color::White,
            "b" => Color::Black,
            _ => return Err("bad side".into()),
        };
        let mut c = [false; 4];
        for ch in castling.chars() {
            match ch {
                'K' => c[0] = true,
                'Q' => c[1] = true,
                'k' => c[2] = true,
                'q' => c[3] = true,
                _ => {}
            }
        }
        let ep = if ep == "-" { None } else { parse_sq(ep) };
        Ok(Position {
            board,
            side,
            castling: c,
            ep,
            halfmove: half.parse().unwrap_or(0),
            fullmove: full.parse().unwrap_or(1),
        })
    }

    pub fn to_fen(&self) -> String {
        let mut s = String::new();
        for r in (0..8u8).rev() {
            let mut empty = 0;
            for f in 0..8u8 {
                match self.board[(r * 8 + f) as usize] {
                    None => empty += 1,
                    Some(p) => {
                        if empty > 0 {
                            s.push_str(&empty.to_string());
                            empty = 0;
                        }
                        s.push(p.fen_char());
                    }
                }
            }
            if empty > 0 {
                s.push_str(&empty.to_string());
            }
            if r > 0 {
                s.push('/');
            }
        }
        s.push(' ');
        s.push(if self.side == Color::White { 'w' } else { 'b' });
        s.push(' ');
        let mut c = String::new();
        for (i, ch) in ['K', 'Q', 'k', 'q'].iter().enumerate() {
            if self.castling[i] {
                c.push(*ch);
            }
        }
        if c.is_empty() {
            c.push('-');
        }
        s.push_str(&c);
        s.push(' ');
        match self.ep {
            Some(e) => s.push_str(&sq_name(e)),
            None => s.push('-'),
        }
        s.push_str(&format!(" {} {}", self.halfmove, self.fullmove));
        s
    }

    pub fn king_sq(&self, color: Color) -> Option<Sq> {
        (0..64u8).find(|&s| self.board[s as usize] == Some(Piece { color, kind: Kind::King }))
    }

    /// Is `sq` attacked by any piece of colour `by`?
    pub fn attacked(&self, sq: Sq, by: Color) -> bool {
        let pdr: i8 = if by == Color::White { -1 } else { 1 };
        for df in [-1i8, 1] {
            if let Some(s) = offs(sq, df, pdr) {
                if self.board[s as usize] == Some(Piece { color: by, kind: Kind::Pawn }) {
                    return true;
                }
            }
        }
        for (df, dr) in KNIGHT {
            if let Some(s) = offs(sq, df, dr) {
                if self.board[s as usize] == Some(Piece { color: by, kind: Kind::Knight }) {
                    return true;
                }
            }
        }
        for (df, dr) in KING {
            if let Some(s) = offs(sq, df, dr) {
                if self.board[s as usize] == Some(Piece { color: by, kind: Kind::King }) {
                    return true;
                }
            }
        }
        for &(df, dr) in &BISHOP_DIRS {
            let mut cur = sq;
            while let Some(s) = offs(cur, df, dr) {
                match self.board[s as usize] {
                    None => cur = s,
                    Some(p) => {
                        if p.color == by && (p.kind == Kind::Bishop || p.kind == Kind::Queen) {
                            return true;
                        }
                        break;
                    }
                }
            }
        }
        for &(df, dr) in &ROOK_DIRS {
            let mut cur = sq;
            while let Some(s) = offs(cur, df, dr) {
                match self.board[s as usize] {
                    None => cur = s,
                    Some(p) => {
                        if p.color == by && (p.kind == Kind::Rook || p.kind == Kind::Queen) {
                            return true;
                        }
                        break;
                    }
                }
            }
        }
        false
    }

    fn king_attacked(&self, color: Color) -> bool {
        match self.king_sq(color) {
            Some(k) => self.attacked(k, color.opp()),
            None => false,
        }
    }

    pub fn in_check(&self) -> bool {
        self.king_attacked(self.side)
    }

    fn pawn_moves(&self, sq: Sq, us: Color, v: &mut Vec<Move>) {
        let dir: i8 = if us == Color::White { 1 } else { -1 };
        let start_rank = if us == Color::White { 1 } else { 6 };
        let promo_rank = if us == Color::White { 7 } else { 0 };
        if let Some(t) = offs(sq, 0, dir) {
            if self.board[t as usize].is_none() {
                push_pawn(v, sq, t, promo_rank);
                if rank(sq) == start_rank {
                    if let Some(t2) = offs(sq, 0, 2 * dir) {
                        if self.board[t2 as usize].is_none() {
                            v.push(Move { from: sq, to: t2, promo: None });
                        }
                    }
                }
            }
        }
        for df in [-1i8, 1] {
            if let Some(t) = offs(sq, df, dir) {
                match self.board[t as usize] {
                    Some(q) if q.color != us => push_pawn(v, sq, t, promo_rank),
                    None if Some(t) == self.ep => v.push(Move { from: sq, to: t, promo: None }),
                    _ => {}
                }
            }
        }
    }

    pub fn pseudo_moves(&self) -> Vec<Move> {
        let mut v = Vec::with_capacity(64);
        let us = self.side;
        for sq in 0..64u8 {
            let Some(p) = self.board[sq as usize] else { continue };
            if p.color != us {
                continue;
            }
            match p.kind {
                Kind::Pawn => self.pawn_moves(sq, us, &mut v),
                Kind::Knight => {
                    for (df, dr) in KNIGHT {
                        if let Some(t) = offs(sq, df, dr) {
                            match self.board[t as usize] {
                                Some(q) if q.color == us => {}
                                _ => v.push(Move { from: sq, to: t, promo: None }),
                            }
                        }
                    }
                }
                Kind::King => {
                    for (df, dr) in KING {
                        if let Some(t) = offs(sq, df, dr) {
                            match self.board[t as usize] {
                                Some(q) if q.color == us => {}
                                _ => v.push(Move { from: sq, to: t, promo: None }),
                            }
                        }
                    }
                    self.castle_moves(sq, us, &mut v);
                }
                Kind::Bishop | Kind::Rook | Kind::Queen => {
                    let dirs: &[(i8, i8)] = match p.kind {
                        Kind::Bishop => &BISHOP_DIRS,
                        Kind::Rook => &ROOK_DIRS,
                        _ => &QUEEN_DIRS,
                    };
                    for &(df, dr) in dirs {
                        let mut cur = sq;
                        while let Some(t) = offs(cur, df, dr) {
                            match self.board[t as usize] {
                                None => {
                                    v.push(Move { from: sq, to: t, promo: None });
                                    cur = t;
                                }
                                Some(q) => {
                                    if q.color != us {
                                        v.push(Move { from: sq, to: t, promo: None });
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
        v
    }

    /// Castling: rights, empty squares, rook present, king not in check and
    /// not passing through an attacked square. (Landing square is verified by
    /// the generic king-safety filter in `legal_moves`.)
    fn castle_moves(&self, sq: Sq, us: Color, v: &mut Vec<Move>) {
        let (r0, ki, qi, them) = if us == Color::White { (0u8, 0usize, 1usize, Color::Black) } else { (7u8, 2, 3, Color::White) };
        let ksq = r0 * 8 + 4;
        if sq != ksq || self.attacked(ksq, them) {
            return;
        }
        let at = |f: u8| self.board[(r0 * 8 + f) as usize];
        let rook = Some(Piece { color: us, kind: Kind::Rook });
        if self.castling[ki] && at(5).is_none() && at(6).is_none() && at(7) == rook
            && !self.attacked(r0 * 8 + 5, them) && !self.attacked(r0 * 8 + 6, them)
        {
            v.push(Move { from: ksq, to: r0 * 8 + 6, promo: None });
        }
        if self.castling[qi] && at(1).is_none() && at(2).is_none() && at(3).is_none() && at(0) == rook
            && !self.attacked(r0 * 8 + 3, them) && !self.attacked(r0 * 8 + 2, them)
        {
            v.push(Move { from: ksq, to: r0 * 8 + 2, promo: None });
        }
    }

    pub fn legal_moves(&self) -> Vec<Move> {
        let us = self.side;
        self.pseudo_moves()
            .into_iter()
            .filter(|&m| {
                let mut c = self.clone();
                c.make(m);
                !c.king_attacked(us)
            })
            .collect()
    }

    pub fn is_capture(&self, m: Move) -> bool {
        self.board[m.to as usize].is_some()
            || (matches!(self.board[m.from as usize], Some(Piece { kind: Kind::Pawn, .. })) && file(m.from) != file(m.to))
    }

    /// Apply a (legal) move. Returns the captured piece, if any.
    pub fn make(&mut self, m: Move) -> Option<Piece> {
        let us = self.side;
        let p = self.board[m.from as usize].expect("no piece on from-square");
        let mut captured = self.board[m.to as usize];
        if p.kind == Kind::Pawn && Some(m.to) == self.ep && file(m.from) != file(m.to) && captured.is_none() {
            let cap_sq = (rank(m.from) * 8 + file(m.to)) as usize;
            captured = self.board[cap_sq];
            self.board[cap_sq] = None;
        }
        self.board[m.from as usize] = None;
        let placed = match m.promo {
            Some(k) => Piece { color: us, kind: k },
            None => p,
        };
        self.board[m.to as usize] = Some(placed);
        if p.kind == Kind::King && (file(m.from) as i8 - file(m.to) as i8).abs() == 2 {
            let r = rank(m.from);
            let (rf, rt) = if file(m.to) == 6 { (r * 8 + 7, r * 8 + 5) } else { (r * 8, r * 8 + 3) };
            self.board[rt as usize] = self.board[rf as usize].take();
        }
        if p.kind == Kind::King {
            if us == Color::White {
                self.castling[0] = false;
                self.castling[1] = false;
            } else {
                self.castling[2] = false;
                self.castling[3] = false;
            }
        }
        for (idx, s) in [(0usize, 7u8), (1, 0), (2, 63), (3, 56)] {
            if m.from == s || m.to == s {
                self.castling[idx] = false;
            }
        }
        self.ep = None;
        if p.kind == Kind::Pawn && (rank(m.from) as i8 - rank(m.to) as i8).abs() == 2 {
            self.ep = Some((m.from + m.to) / 2);
        }
        if p.kind == Kind::Pawn || captured.is_some() {
            self.halfmove = 0;
        } else {
            self.halfmove += 1;
        }
        if us == Color::Black {
            self.fullmove += 1;
        }
        self.side = us.opp();
        captured
    }

    /// En-passant square only if an ep capture is actually legal (FIDE repetition rule).
    fn effective_ep(&self) -> Option<Sq> {
        let ep = self.ep?;
        let ok = self
            .legal_moves()
            .iter()
            .any(|m| m.to == ep && matches!(self.board[m.from as usize], Some(Piece { kind: Kind::Pawn, .. })));
        if ok { Some(ep) } else { None }
    }

    /// Repetition / opening-book key: placement, side, castling, effective ep.
    pub fn key(&self) -> String {
        let fen = self.to_fen();
        let parts: Vec<&str> = fen.split(' ').collect();
        let ep = match self.effective_ep() {
            Some(e) => sq_name(e),
            None => "-".to_string(),
        };
        format!("{} {} {} {}", parts[0], parts[1], parts[2], ep)
    }

    /// Dead position by material: K v K, K+minor v K, or only same-coloured bishops.
    pub fn insufficient_material(&self) -> bool {
        let (mut knights, mut light, mut dark) = (0, 0, 0);
        for sq in 0..64u8 {
            if let Some(p) = self.board[sq as usize] {
                match p.kind {
                    Kind::Pawn | Kind::Rook | Kind::Queen => return false,
                    Kind::Knight => knights += 1,
                    Kind::Bishop => {
                        if (file(sq) + rank(sq)) % 2 == 0 { dark += 1 } else { light += 1 }
                    }
                    Kind::King => {}
                }
            }
        }
        let minors = knights + light + dark;
        minors <= 1 || (knights == 0 && (light == 0 || dark == 0))
    }
}
