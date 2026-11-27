use crate::eco;
use crate::game::{Game, Outcome};
use crate::position::Position;
use crate::review::sacrifice_cp;
use crate::types::*;

fn perft(pos: &Position, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let moves = pos.legal_moves();
    if depth == 1 {
        return moves.len() as u64;
    }
    moves
        .iter()
        .map(|&m| {
            let mut c = pos.clone();
            c.make(m);
            perft(&c, depth - 1)
        })
        .sum()
}

fn check_perft(fen: &str, expected: &[u64]) {
    let pos = Position::from_fen(fen).unwrap();
    for (i, &e) in expected.iter().enumerate() {
        assert_eq!(perft(&pos, i as u32 + 1), e, "perft depth {} for {}", i + 1, fen);
    }
}

fn play(g: &mut Game, ucis: &[&str]) {
    for u in ucis {
        g.play_uci(u).unwrap_or_else(|e| panic!("{u}: {e}"));
    }
}

fn has(fen: &str, uci: &str) -> bool {
    let p = Position::from_fen(fen).unwrap();
    let m = Move::parse_uci(uci).unwrap();
    p.legal_moves().contains(&m)
}

#[test]
fn perft_start() {
    check_perft("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", &[20, 400, 8902, 197_281]);
}
#[test]
fn perft_kiwipete() {
    check_perft("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1", &[48, 2039, 97_862]);
}
#[test]
fn perft_position3() {
    check_perft("8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1", &[14, 191, 2812, 43_238]);
}
#[test]
fn perft_position4() {
    check_perft("r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1", &[6, 264, 9467]);
}
#[test]
fn perft_position5() {
    check_perft("rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8", &[44, 1486, 62_379]);
}

#[test]
fn fen_roundtrip() {
    for f in [
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 3 17",
        "7k/8/8/KPp4r/8/8/8/8 w - c6 0 1",
    ] {
        assert_eq!(Position::from_fen(f).unwrap().to_fen(), f);
    }
}

#[test]
fn castling_both_sides_and_execution() {
    let f = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
    assert!(has(f, "e1g1") && has(f, "e1c1"));
    let mut p = Position::from_fen(f).unwrap();
    p.make(Move::parse_uci("e1g1").unwrap());
    assert_eq!(p.board[6].map(|x| x.kind), Some(Kind::King));
    assert_eq!(p.board[5].map(|x| x.kind), Some(Kind::Rook));
    assert!(p.board[7].is_none() && !p.castling[0] && !p.castling[1]);
}

#[test]
fn castling_through_check_and_out_of_check_illegal() {
    // Black rook on f8 attacks f1: king-side illegal, queen-side fine.
    let f = "5r1k/8/8/8/8/8/8/R3K2R w KQ - 0 1";
    assert!(!has(f, "e1g1"));
    assert!(has(f, "e1c1"));
    // In check: neither side.
    let f = "4r2k/8/8/8/8/8/8/R3K2R w KQ - 0 1";
    assert!(!has(f, "e1g1") && !has(f, "e1c1"));
    // Attacked b1 does not prevent queen-side castling (only the king's path matters).
    assert!(has("1r5k/8/8/8/8/8/8/R3K3 w Q - 0 1", "e1c1"));
}

#[test]
fn castling_rights_tracking() {
    let mut p = Position::from_fen("r3k2r/8/8/8/8/8/6b1/R3K2R b KQkq - 0 1").unwrap();
    p.make(Move::parse_uci("g2h1").unwrap()); // bishop captures the h1 rook
    assert!(!p.castling[0] && p.castling[1]);
    let mut p = Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
    p.make(Move::parse_uci("a1a2").unwrap());
    assert!(p.castling[0] && !p.castling[1]);
    let mut p = Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
    p.make(Move::parse_uci("e1e2").unwrap());
    assert!(!p.castling[0] && !p.castling[1] && p.castling[2] && p.castling[3]);
}

#[test]
fn en_passant() {
    let mut g = Game::new();
    play(&mut g, &["e2e4", "a7a6", "e4e5", "d7d5"]);
    assert!(g.pos.legal_moves().contains(&Move::parse_uci("e5d6").unwrap()));
    play(&mut g, &["e5d6"]);
    assert!(g.pos.board[35].is_none(), "captured pawn on d5 must be removed");
    assert_eq!(g.moves.last().unwrap().captured.map(|p| p.kind), Some(Kind::Pawn));
    assert_eq!(g.moves.last().unwrap().san, "exd6");
}

#[test]
fn en_passant_illegal_when_pinned_horizontally() {
    assert!(!has("7k/8/8/KPp4r/8/8/8/8 w - c6 0 1", "b5c6"));
}

#[test]
fn en_passant_only_immediately() {
    let mut g = Game::new();
    play(&mut g, &["e2e4", "a7a6", "e4e5", "d7d5", "h2h3", "h7h6"]);
    assert!(!g.pos.legal_moves().contains(&Move::parse_uci("e5d6").unwrap()));
}

#[test]
fn promotion_generates_four_choices() {
    let p = Position::from_fen("8/P6k/8/8/8/8/8/K7 w - - 0 1").unwrap();
    let promos: Vec<_> = p.legal_moves().into_iter().filter(|m| m.from == 48).collect();
    assert_eq!(promos.len(), 4);
    assert!(promos.iter().all(|m| m.promo.is_some()));
    let mut g = Game::from_fen("8/P6k/8/8/8/8/8/K7 w - - 0 1").unwrap();
    g.play_uci("a7a8n").unwrap();
    assert_eq!(g.pos.board[56].map(|p| p.kind), Some(Kind::Knight));
    assert_eq!(g.moves[0].san, "a8=N");
    assert!(g.play_uci("a7a8").is_err());
}

