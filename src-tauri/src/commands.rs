use crate::{net, protocol::*, state::*};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use cheslan_core::{eco, game::Outcome, review, types::*};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};
use tauri::State;

type Sh<'a> = State<'a, Arc<Shared>>;

#[derive(Serialize)]
pub struct ProfileDto {
    pub id: String,
    pub name: String,
    pub os: String,
    pub avatar: Option<String>,
}

#[derive(Deserialize)]
pub struct GameSettings {
    /// 0 = no timer
    pub initial_ms: u64,
    pub bonus_ms: u64,
    /// "white" | "black" | "random" (the challenger's colour)
    pub color: String,
}

impl GameSettings {
    fn parse(self) -> Result<(TimeControl, String), String> {
        let tc = TimeControl { initial_ms: self.initial_ms, bonus_ms: self.bonus_ms };
        tc.validate()?;
        if !valid_color_pref(&self.color) {
            return Err("Bad colour choice.".into());
        }
        Ok((tc, self.color))
    }
}

#[derive(Serialize)]
pub struct SettingsDto {
    pub initial_ms: u64,
    pub bonus_ms: u64,
    pub color: String,
}

#[derive(Serialize)]
pub struct PhaseDto {
    pub phase: &'static str,
    pub peer: Option<PlayerDto>,
    pub settings: Option<SettingsDto>,
}

#[derive(Serialize)]
pub struct ReviewPly {
    pub mover: &'static str,
    pub san: String,
    pub uci: String,
    pub fen_before: String,
    pub fen_after: String,
    /// "none" | "mate" (mover delivered mate) | "draw"
    pub after_state: &'static str,
    pub sac_cp: i32,
    pub book: bool,
}

#[tauri::command]
pub fn get_profile(sh: Sh<'_>) -> Option<ProfileDto> {
    let g = sh.inner.lock().unwrap();
    g.profile.as_ref().map(|p| ProfileDto { id: sh.my_id.clone(), name: p.name.clone(), os: my_os(), avatar: p.avatar_data_url.clone() })
}

#[tauri::command]
pub fn save_profile(sh: Sh<'_>, name: String, avatar_png: Option<String>, avatar_thumb: Option<String>) -> Result<ProfileDto, String> {
    let name = {
        let n = clean_name(&name);
        if n.is_empty() { format!("user{}", random_alnum(8)) } else { n }
    };
    let mut g = sh.inner.lock().unwrap();
    let dir = g.data_dir.clone();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut prof = g.profile.clone().unwrap_or(Profile { name: name.clone(), avatar_data_url: None, thumb_b64: None, avatar_hash: String::new() });
    prof.name = name.clone();
    if let (Some(png_b64), Some(thumb_b64)) = (avatar_png, avatar_thumb) {
        let png = B64.decode(png_b64.trim()).map_err(|_| "bad image data")?;
        let thumb = B64.decode(thumb_b64.trim()).map_err(|_| "bad thumbnail data")?;
        if png.len() > 2_000_000 || png.len() < 8 || png[..4] != [0x89, b'P', b'N', b'G'] {
            return Err("Avatar must be a PNG under 2 MB.".into());
        }
        if thumb.len() > MAX_THUMB_BYTES || thumb.len() < 4 || thumb[..3] != [0xFF, 0xD8, 0xFF] {
            return Err("Thumbnail must be a JPEG under 64 KB.".into());
        }
        // Copy the cropped image into app-local storage (never reference the original path).
        std::fs::write(dir.join("avatar.png"), &png).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("avatar_thumb.jpg"), &thumb).map_err(|e| e.to_string())?;
        prof.avatar_data_url = Some(format!("data:image/png;base64,{}", B64.encode(&png)));
        prof.avatar_hash = hash_bytes(&thumb);
        prof.thumb_b64 = Some(B64.encode(&thumb));
    }
    std::fs::write(dir.join("profile.json"), serde_json::json!({ "name": name }).to_string()).map_err(|e| e.to_string())?;
    let dto = ProfileDto { id: sh.my_id.clone(), name, os: my_os(), avatar: prof.avatar_data_url.clone() };
    g.profile = Some(prof);
    Ok(dto)
}

#[tauri::command]
pub fn list_peers(sh: Sh<'_>) -> Vec<PlayerDto> {
    let g = sh.inner.lock().unwrap();
    let mut v: Vec<PlayerDto> = g.peers.values().map(|p| p.info.clone()).collect();
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    v
}

#[tauri::command]
pub async fn challenge(sh: Sh<'_>, peer_id: String, settings: GameSettings) -> Result<(), String> {
    let (tc, color) = settings.parse()?;
    net::challenge_peer(sh.inner().clone(), peer_id, tc, color).await
}

