//! ECO opening lookup. Data: lichess-org/chess-openings (CC0), embedded at compile time.
//! Each TSV row is `eco \t name \t pgn`; lines are replayed with our own SAN parser and
//! indexed by position key, so transpositions are recognised.
use crate::{position::Position, san};
use std::{collections::HashMap, sync::OnceLock};

const RAW: &str = include_str!("../resources/eco.tsv");

struct Db {
    map: HashMap<String, (&'static str, &'static str)>,
}

static DB: OnceLock<Db> = OnceLock::new();

fn db() -> &'static Db {
    DB.get_or_init(|| {
        let mut map = HashMap::with_capacity(4096);
        for line in RAW.lines() {
            let mut it = line.split('\t');
            let (Some(eco), Some(name), Some(pgn)) = (it.next(), it.next(), it.next()) else { continue };
            let mut pos = Position::startpos();
            let mut ok = true;
            for tok in pgn.split_whitespace() {
                if tok.ends_with('.') {
                    continue;
                }
                match san::from_san(&pos, tok) {
                    Some(m) => {
                        pos.make(m);
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                map.entry(pos.key()).or_insert((eco, name));
            }
        }
        Db { map }
    })
}

/// Latest known opening for a game, given the position keys of every ply (index 0 = start).
pub fn lookup(keys: &[String]) -> Option<(&'static str, &'static str)> {
    let d = db();
    keys.iter().rev().find_map(|k| d.map.get(k.as_str()).copied())
}

pub fn contains(key: &str) -> bool {
    db().map.contains_key(key)
}

pub fn entries() -> usize {
    db().map.len()
}
