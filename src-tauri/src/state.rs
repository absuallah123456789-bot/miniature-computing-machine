use crate::protocol::*;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use cheslan_core::{eco, game::*, types::*};
use rand::{distributions::Alphanumeric, Rng};
use serde::Serialize;
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU16, AtomicU64},
        Mutex,
    },
    time::Instant,
};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc::UnboundedSender;

// ---------- sanitising helpers ----------

pub fn clean_text(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
}
pub fn clean_name(s: &str) -> String {
    clean_text(s, MAX_NAME_CHARS)
}
pub fn peer_name(s: &str) -> String {
    let n = clean_name(s);
    if n.is_empty() { "player".into() } else { n }
}
pub fn clean_os(s: &str) -> String {
    match s.to_ascii_lowercase().as_str() {
        "windows" => "Windows",
        "linux" => "Linux",
        "android" => "Android",
        "macos" => "macOS",
        "ios" => "iOS",
        _ => "Other",
    }
    .to_string()
}
pub fn my_os() -> String {
    clean_os(std::env::consts::OS)
}
pub fn random_alnum(n: usize) -> String {
    rand::thread_rng().sample_iter(&Alphanumeric).take(n).map(|b| (b as char).to_ascii_lowercase()).collect()
}

/// Validate a base64 JPEG thumbnail from the network and turn it into a data URL.
pub fn thumb_data_url(b64: Option<&str>) -> Option<String> {
    let bytes = B64.decode(b64?).ok()?;
    if bytes.len() > MAX_THUMB_BYTES || bytes.len() < 4 || bytes[..3] != [0xFF, 0xD8, 0xFF] {
        return None;
    }
    Some(format!("data:image/jpeg;base64,{}", B64.encode(bytes)))
}

// ---------- model ----------

#[derive(Clone)]
pub struct Profile {
    pub name: String,
    pub avatar_data_url: Option<String>,
    pub thumb_b64: Option<String>,
    pub avatar_hash: String,
}

pub fn hash_bytes(b: &[u8]) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    b.hash(&mut h);
    format!("{:016x}", h.finish())
}