#[tauri::command]
pub async fn challenge_ip(sh: Sh<'_>, addr: String, settings: GameSettings) -> Result<(), String> {
    let (tc, color) = settings.parse()?;
    let addr = addr.trim();
    let sa = if let Ok(sa) = addr.parse::<SocketAddr>() {
        sa
    } else if let Ok(ip) = addr.parse::<IpAddr>() {
        SocketAddr::new(ip, GAME_PORT)
    } else {
        return Err("Enter an IPv4 address such as 192.168.1.20 (optionally :port).".into());
    };
    net::challenge_addr(sh.inner().clone(), sa, tc, color).await
}

#[tauri::command]
pub fn respond_challenge(sh: Sh<'_>, accept: bool) -> Result<(), String> {
    {
        let mut g = sh.inner.lock().unwrap();
        let inner = &mut *g;
        let Phase::Incoming(opp) = inner.phase.clone() else { return Err("No pending challenge.".into()) };
        if accept {
            let (tc, pref) = inner.pending.take().unwrap_or((TimeControl::default(), "random".into()));
            let challenger = match pref.as_str() {
                "white" => Color::White,
                "black" => Color::Black,
                _ => if rand::thread_rng().gen::<bool>() { Color::White } else { Color::Black },
            };
            let me = sh.me_dto(inner);
            let wire = sh.wire_profile(inner);
            let sid = random_alnum(16);
            inner.session = Some(Session::new(me, opp, challenger.opp(), 1, tc, sid.clone(), false));
            inner.phase = Phase::Playing;
            send(inner, Msg::Accept { your_color: challenger, profile: wire, tc, sid });
        } else {
            send(inner, Msg::Decline);
            inner.link = None;
            inner.phase = Phase::Idle;
        }
    }
    sh.emit_state();
    Ok(())
}

#[tauri::command]
pub fn cancel_challenge(sh: Sh<'_>) {
    {
        let mut g = sh.inner.lock().unwrap();
        let inner = &mut *g;
        if inner.phase.tag() == "outgoing" {
            send(inner, Msg::Leave);
            inner.link = None;
            inner.phase = Phase::Idle;
        }
    }
    sh.emit_state();
}

#[tauri::command]
pub fn get_phase(sh: Sh<'_>) -> PhaseDto {
    let g = sh.inner.lock().unwrap();
    let peer = match &g.phase {
        Phase::Outgoing(p) | Phase::Incoming(p) => Some(p.clone()),
        _ => None,
    };
    let settings = g.pending.as_ref().map(|(tc, c)| SettingsDto { initial_ms: tc.initial_ms, bonus_ms: tc.bonus_ms, color: c.clone() });
    PhaseDto { phase: g.phase.tag(), peer, settings }
}

#[tauri::command]
pub fn get_session(sh: Sh<'_>) -> Option<SessionDto> {
    sh.inner.lock().unwrap().session.as_ref().map(|s| s.dto())
}

#[tauri::command]
pub fn make_move(sh: Sh<'_>, uci: String) -> Result<SessionDto, String> {
    let mut g = sh.inner.lock().unwrap();
    let inner = &mut *g;
    let s = inner.session.as_mut().ok_or("No game in progress.".to_string())?;
    if !s.connected {
        return Err("Opponent is disconnected.".into());
    }
    if s.end().is_some() {
        return Err("The game is over.".into());
    }
    if s.game.pos.side != s.my_color {
        return Err("It's not your turn.".into());
    }
    let m = Move::parse_uci(&uci).ok_or("Malformed move.".to_string())?;
    let ply_before = s.game.moves.len();
    let mover = s.my_color;
    s.game.play(m)?;
    s.draw_in = false;
    s.draw_out = false;
    s.after_move(mover, ply_before, None);
    let clock_ms = s.my_clock_report();
    let dto = s.dto();
    send(inner, Msg::Move { uci, ply: ply_before as u32, clock_ms });
    Ok(dto)
}

fn with_live_session<F: FnOnce(&mut Session, &Inner) -> Option<Msg>>(sh: &Arc<Shared>, f: F) -> Result<(), String> {
    {
        let mut g = sh.inner.lock().unwrap();
        let inner = &mut *g;
        let mut s = inner.session.take().ok_or("No game in progress.".to_string())?;
        let msg = f(&mut s, inner);
        inner.session = Some(s);
        if let Some(m) = msg {
            send(inner, m);
        }
    }
    sh.emit_state();
    Ok(())
}