#[test]
fn fools_mate_is_checkmate() {
    let mut g = Game::new();
    play(&mut g, &["f2f3", "e7e5", "g2g4", "d8h4"]);
    assert_eq!(g.outcome(), Outcome::Checkmate { winner: Color::Black });
    assert_eq!(g.moves.last().unwrap().san, "Qh4#");
    assert!(g.play_uci("a2a3").is_err());
}

#[test]
fn stalemate() {
    let g = Game::from_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").unwrap();
    assert_eq!(g.outcome(), Outcome::Stalemate);
}

#[test]
fn insufficient_material_cases() {
    let ins = |f: &str| Position::from_fen(f).unwrap().insufficient_material();
    assert!(ins("4k3/8/8/8/8/8/8/4K3 w - - 0 1"));
    assert!(ins("4k3/8/8/8/8/8/8/2B1K3 w - - 0 1"));
    assert!(ins("4k3/8/8/8/8/8/8/1N2K3 w - - 0 1"));
    assert!(ins("4kb2/8/8/8/8/8/8/2B1K3 w - - 0 1")); // same-coloured bishops
    assert!(!ins("4k3/8/8/8/8/8/8/2B1KB2 w - - 0 1")); // opposite colours
    assert!(!ins("4k3/8/8/8/8/8/8/1N2KN2 w - - 0 1"));
    assert!(!ins("4k3/8/8/8/8/8/P7/4K3 w - - 0 1"));
    assert_eq!(Game::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap().outcome(), Outcome::DrawInsufficient);
}

#[test]
fn fifty_move_rule() {
    let g = Game::from_fen("4k3/8/8/8/8/8/8/R3K3 w - - 100 80").unwrap();
    assert_eq!(g.outcome(), Outcome::DrawFiftyMove);
    let g = Game::from_fen("4k3/8/8/8/8/8/8/R3K3 w - - 99 80").unwrap();
    assert_eq!(g.outcome(), Outcome::Ongoing);
}

#[test]
fn threefold_repetition() {
    let mut g = Game::new();
    let shuffle = ["g1f3", "g8f6", "f3g1", "f6g8"];
    play(&mut g, &shuffle);
    assert_eq!(g.outcome(), Outcome::Ongoing);
    play(&mut g, &shuffle);
    assert_eq!(g.outcome(), Outcome::DrawThreefold);
}

#[test]
fn repetition_ignores_unusable_ep() {
    // After 1.Nf3 Nf6 2.Ng1 Ng8 the start position repeats even though no ep is involved.
    let mut g = Game::new();
    play(&mut g, &["e2e4", "g8f6", "g1f3", "f6g8", "f3g1", "g8f6"]);
    assert_eq!(g.keys().len(), 7);
}

#[test]
fn san_disambiguation_and_suffixes() {
    let mut g = Game::from_fen("4k3/8/8/8/8/8/8/1N2KN2 w - - 0 1").unwrap();
    g.play_uci("b1d2").unwrap();
    assert_eq!(g.moves[0].san, "Nbd2");
    let mut g = Game::new();
    play(&mut g, &["e2e4", "d7d5", "e4d5", "d8d5", "b1c3"]);
    let sans: Vec<_> = g.moves.iter().map(|m| m.san.as_str()).collect();
    assert_eq!(sans, ["e4", "d5", "exd5", "Qxd5", "Nc3"]);
    let mut g = Game::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
    g.play_uci("e1g1").unwrap();
    assert_eq!(g.moves[0].san, "O-O");
}

#[test]
fn san_roundtrip() {
    let g = Game::new();
    let m = crate::san::from_san(&g.pos, "Nf3").unwrap();
    assert_eq!(m.uci(), "g1f3");
    assert!(crate::san::from_san(&g.pos, "Nf6").is_none());
}

#[test]
fn pgn_output() {
    let mut g = Game::new();
    play(&mut g, &["e2e4", "e7e5", "g1f3"]);
    let pgn = g.pgn("Ann", "Bob", "*");
    assert!(pgn.contains("[White \"Ann\"]") && pgn.contains("[Black \"Bob\"]"));
    assert!(pgn.contains("1. e4 e5 2. Nf3 *"));
}

#[test]
fn eco_lookup() {
    assert!(eco::entries() > 3000);
    let mut g = Game::new();
    play(&mut g, &["e2e4", "e7e5", "g1f3", "b8c6", "f1b5"]);
    let (code, name) = eco::lookup(g.keys()).unwrap();
    assert!(name.starts_with("Ruy Lopez"), "{code} {name}");
    // Transposition: 1.Nf3 e5 2.e4 Nc6 3.Bb5 reaches the same position.
    let mut g2 = Game::new();
    play(&mut g2, &["g1f3", "e7e5", "e2e4", "b8c6", "f1b5"]);
    assert!(eco::lookup(g2.keys()).unwrap().1.starts_with("Ruy Lopez"));
}

#[test]
fn sacrifice_detection() {
    // Queen takes a defended pawn: gives up material.
    assert!(sacrifice_cp("4k3/8/2p5/1p6/8/8/8/1Q2K3 w - - 0 1", "b1b5") <= -200);
    // Plain developing move: no change.
    assert_eq!(sacrifice_cp("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", "g1f3"), 0);
    // Hanging a knight to a pawn.
    assert!(sacrifice_cp("4k3/8/8/3p4/8/2N5/8/4K3 w - - 0 1", "c3e4") <= -200);
}