pub fn load_profile(dir: &Path) -> Option<Profile> {
    let raw = std::fs::read_to_string(dir.join("profile.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let name = clean_name(v.get("name")?.as_str()?);
    let png = std::fs::read(dir.join("avatar.png")).ok();
    let thumb = std::fs::read(dir.join("avatar_thumb.jpg")).ok();
    Some(Profile {
        name,
        avatar_data_url: png.map(|b| format!("data:image/png;base64,{}", B64.encode(b))),
        avatar_hash: thumb.as_deref().map(hash_bytes).unwrap_or_default(),
        thumb_b64: thumb.map(|b| B64.encode(b)),
    })
}

#[derive(Clone, Serialize)]
pub struct PlayerDto {
    pub id: String,
    pub name: String,
    pub os: String,
    pub avatar: Option<String>,
}

pub struct Peer {
    pub info: PlayerDto,
    pub ip: IpAddr,
    pub port: u16,
    pub avatar_hash: String,
    pub fetched_hash: String,
    pub fetching: bool,
    pub last_seen: Instant,
}

#[derive(Clone)]
pub enum Phase {
    Idle,
    Outgoing(PlayerDto),
    Incoming(PlayerDto),
    Playing,
}

impl Phase {
    pub fn tag(&self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Outgoing(_) => "outgoing",
            Phase::Incoming(_) => "incoming",
            Phase::Playing => "playing",
        }
    }
}

pub struct Link {
    pub id: u64,
    pub tx: UnboundedSender<Msg>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EndKind {
    Checkmate,
    Stalemate,
    FiftyMove,
    Threefold,
    Insufficient,
    Resigned,
    DrawAgreed,
    Timeout,
}

#[derive(Clone, Copy, Debug)]
pub struct GameEnd {
    pub kind: EndKind,
    pub winner: Option<Color>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reconnect {
    None,
    Trying(u8),
    GaveUp,
}

pub struct Clock {
    pub white_ms: i64,
    pub black_ms: i64,
    /// Set while the side-to-move's clock is running.
    pub last: Option<Instant>,
}

pub struct Session {
    pub sid: String,
    pub dialer: bool,
    pub tc: TimeControl,
    pub clock: Option<Clock>,
    pub opp_left: bool,
    pub reconnect: Reconnect,
    pub me: PlayerDto,
    pub opp: PlayerDto,
    pub my_color: Color,
    pub game: Game,
    pub forced: Option<GameEnd>,
    pub connected: bool,
    pub draw_in: bool,
    pub draw_out: bool,
    pub rematch_in: bool,
    pub rematch_out: bool,
    pub game_no: u32,
}

pub struct Inner {
    pub profile: Option<Profile>,
    pub peers: HashMap<String, Peer>,
    pub link: Option<Link>,
    pub phase: Phase,
    pub session: Option<Session>,
    pub data_dir: PathBuf,
    pub pending: Option<(TimeControl, String)>,
    pub peer_addr: Option<SocketAddr>,
}

pub struct Shared {
    pub app: AppHandle,
    pub my_id: String,
    pub game_port: AtomicU16,
    pub conn_seq: AtomicU64,
    pub inner: Mutex<Inner>,
}

impl Shared {
    pub fn new(app: AppHandle, data_dir: PathBuf, profile: Option<Profile>) -> Shared {
        let id_path = data_dir.join("id.txt");
        let my_id = std::fs::read_to_string(&id_path)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| s.len() == 16)
            .unwrap_or_else(|| {
                let id = random_alnum(16);
                let _ = std::fs::write(&id_path, &id);
                id
            });
        Shared {
            app,
            my_id,
            game_port: AtomicU16::new(GAME_PORT),
            conn_seq: AtomicU64::new(0),
            inner: Mutex::new(Inner { profile, peers: HashMap::new(), link: None, phase: Phase::Idle, session: None, data_dir, pending: None, peer_addr: None }),
        }
    }

    pub fn emit<S: Serialize + Clone>(&self, ev: &str, payload: S) {
        let _ = self.app.emit(ev, payload);
    }
    pub fn emit_state(&self) {
        self.emit("state-changed", ());
    }

    pub fn me_dto(&self, inner: &Inner) -> PlayerDto {
        let p = inner.profile.as_ref();
        PlayerDto {
            id: self.my_id.clone(),
            name: p.map(|p| p.name.clone()).unwrap_or_default(),
            os: my_os(),
            avatar: p.and_then(|p| p.avatar_data_url.clone()),
        }
    }

    pub fn wire_profile(&self, inner: &Inner) -> WireProfile {
        let p = inner.profile.as_ref();
        WireProfile {
            id: self.my_id.clone(),
            name: p.map(|p| p.name.clone()).unwrap_or_default(),
            os: my_os(),
            avatar: p.and_then(|p| p.thumb_b64.clone()),
        }
    }
}

pub fn send(inner: &Inner, msg: Msg) {
    if let Some(l) = &inner.link {
        let _ = l.tx.send(msg);
    }
}

// ---------- session & DTOs ----------

#[derive(Serialize)]
pub struct PieceDto {
    pub sq: u8,
    pub color: &'static str,
    pub kind: &'static str,
}
#[derive(Serialize)]
pub struct MoveDto {
    pub from: u8,
    pub to: u8,
    pub promo: Option<&'static str>,
}
#[derive(Serialize)]
pub struct CapDto {
    pub color: &'static str,
    pub kind: &'static str,
}
#[derive(Serialize)]
pub struct EndDto {
    pub kind: &'static str,
    pub winner: Option<&'static str>,
}
#[derive(Serialize)]
pub struct OpeningDto {
    pub eco: String,
    pub name: String,
}
#[derive(Serialize)]
pub struct ClockDto {
    pub initial_ms: u64,
    pub bonus_ms: u64,
    pub white_ms: i64,
    pub black_ms: i64,
    pub running: Option<&'static str>,
}
#[derive(Serialize)]
pub struct ReconnectDto {
    pub state: &'static str,
    pub attempt: u8,
    pub max: u8,
}
#[derive(Serialize)]
pub struct SessionDto {
    pub clock: Option<ClockDto>,
    pub reconnect: ReconnectDto,
    pub opp_left: bool,
    pub me: PlayerDto,
    pub opponent: PlayerDto,
    pub my_color: &'static str,
    pub turn: &'static str,
    pub pieces: Vec<PieceDto>,
    pub legal: Vec<MoveDto>,
    pub check_sq: Option<u8>,
    pub last_move: Option<(u8, u8)>,
    pub history: Vec<String>,
    pub position_history: Vec<String>,
    pub move_history: Vec<(u8, u8)>,
    pub captured_by_me: Vec<CapDto>,
    pub captured_by_opp: Vec<CapDto>,
    pub opening: Option<OpeningDto>,
    pub end: Option<EndDto>,
    pub connected: bool,
    pub draw_in: bool,
    pub draw_out: bool,
    pub rematch_in: bool,
    pub rematch_out: bool,
    pub game_no: u32,
    pub ply: usize,
}

impl Session {
    pub fn new(me: PlayerDto, opp: PlayerDto, my_color: Color, game_no: u32, tc: TimeControl, sid: String, dialer: bool) -> Session {
        Session {
            sid,
            dialer,
            tc,
            clock: Session::mk_clock(tc),
            opp_left: false,
            reconnect: Reconnect::None,
            me,
            opp,
            my_color,
            game: Game::new(),
            forced: None,
            connected: true,
            draw_in: false,
            draw_out: false,
            rematch_in: false,
            rematch_out: false,
            game_no,
        }
    }

    fn mk_clock(tc: TimeControl) -> Option<Clock> {
        if tc.enabled() {
            Some(Clock { white_ms: tc.initial_ms as i64, black_ms: tc.initial_ms as i64, last: None })
        } else {
            None
        }
    }

    // ----- clock -----
    /// Remaining ms for `c` right now (may be negative once flagged).
    pub fn remaining(&self, c: Color) -> i64 {
        let Some(cl) = &self.clock else { return 0 };
        let base = if c == Color::White { cl.white_ms } else { cl.black_ms };
        let el = match cl.last {
            Some(t) if c == self.game.pos.side => t.elapsed().as_millis() as i64,
            _ => 0,
        };
        base - el
    }
    pub fn running(&self) -> Option<Color> {
        self.clock.as_ref().and_then(|c| c.last.map(|_| self.game.pos.side))
    }
    pub fn my_clock_report(&self) -> Option<i64> {
        self.clock.as_ref().map(|_| self.remaining(self.my_color).max(0))
    }
    /// Call right after a move was played. `reported` = mover's own clock value from the network.
    /// Clocks start once both players have moved (like lichess).
    pub fn after_move(&mut self, mover: Color, ply_before: usize, reported: Option<i64>) {
        let ended = self.end().is_some();
        let bonus = self.tc.bonus_ms as i64;
        let Some(cl) = self.clock.as_mut() else { return };
        if ply_before >= 2 {
            let el = cl.last.map(|t| t.elapsed().as_millis() as i64).unwrap_or(0);
            let cur = if mover == Color::White { cl.white_ms } else { cl.black_ms };
            let v = match reported {
                Some(ms) => ms.max(0),
                None => (cur - el).max(0) + bonus,
            };
            if mover == Color::White { cl.white_ms = v } else { cl.black_ms = v }
        }
        cl.last = if ply_before + 1 >= 2 && !ended { Some(Instant::now()) } else { None };
    }
    pub fn pause_clock(&mut self) {
        let side = self.game.pos.side;
        if let Some(cl) = self.clock.as_mut() {
            if let Some(t) = cl.last.take() {
                let el = t.elapsed().as_millis() as i64;
                if side == Color::White { cl.white_ms -= el } else { cl.black_ms -= el }
            }
        }
    }
    pub fn resume_clock(&mut self) {
        let ended = self.end().is_some();
        let ply = self.game.moves.len();
        if let Some(cl) = self.clock.as_mut() {
            if ply >= 2 && !ended && cl.last.is_none() {
                cl.last = Some(Instant::now());
            }
        }
    }
    /// End the game by a non-board reason (resign, agreement, timeout), freezing the clock.
    pub fn finish(&mut self, e: GameEnd) {
        self.pause_clock();
        self.forced = Some(e);
    }

    // ----- reconnect sync -----
    pub fn uci_moves(&self) -> Vec<String> {
        self.game.moves.iter().map(|r| r.mv.uci()).collect()
    }
    /// Reconcile with the other side's full move list. Applies moves we are missing.
    pub fn apply_resume(&mut self, theirs: &[String], opp_clock: Option<i64>) -> Result<(), String> {
        let mine = self.uci_moves();
        let n = mine.len().min(theirs.len());
        if mine[..n] != theirs[..n] {
            return Err("game histories diverged".into());
        }
        if theirs.len() > mine.len() {
            for u in &theirs[mine.len()..] {
                self.game.play_uci(u)?;
            }
        }
        let opp = self.my_color.opp();
        let ended = self.end().is_some();
        if let (Some(cl), Some(ms)) = (self.clock.as_mut(), opp_clock) {
            if opp == Color::White { cl.white_ms = ms } else { cl.black_ms = ms }
            if ended { cl.last = None }
        }
        Ok(())
    }

    pub fn end(&self) -> Option<GameEnd> {
        if let Some(e) = self.forced {
            return Some(e);
        }
        let mk = |kind, winner| Some(GameEnd { kind, winner });
        match self.game.outcome() {
            Outcome::Ongoing => None,
            Outcome::Checkmate { winner } => mk(EndKind::Checkmate, Some(winner)),
            Outcome::Stalemate => mk(EndKind::Stalemate, None),
            Outcome::DrawFiftyMove => mk(EndKind::FiftyMove, None),
            Outcome::DrawThreefold => mk(EndKind::Threefold, None),
            Outcome::DrawInsufficient => mk(EndKind::Insufficient, None),
        }
    }

    pub fn restart_swapped(&mut self) {
        self.my_color = self.my_color.opp();
        self.game = Game::new();
        self.clock = Session::mk_clock(self.tc);
        self.forced = None;
        self.draw_in = false;
        self.draw_out = false;
        self.rematch_in = false;
        self.rematch_out = false;
        self.game_no += 1;
    }

    pub fn dto(&self) -> SessionDto {
        let pos = &self.game.pos;
        let end = self.end();
        let pieces = (0..64u8)
            .filter_map(|sq| pos.board[sq as usize].map(|p| PieceDto { sq, color: p.color.name(), kind: p.kind.name() }))
            .collect();
        let legal = if end.is_none() && self.connected && pos.side == self.my_color {
            pos.legal_moves().into_iter().map(|m| MoveDto { from: m.from, to: m.to, promo: m.promo.map(|k| k.name()) }).collect()
        } else {
            vec![]
        };
        let mut by_me: Vec<(i32, CapDto)> = vec![];
        let mut by_opp: Vec<(i32, CapDto)> = vec![];
        for r in &self.game.moves {
            if let Some(c) = r.captured {
                let d = CapDto { color: c.color.name(), kind: c.kind.name() };
                if r.mover == self.my_color { by_me.push((c.kind.value(), d)) } else { by_opp.push((c.kind.value(), d)) }
            }
        }
        by_me.sort_by(|a, b| b.0.cmp(&a.0));
        by_opp.sort_by(|a, b| b.0.cmp(&a.0));
        let opening = if self.game.moves.is_empty() {
            None
        } else {
            Some(match eco::lookup(self.game.keys()) {
                Some((e, n)) => OpeningDto { eco: e.into(), name: n.into() },
                None => OpeningDto { eco: String::new(), name: "Unlisted opening".into() },
            })
        };
        let clock = self.clock.as_ref().map(|_| ClockDto {
            initial_ms: self.tc.initial_ms,
            bonus_ms: self.tc.bonus_ms,
            white_ms: self.remaining(Color::White).max(0),
            black_ms: self.remaining(Color::Black).max(0),
            running: self.running().map(|c| c.name()),
        });
        let reconnect = match self.reconnect {
            Reconnect::None => ReconnectDto { state: "none", attempt: 0, max: RECONNECT_POLLS },
            Reconnect::Trying(n) => ReconnectDto { state: "trying", attempt: n, max: RECONNECT_POLLS },
            Reconnect::GaveUp => ReconnectDto { state: "gave_up", attempt: RECONNECT_POLLS, max: RECONNECT_POLLS },
        };
        SessionDto {
            clock,
            reconnect,
            opp_left: self.opp_left,
            me: self.me.clone(),
            opponent: self.opp.clone(),
            my_color: self.my_color.name(),
            turn: pos.side.name(),
            pieces,
            legal,
            check_sq: if pos.in_check() { pos.king_sq(pos.side) } else { None },
            last_move: self.game.moves.last().map(|r| (r.mv.from, r.mv.to)),
            history: self.game.moves.iter().map(|r| r.san.clone()).collect(),
            position_history: self.game.moves.iter().map(|r| r.fen_before.clone()).chain(std::iter::once(pos.to_fen())).collect(),
            move_history: self.game.moves.iter().map(|r| (r.mv.from, r.mv.to)).collect(),
            captured_by_me: by_me.into_iter().map(|x| x.1).collect(),
            captured_by_opp: by_opp.into_iter().map(|x| x.1).collect(),
            opening,
            end: end.map(|e| EndDto {
                kind: match e.kind {
                    EndKind::Checkmate => "checkmate",
                    EndKind::Stalemate => "stalemate",
                    EndKind::FiftyMove => "fifty_move",
                    EndKind::Threefold => "threefold",
                    EndKind::Insufficient => "insufficient",
                    EndKind::Resigned => "resigned",
                    EndKind::DrawAgreed => "draw_agreed",
                    EndKind::Timeout => "timeout",
                },
                winner: e.winner.map(|c| c.name()),
            }),
            connected: self.connected,
            draw_in: self.draw_in,
            draw_out: self.draw_out,
            rematch_in: self.rematch_in,
            rematch_out: self.rematch_out,
            game_no: self.game_no,
            ply: self.game.moves.len(),
        }
    }
}