#[tauri::command]
pub fn resign(sh: Sh<'_>) -> Result<(), String> {
    with_live_session(sh.inner(), |s, _| {
        if s.end().is_some() || !s.connected {
            return None;
        }
        s.finish(GameEnd { kind: EndKind::Resigned, winner: Some(s.my_color.opp()) });
        Some(Msg::Resign)
    })
}

#[tauri::command]
pub fn offer_draw(sh: Sh<'_>) -> Result<(), String> {
    with_live_session(sh.inner(), |s, _| {
        if s.end().is_some() || !s.connected || s.draw_out {
            return None;
        }
        s.draw_out = true;
        Some(Msg::DrawOffer)
    })
}

#[tauri::command]
pub fn respond_draw(sh: Sh<'_>, accept: bool) -> Result<(), String> {
    with_live_session(sh.inner(), |s, _| {
        if !s.draw_in || s.end().is_some() {
            return None;
        }
        s.draw_in = false;
        if accept {
            s.finish(GameEnd { kind: EndKind::DrawAgreed, winner: None });
            Some(Msg::DrawAccept)
        } else {
            Some(Msg::DrawDecline)
        }
    })
}

#[tauri::command]
pub fn send_chat(sh: Sh<'_>, text: String) -> Result<String, String> {
    let t = clean_text(&text, MAX_CHAT_CHARS);
    if t.is_empty() {
        return Err("Empty message.".into());
    }
    let g = sh.inner.lock().unwrap();
    if g.session.as_ref().map(|s| s.connected) != Some(true) {
        return Err("Opponent is disconnected.".into());
    }
    send(&g, Msg::Chat { text: t.clone() });
    Ok(t)
}

#[tauri::command]
pub fn request_rematch(sh: Sh<'_>) -> Result<(), String> {
    with_live_session(sh.inner(), |s, _| {
        if s.end().is_none() || !s.connected || s.rematch_out {
            return None;
        }
        if s.rematch_in {
            s.restart_swapped();
            return Some(Msg::RematchAccept);
        }
        s.rematch_out = true;
        Some(Msg::RematchReq)
    })
}

#[tauri::command]
pub fn respond_rematch(sh: Sh<'_>, accept: bool) -> Result<(), String> {
    with_live_session(sh.inner(), |s, _| {
        if !s.rematch_in || !s.connected {
            return None;
        }
        if accept {
            s.restart_swapped();
            Some(Msg::RematchAccept)
        } else {
            s.rematch_in = false;
            Some(Msg::RematchDecline)
        }
    })
}

#[tauri::command]
pub fn leave_game(sh: Sh<'_>) {
    {
        let mut g = sh.inner.lock().unwrap();
        let inner = &mut *g;
        send(inner, Msg::Leave);
        inner.link = None;
        inner.session = None;
        inner.pending = None;
        inner.peer_addr = None;
        inner.phase = Phase::Idle;
    }
    sh.emit_state();
}

#[tauri::command]
pub fn get_pgn(sh: Sh<'_>) -> Result<String, String> {
    let g = sh.inner.lock().unwrap();
    let s = g.session.as_ref().ok_or("No game.".to_string())?;
    let (w, b) = if s.my_color == Color::White { (&s.me.name, &s.opp.name) } else { (&s.opp.name, &s.me.name) };
    let result = match s.end() {
        Some(e) => match e.winner {
            Some(Color::White) => "1-0",
            Some(Color::Black) => "0-1",
            None => "1/2-1/2",
        },
        None => "*",
    };
    Ok(s.game.pgn(w, b, result))
}

#[tauri::command]
pub fn get_review_input(sh: Sh<'_>) -> Result<Vec<ReviewPly>, String> {
    let g = sh.inner.lock().unwrap();
    let s = g.session.as_ref().ok_or("No game.".to_string())?;
    let game = &s.game;
    let last_state = match game.outcome() {
        Outcome::Checkmate { .. } => "mate",
        Outcome::Stalemate | Outcome::DrawFiftyMove | Outcome::DrawThreefold | Outcome::DrawInsufficient => "draw",
        Outcome::Ongoing => "none",
    };
    let n = game.moves.len();
    Ok(game
        .moves
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let fen_after = if i + 1 < n { game.moves[i + 1].fen_before.clone() } else { game.pos.to_fen() };
            ReviewPly {
                mover: r.mover.name(),
                san: r.san.clone(),
                uci: r.mv.uci(),
                sac_cp: review::sacrifice_cp(&r.fen_before, &r.mv.uci()),
                fen_before: r.fen_before.clone(),
                fen_after,
                after_state: if i + 1 == n { last_state } else { "none" },
                book: eco::contains(&game.keys()[i + 1]),
            }
        })
        .collect())
}
