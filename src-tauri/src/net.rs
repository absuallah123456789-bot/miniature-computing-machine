//! LAN networking.
//! * Discovery: UDP broadcast beacons on DISCOVERY_PORT every 2 s (to 255.255.255.255 and each
//!   interface's subnet broadcast). Peers expire after 7 s without a beacon.
//! * Game channel: one TCP connection per game, length-prefixed JSON frames, 5 s pings,
//!   15 s silence = disconnect.
use crate::protocol::*;
use crate::state::*;
use cheslan_core::game::{Game, Outcome};
use cheslan_core::types::*;
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    io,
    net::{IpAddr, SocketAddr},
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{tcp::{OwnedReadHalf, OwnedWriteHalf}, TcpListener, TcpStream, UdpSocket},
    sync::mpsc,
    time::{interval, sleep, timeout},
};

// ---------- framing ----------

async fn write_msg<W: AsyncWriteExt + Unpin>(w: &mut W, m: &Msg) -> io::Result<()> {
    let body = serde_json::to_vec(m).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if body.len() > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    w.write_all(&(body.len() as u32).to_be_bytes()).await?;
    w.write_all(&body).await?;
    w.flush().await
}

async fn read_msg<R: AsyncReadExt + Unpin>(r: &mut R) -> io::Result<Msg> {
    let mut lenb = [0u8; 4];
    r.read_exact(&mut lenb).await?;
    let len = u32::from_be_bytes(lenb) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad frame length"));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    serde_json::from_slice(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn player_from_wire(w: WireProfile) -> PlayerDto {
    PlayerDto { id: clean_text(&w.id, 32), name: peer_name(&w.name), os: clean_os(&w.os), avatar: thumb_data_url(w.avatar.as_deref()), in_game: false }
}

// ---------- discovery ----------

fn bind_udp(port: u16) -> io::Result<UdpSocket> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
    let _ = s.set_reuse_port(true);
    s.set_broadcast(true)?;
    s.set_nonblocking(true)?;
    s.bind(&SocketAddr::from(([0, 0, 0, 0], port)).into())?;
    UdpSocket::from_std(s.into())
}

fn broadcast_targets() -> Vec<SocketAddr> {
    let mut v = vec![SocketAddr::from(([255, 255, 255, 255], DISCOVERY_PORT))];
    if let Ok(ifs) = if_addrs::get_if_addrs() {
        for i in ifs {
            if i.is_loopback() {
                continue;
            }
            if let if_addrs::IfAddr::V4(a) = i.addr {
                if let Some(b) = a.broadcast {
                    v.push(SocketAddr::new(IpAddr::V4(b), DISCOVERY_PORT));
                }
            }
        }
    }
    v.sort();
    v.dedup();
    v
}

fn emit_peers(sh: &Arc<Shared>) {
    let mut list: Vec<PlayerDto> = {
        let g = sh.inner.lock().unwrap();
        g.peers.values().map(|p| p.info.clone()).collect()
    };
    list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    sh.emit("peers-changed", list);
}

pub async fn run_discovery(sh: Arc<Shared>) {
    let sock = match bind_udp(DISCOVERY_PORT) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            sh.emit("net-error", format!("LAN discovery unavailable: {e}. You can still connect by IP."));
            return;
        }
    };

    // beacon sender
    {
        let sock = sock.clone();
        let sh = sh.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let beacon = {
                    let g = sh.inner.lock().unwrap();
                    let in_game = matches!(g.phase, Phase::Playing) && g.session.is_some();
                    g.profile.as_ref().map(|p| Beacon {
                        app: "cheslan".into(),
                        v: PROTO_VERSION,
                        id: sh.my_id.clone(),
                        name: p.name.clone(),
                        os: my_os(),
                        port: sh.game_port.load(Ordering::SeqCst),
                        avatar_hash: p.avatar_hash.clone(),
                        in_game,
                    })
                };
                if let Some(b) = beacon {
                    if let Ok(data) = serde_json::to_vec(&b) {
                        for t in broadcast_targets() {
                            let _ = sock.send_to(&data, t).await;
                        }
                    }
                }
                sleep(Duration::from_secs(2)).await;
            }
        });
    }

    // reaper
    {
        let sh = sh.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                sleep(Duration::from_secs(2)).await;
                let changed = {
                    let mut g = sh.inner.lock().unwrap();
                    let before = g.peers.len();
                    g.peers.retain(|_, p| p.last_seen.elapsed() < Duration::from_secs(7));
                    g.peers.len() != before
                };
                if changed {
                    emit_peers(&sh);
                }
            }
        });
    }

    // receiver
    let mut buf = [0u8; 2048];
    loop {
        match sock.recv_from(&mut buf).await {
            Ok((n, from)) => handle_beacon(&sh, &buf[..n], from.ip()),
            Err(_) => sleep(Duration::from_millis(100)).await,
        }
    }
}

fn handle_beacon(sh: &Arc<Shared>, data: &[u8], from: IpAddr) {
    let Ok(b) = serde_json::from_slice::<Beacon>(data) else { return };
    if b.app != "cheslan" || b.v != PROTO_VERSION || b.id == sh.my_id || b.id.len() > 32 {
        return;
    }
    let (mut fetch, mut changed) = (false, false);
    {
        let mut g = sh.inner.lock().unwrap();
        let name = peer_name(&b.name);
        let os = clean_os(&b.os);
        let hash = clean_text(&b.avatar_hash, 32);
        match g.peers.get_mut(&b.id) {
            Some(p) => {
                if p.info.name != name || p.ip != from || p.port != b.port || p.info.os != os || p.info.in_game != b.in_game {
                    changed = true;
                }
                p.info.name = name;
                p.info.os = os;
                p.info.in_game = b.in_game;
                p.ip = from;
                p.port = b.port;
                p.last_seen = Instant::now();
                if p.avatar_hash != hash {
                    p.avatar_hash = hash;
                }
                if p.fetched_hash != p.avatar_hash && !p.fetching {
                    p.fetching = true;
                    fetch = true;
                }
            }
            None => {
                g.peers.insert(
                    b.id.clone(),
                    Peer {
                        info: PlayerDto { id: b.id.clone(), name, os, avatar: None, in_game: b.in_game },
                        ip: from,
                        port: b.port,
                        avatar_hash: hash,
                        fetched_hash: "~".into(),
                        fetching: true,
                        last_seen: Instant::now(),
                    },
                );
                fetch = true;
                changed = true;
            }
        }
    }
    if changed {
        emit_peers(sh);
    }
    if fetch {
        tauri::async_runtime::spawn(fetch_profile(sh.clone(), b.id, from, b.port));
    }
}

async fn fetch_profile(sh: Arc<Shared>, id: String, ip: IpAddr, port: u16) {
    let res = async {
        let mut s = timeout(Duration::from_secs(3), TcpStream::connect(SocketAddr::new(ip, port))).await.ok()?.ok()?;
        write_msg(&mut s, &Msg::ProfileReq).await.ok()?;
        match timeout(Duration::from_secs(3), read_msg(&mut s)).await.ok()?.ok()? {
            Msg::Profile { profile } => Some(profile),
            _ => None,
        }
    }
    .await;
    let mut changed = false;
    {
        let mut g = sh.inner.lock().unwrap();
        if let Some(p) = g.peers.get_mut(&id) {
            p.fetching = false;
            if let Some(w) = res {
                p.fetched_hash = p.avatar_hash.clone();
                p.info.avatar = thumb_data_url(w.avatar.as_deref());
                p.info.name = peer_name(&w.name);
                changed = true;
            }
        }
    }
    if changed {
        emit_peers(&sh);
    }
}

// ---------- server ----------

pub async fn run_server(sh: Arc<Shared>, listener: TcpListener) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tauri::async_runtime::spawn(handle_incoming(sh.clone(), stream));
            }
            Err(_) => sleep(Duration::from_millis(200)).await,
        }
    }
}

async fn handle_incoming(sh: Arc<Shared>, stream: TcpStream) {
    let _ = stream.set_nodelay(true);
    let (mut rd, mut wr) = stream.into_split();
    let first = match timeout(Duration::from_secs(5), read_msg(&mut rd)).await {
        Ok(Ok(m)) => m,
        _ => return,
    };
    match first {
        Msg::ProfileReq => {
            let p = {
                let g = sh.inner.lock().unwrap();
                sh.wire_profile(&g)
            };
            let _ = write_msg(&mut wr, &Msg::Profile { profile: p }).await;
        }
        Msg::Challenge { profile, v, tc, color, pgn } => {
            let (tx, rx) = mpsc::unbounded_channel();
            let conn_id = sh.conn_seq.fetch_add(1, Ordering::SeqCst) + 1;
            let who = player_from_wire(profile);
            let pgn = pgn.filter(|t| !t.trim().is_empty());
            let pgn_ok = match &pgn {
                Some(t) => Game::from_pgn(t).map(|g| g.outcome() == Outcome::Ongoing).unwrap_or(false),
                None => true,
            };
            let accepted = {
                let mut g = sh.inner.lock().unwrap();
                let inner = &mut *g;
                if v == PROTO_VERSION && pgn_ok && tc.validate().is_ok() && valid_color_pref(&color) && matches!(inner.phase, Phase::Idle) && inner.profile.is_some() {
                    inner.phase = Phase::Incoming(who.clone());
                    inner.pending = Some((tc, color));
                    inner.pending_pgn = pgn;
                    inner.link = Some(Link { id: conn_id, tx });
                    true
                } else {
                    false
                }
            };
            if !accepted {
                let _ = write_msg(&mut wr, &Msg::Busy).await;
                return;
            }
            sh.emit("challenge-received", who);
            sh.emit_state();
            run_connection(sh, rd, wr, rx, conn_id).await;
        }
        Msg::Spectate { profile, v } => {
            let who = player_from_wire(profile);
            let (tx, rx) = mpsc::unbounded_channel();
            let spec_id = sh.conn_seq.fetch_add(1, Ordering::SeqCst) + 1;
            let ok = {
                let mut g = sh.inner.lock().unwrap();
                let inner = &mut *g;
                if v == PROTO_VERSION && matches!(inner.phase, Phase::Playing) && inner.session.is_some() {
                    let host_wire = sh.wire_profile(inner);
                    let (welcome, sync) = {
                        let s = inner.session.as_ref().unwrap();
                        (
                            Msg::SpecWelcome { host: host_wire, other: dto_to_wire(&s.opp), tc: s.tc, start_fen: Some(s.game.start.to_fen()) },
                            Msg::SpecSync { data: s.spec_sync() },
                        )
                    };
                    let _ = tx.send(welcome);
                    let _ = tx.send(sync);
                    inner.spectators.push(SpecLink { id: spec_id, name: who.name.clone(), tx });
                    // let the other player know too
                    send(inner, Msg::SpecJoined { name: who.name.clone() });
                    true
                } else {
                    false
                }
            };
            if !ok {
                let _ = write_msg(&mut wr, &Msg::Busy).await;
                return;
            }
            sh.emit("spectator-joined", who.name.clone());
            run_spec_conn(sh, rd, wr, rx, spec_id).await;
        }
        Msg::Resume { sid, moves, clock_ms } => {
            let (tx, rx) = mpsc::unbounded_channel();
            let conn_id = sh.conn_seq.fetch_add(1, Ordering::SeqCst) + 1;
            let ok = {
                let mut g = sh.inner.lock().unwrap();
                let inner = &mut *g;
                match inner.session.as_mut() {
                    Some(s) if s.sid == sid && !s.opp_left => {
                        if s.apply_resume(&moves, clock_ms).is_ok() {
                            s.connected = true;
                            s.reconnect = Reconnect::None;
                            s.resume_clock();
                            let _ = tx.send(Msg::ResumeOk { moves: s.uci_moves(), clock_ms: s.my_clock_report() });
                            // A valid resume replaces any half-dead old link.
                            inner.link = Some(Link { id: conn_id, tx });
                            inner.phase = Phase::Playing;
                            true
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            };
            if !ok {
                let _ = write_msg(&mut wr, &Msg::Busy).await;
                return;
            }
            sh.emit("peer-reconnected", ());
            sh.emit_state();
            run_connection(sh, rd, wr, rx, conn_id).await;
        }
        _ => {}
    }
}

// ---------- outgoing challenge ----------

fn can_challenge(sh: &Arc<Shared>) -> Result<(), String> {
    let g = sh.inner.lock().unwrap();
    if !matches!(g.phase, Phase::Idle) {
        return Err("You're already in a game or a challenge.".into());
    }
    if g.profile.is_none() {
        return Err("Save your profile first.".into());
    }
    Ok(())
}

pub async fn challenge_peer(sh: Arc<Shared>, peer_id: String, tc: TimeControl, color: String, pgn: Option<String>) -> Result<(), String> {
    can_challenge(&sh)?;
    let (addr, who) = {
        let g = sh.inner.lock().unwrap();
        let p = g.peers.get(&peer_id).ok_or_else(|| "That player is no longer visible.".to_string())?;
        (SocketAddr::new(p.ip, p.port), p.info.clone())
    };
    do_challenge(sh, addr, who, tc, color, pgn).await
}

pub async fn challenge_addr(sh: Arc<Shared>, addr: SocketAddr, tc: TimeControl, color: String, pgn: Option<String>) -> Result<(), String> {
    can_challenge(&sh)?;
    let who = PlayerDto { id: String::new(), name: addr.ip().to_string(), os: "Other".into(), avatar: None, in_game: false };
    do_challenge(sh, addr, who, tc, color, pgn).await
}

async fn do_challenge(sh: Arc<Shared>, addr: SocketAddr, who: PlayerDto, tc: TimeControl, color: String, pgn: Option<String>) -> Result<(), String> {
    tc.validate()?;
    if !valid_color_pref(&color) {
        return Err("Bad colour choice.".into());
    }
    let stream = timeout(Duration::from_secs(4), TcpStream::connect(addr))
        .await
        .map_err(|_| "Connection timed out.".to_string())?
        .map_err(|e| format!("Couldn't connect: {e}"))?;
    let _ = stream.set_nodelay(true);
    let (rd, wr) = stream.into_split();
    let (tx, rx) = mpsc::unbounded_channel();
    let conn_id = sh.conn_seq.fetch_add(1, Ordering::SeqCst) + 1;
    {
        let mut g = sh.inner.lock().unwrap();
        let inner = &mut *g;
        if !matches!(inner.phase, Phase::Idle) {
            return Err("You're already in a game or a challenge.".into());
        }
        let _ = tx.send(Msg::Challenge { profile: sh.wire_profile(inner), v: PROTO_VERSION, tc, color: color.clone(), pgn: pgn.clone() });
        inner.pending = Some((tc, color));
        inner.pending_pgn = pgn;
        inner.peer_addr = Some(addr);
        inner.phase = Phase::Outgoing(who);
        inner.link = Some(Link { id: conn_id, tx });
    }
    sh.emit_state();
    tauri::async_runtime::spawn(run_connection(sh, rd, wr, rx, conn_id));
    Ok(())
}

// ---------- spectating ----------

/// Watch the game a visible peer is playing.
pub async fn spectate_peer(sh: Arc<Shared>, peer_id: String) -> Result<(), String> {
    can_challenge(&sh)?;
    let addr = {
        let g = sh.inner.lock().unwrap();
        let p = g.peers.get(&peer_id).ok_or_else(|| "That player is no longer visible.".to_string())?;
        SocketAddr::new(p.ip, p.port)
    };
    let profile = {
        let g = sh.inner.lock().unwrap();
        sh.wire_profile(&g)
    };
    let stream = timeout(Duration::from_secs(4), TcpStream::connect(addr))
        .await
        .map_err(|_| "Connection timed out.".to_string())?
        .map_err(|e| format!("Couldn't connect: {e}"))?;
    let _ = stream.set_nodelay(true);
    let (mut rd, mut wr) = stream.into_split();
    write_msg(&mut wr, &Msg::Spectate { profile, v: PROTO_VERSION }).await.map_err(|e| e.to_string())?;
    let reply = timeout(Duration::from_secs(4), read_msg(&mut rd))
        .await
        .map_err(|_| "No answer from that player.".to_string())?
        .map_err(|e| e.to_string())?;
    match reply {
        Msg::SpecWelcome { host, other, tc, start_fen } => {
            let host = player_from_wire(host);
            let other = player_from_wire(other);
            let (tx, rx) = mpsc::unbounded_channel();
            let conn_id = sh.conn_seq.fetch_add(1, Ordering::SeqCst) + 1;
            {
                let mut g = sh.inner.lock().unwrap();
                let inner = &mut *g;
                if !matches!(inner.phase, Phase::Idle) {
                    return Err("You're already in a game or a challenge.".into());
                }
                inner.session = Some(Session::new_spectator(host, other, tc, start_fen));
                inner.phase = Phase::Spectating;
                inner.link = Some(Link { id: conn_id, tx });
            }
            sh.emit_state();
            tauri::async_runtime::spawn(run_connection(sh, rd, wr, rx, conn_id));
            Ok(())
        }
        _ => Err("That game can't be spectated right now.".into()),
    }
}

/// Host side of one spectator connection.
async fn run_spec_conn(sh: Arc<Shared>, mut rd: OwnedReadHalf, mut wr: OwnedWriteHalf, mut rx: mpsc::UnboundedReceiver<Msg>, spec_id: u64) {
    let mut writer = tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                m = rx.recv() => match m {
                    Some(m) => {
                        let leave = matches!(m, Msg::Leave | Msg::Busy);
                        if write_msg(&mut wr, &m).await.is_err() || leave { break; }
                    }
                    None => break,
                },
                _ = tick.tick() => {
                    if write_msg(&mut wr, &Msg::Ping).await.is_err() { break; }
                }
            }
        }
        let _ = wr.shutdown().await;
    });

    let sh2 = sh.clone();
    let reader = async move {
        loop {
            match timeout(Duration::from_secs(15), read_msg(&mut rd)).await {
                Ok(Ok(Msg::GroupChat { text, .. })) => handle_spec_chat(&sh2, spec_id, text),
                Ok(Ok(Msg::Leave)) => break,
                Ok(Ok(_)) => {}
                _ => break,
            }
        }
    };
    tokio::pin!(reader);
    tokio::select! {
        _ = &mut writer => {}
        _ = &mut reader => {}
    }
    writer.abort();
    let mut g = sh.inner.lock().unwrap();
    g.spectators.retain(|sp| sp.id != spec_id);
}

/// A spectator said something: show it to us, the other player and the other spectators.
fn handle_spec_chat(sh: &Arc<Shared>, spec_id: u64, text: String) {
    let t = clean_text(&text, MAX_CHAT_CHARS);
    if t.is_empty() {
        return;
    }
    let from = {
        let g = sh.inner.lock().unwrap();
        let Some(name) = g.spectators.iter().find(|sp| sp.id == spec_id).map(|sp| sp.name.clone()) else { return };
        let msg = Msg::GroupChat { from: name.clone(), text: t.clone() };
        send(&g, msg.clone());
        fan_out(&g, &msg, Some(spec_id));
        name
    };
    sh.emit("chat", serde_json::json!({ "text": t, "from": from }));
}

// ---------- connection actor ----------

async fn run_connection(sh: Arc<Shared>, mut rd: OwnedReadHalf, mut wr: OwnedWriteHalf, mut rx: mpsc::UnboundedReceiver<Msg>, conn_id: u64) {
    let mut writer = tokio::spawn(async move {
        let mut tick = interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                m = rx.recv() => match m {
                    Some(m) => {
                        let leave = matches!(m, Msg::Leave | Msg::Decline | Msg::Busy);
                        if write_msg(&mut wr, &m).await.is_err() || leave { break; }
                    }
                    None => break,
                },
                _ = tick.tick() => {
                    if write_msg(&mut wr, &Msg::Ping).await.is_err() { break; }
                }
            }
        }
        let _ = wr.shutdown().await;
    });

    let sh2 = sh.clone();
    let reader = async move {
        loop {
            match timeout(Duration::from_secs(15), read_msg(&mut rd)).await {
                Ok(Ok(msg)) => {
                    if !handle_msg(&sh2, msg) {
                        break;
                    }
                }
                _ => break,
            }
        }
    };
    tokio::pin!(reader);
    tokio::select! {
        _ = &mut writer => {}
        _ = &mut reader => {}
    }
    writer.abort();
    on_link_closed(&sh, conn_id);
}

fn on_link_closed(sh: &Arc<Shared>, conn_id: u64) {
    let mut respawn: Option<(String, bool, Option<SocketAddr>)> = None;
    let ev: Option<&'static str> = {
        let mut g = sh.inner.lock().unwrap();
        let inner = &mut *g;
        match &inner.link {
            Some(l) if l.id == conn_id => {}
            _ => return,
        }
        inner.link = None;
        match inner.phase.tag() {
            "outgoing" => {
                inner.phase = Phase::Idle;
                Some("challenge-declined")
            }
            "incoming" => {
                inner.phase = Phase::Idle;
                Some("challenge-cancelled")
            }
            "spectating" => {
                inner.phase = Phase::Idle;
                inner.session = None;
                Some("spectate-ended")
            }
            "playing" => {
                let addr = inner.peer_addr;
                let mut ev = None;
                if let Some(s) = inner.session.as_mut() {
                    // A failed resume attempt closes a link while already disconnected: ignore.
                    if s.connected {
                        s.connected = false;
                        s.pause_clock();
                        ev = Some("peer-disconnected");
                        if !s.opp_left {
                            s.reconnect = Reconnect::Trying(0);
                            respawn = Some((s.sid.clone(), s.dialer, addr));
                        }
                    }
                }
                ev
            }
            _ => None,
        }
    };
    if let Some(e) = ev {
        sh.emit(e, ());
    }
    sh.emit_state();
    if let Some((sid, dialer, addr)) = respawn {
        match (dialer, addr) {
            (true, Some(a)) => {
                tauri::async_runtime::spawn(reconnect_dialer(sh.clone(), sid, a));
            }
            _ => {
                tauri::async_runtime::spawn(reconnect_listener(sh.clone(), sid));
            }
        }
    }
}

// ---------- auto-reconnect (10 polls, then give up) ----------

/// Sets the reconnect state if the session is still waiting for this reconnect. false => stop polling.
fn reconnect_set(sh: &Arc<Shared>, sid: &str, st: Reconnect) -> bool {
    let ok = {
        let mut g = sh.inner.lock().unwrap();
        match g.session.as_mut() {
            Some(s) if s.sid == sid && !s.connected && !s.opp_left => {
                s.reconnect = st;
                true
            }
            _ => false,
        }
    };
    if ok {
        sh.emit_state();
    }
    ok
}

fn reconnect_giveup(sh: &Arc<Shared>, sid: &str) {
    let gave_up = {
        let mut g = sh.inner.lock().unwrap();
        match g.session.as_mut() {
            Some(s) if s.sid == sid && !s.connected && !s.opp_left => {
                s.reconnect = Reconnect::GaveUp;
                true
            }
            _ => false,
        }
    };
    if gave_up {
        sh.emit("reconnect-failed", ());
        sh.emit_state();
    }
}

/// The side that originally dialled retries the TCP connection.
async fn reconnect_dialer(sh: Arc<Shared>, sid: String, addr: SocketAddr) {
    for attempt in 1..=RECONNECT_POLLS {
        if !reconnect_set(&sh, &sid, Reconnect::Trying(attempt)) {
            return;
        }
        if try_resume(&sh, &sid, addr).await {
            return;
        }
        sleep(Duration::from_secs(1)).await;
    }
    reconnect_giveup(&sh, &sid);
}

/// The accepting side waits for the dialler to come back, polling the same number of times.
async fn reconnect_listener(sh: Arc<Shared>, sid: String) {
    for attempt in 1..=RECONNECT_POLLS {
        if !reconnect_set(&sh, &sid, Reconnect::Trying(attempt)) {
            return;
        }
        sleep(Duration::from_secs(3)).await;
    }
    reconnect_giveup(&sh, &sid);
}

async fn try_resume(sh: &Arc<Shared>, sid: &str, addr: SocketAddr) -> bool {
    let stream = match timeout(Duration::from_secs(2), TcpStream::connect(addr)).await {
        Ok(Ok(s)) => s,
        _ => return false,
    };
    let _ = stream.set_nodelay(true);
    let (rd, wr) = stream.into_split();
    let (tx, rx) = mpsc::unbounded_channel();
    let conn_id = sh.conn_seq.fetch_add(1, Ordering::SeqCst) + 1;
    {
        let mut g = sh.inner.lock().unwrap();
        let inner = &mut *g;
        if inner.link.is_some() {
            return false;
        }
        let msg = match inner.session.as_ref() {
            Some(s) if s.sid == sid && !s.connected && !s.opp_left => Msg::Resume { sid: sid.to_string(), moves: s.uci_moves(), clock_ms: s.my_clock_report() },
            _ => return false,
        };
        let _ = tx.send(msg);
        inner.link = Some(Link { id: conn_id, tx });
    }
    tauri::async_runtime::spawn(run_connection(sh.clone(), rd, wr, rx, conn_id));
    for _ in 0..30 {
        sleep(Duration::from_millis(100)).await;
        let (connected, link_alive) = {
            let g = sh.inner.lock().unwrap();
            (
                g.session.as_ref().map(|s| s.sid == sid && s.connected).unwrap_or(false),
                g.link.as_ref().map(|l| l.id == conn_id).unwrap_or(false),
            )
        };
        if connected {
            sh.emit("peer-reconnected", ());
            return true;
        }
        if !link_alive {
            return false;
        }
    }
    let mut g = sh.inner.lock().unwrap();
    if g.link.as_ref().map(|l| l.id == conn_id).unwrap_or(false) {
        g.link = None;
    }
    false
}

// ---------- clock watchdog ----------

pub async fn run_clock_watch(sh: Arc<Shared>) {
    loop {
        sleep(Duration::from_millis(250)).await;
        let changed = {
            let mut g = sh.inner.lock().unwrap();
            let inner = &mut *g;
            let mut out: Option<Msg> = None;
            let mut changed = false;
            if let Some(s) = inner.session.as_mut() {
                if s.connected && s.clock.is_some() && s.end().is_none() {
                    if let Some(run) = s.running() {
                        let rem = s.remaining(run);
                        if run == s.my_color && rem <= 0 {
                            s.finish(GameEnd { kind: EndKind::Timeout, winner: Some(s.my_color.opp()) });
                            out = Some(Msg::Timeout);
                            changed = true;
                        } else if run != s.my_color && rem <= -2000 {
                            // Opponent's flag fell and they never reported it: claim after a short grace.
                            s.finish(GameEnd { kind: EndKind::Timeout, winner: Some(s.my_color) });
                            changed = true;
                        }
                    }
                }
            }
            if let Some(m) = out {
                send(inner, m);
            }
            changed
        };
        if changed {
            sh.emit_state();
        }
    }
}

/// Returns false when the connection should be closed.
fn handle_msg(sh: &Arc<Shared>, msg: Msg) -> bool {
    let mut g = sh.inner.lock().unwrap();
    let inner = &mut *g;
    match msg {
        Msg::Accept { your_color, profile, tc, sid } => {
            if inner.phase.tag() != "outgoing" || tc.validate().is_err() {
                return false;
            }
            let opp = player_from_wire(profile);
            let me = sh.me_dto(inner);
            let mut sess = Session::new(me, opp, your_color, 1, tc, clean_text(&sid, 32), true);
            if let Some(pgn) = inner.pending_pgn.take() {
                if let Ok(game) = Game::from_pgn(&pgn) {
                    sess.with_game(game);
                }
            }
            inner.session = Some(sess);
            inner.phase = Phase::Playing;
            drop(g);
            sh.emit_state();
        }
        Msg::Decline | Msg::Busy => {
            if inner.phase.tag() == "outgoing" {
                inner.phase = Phase::Idle;
                inner.link = None;
                drop(g);
                sh.emit("challenge-declined", ());
                sh.emit_state();
            }
            return false;
        }
        Msg::Move { uci, ply, clock_ms } => {
            let mut bad: Option<String> = None;
            match inner.session.as_mut() {
                Some(s) if s.end().is_none() && s.game.pos.side != s.my_color && ply as usize == s.game.moves.len() => {
                    let ply_before = s.game.moves.len();
                    let mover = s.game.pos.side;
                    match Move::parse_uci(&uci) {
                        Some(m) => match s.game.play(m) {
                            Ok(()) => {
                                s.draw_in = false;
                                s.draw_out = false;
                                s.after_move(mover, ply_before, clock_ms);
                            }
                            Err(e) => bad = Some(e),
                        },
                        None => bad = Some("malformed move".into()),
                    }
                }
                _ => bad = Some("unexpected move".into()),
            }
            drop(g);
            if let Some(e) = bad {
                if let Some(s) = sh.inner.lock().unwrap().session.as_mut() {
                    s.opp_left = true; // protocol violation: don't auto-reconnect
                }
                sh.emit("net-error", format!("Opponent sent an invalid move ({e}). Connection closed."));
                return false;
            }
            sh.emit_state();
        }
        Msg::Chat { text } => {
            let t = clean_text(&text, MAX_CHAT_CHARS);
            if inner.session.is_some() && !t.is_empty() {
                let from = inner.session.as_ref().map(|s| s.opp.name.clone()).unwrap_or_default();
                fan_out(inner, &Msg::GroupChat { from, text: t.clone() }, None);
                drop(g);
                sh.emit("chat", serde_json::json!({ "text": t, "from": serde_json::Value::Null }));
            }
        }
        Msg::DrawOffer => {
            if let Some(s) = inner.session.as_mut() {
                if s.end().is_none() {
                    s.draw_in = true;
                }
            }
            drop(g);
            sh.emit_state();
        }
        Msg::DrawAccept => {
            if let Some(s) = inner.session.as_mut() {
                if s.draw_out && s.end().is_none() {
                    s.finish(GameEnd { kind: EndKind::DrawAgreed, winner: None });
                }
                s.draw_out = false;
            }
            drop(g);
            sh.emit_state();
        }
        Msg::DrawDecline => {
            if let Some(s) = inner.session.as_mut() {
                s.draw_out = false;
            }
            drop(g);
            sh.emit("draw-declined", ());
            sh.emit_state();
        }
        Msg::Resign => {
            if let Some(s) = inner.session.as_mut() {
                if s.end().is_none() {
                    s.finish(GameEnd { kind: EndKind::Resigned, winner: Some(s.my_color) });
                }
            }
            drop(g);
            sh.emit_state();
        }
        Msg::RematchReq => {
            if let Some(s) = inner.session.as_mut() {
                if s.end().is_some() {
                    s.rematch_in = true;
                }
            }
            drop(g);
            sh.emit_state();
        }
        Msg::RematchAccept => {
            if let Some(s) = inner.session.as_mut() {
                if s.rematch_out {
                    s.restart_swapped();
                }
            }
            drop(g);
            sh.emit_state();
        }
        Msg::RematchDecline => {
            if let Some(s) = inner.session.as_mut() {
                s.rematch_out = false;
            }
            drop(g);
            sh.emit("rematch-declined", ());
            sh.emit_state();
        }
        Msg::Timeout => {
            if let Some(s) = inner.session.as_mut() {
                if s.end().is_none() {
                    s.finish(GameEnd { kind: EndKind::Timeout, winner: Some(s.my_color) });
                }
            }
            drop(g);
            sh.emit_state();
        }
        Msg::ResumeOk { moves, clock_ms } => {
            let mut bad = false;
            if let Some(s) = inner.session.as_mut() {
                match s.apply_resume(&moves, clock_ms) {
                    Ok(()) => {
                        s.connected = true;
                        s.reconnect = Reconnect::None;
                        s.resume_clock();
                    }
                    Err(_) => {
                        s.opp_left = true;
                        bad = true;
                    }
                }
            }
            drop(g);
            if bad {
                sh.emit("net-error", "Couldn't resume: the two game histories don't match.".to_string());
                return false;
            }
            sh.emit_state();
        }
        Msg::Leave => {
            if let Some(s) = inner.session.as_mut() {
                s.opp_left = true;
            }
            return false;
        }
        Msg::GroupChat { from, text } => {
            let t = clean_text(&text, MAX_CHAT_CHARS);
            let f = peer_name(&from);
            if inner.session.is_some() && !t.is_empty() {
                if !matches!(inner.phase, Phase::Spectating) {
                    // relay to our own spectators (never back over the player link)
                    fan_out(inner, &Msg::GroupChat { from: f.clone(), text: t.clone() }, None);
                }
                drop(g);
                sh.emit("chat", serde_json::json!({ "text": t, "from": f }));
            }
        }
        Msg::SpecSync { data } => {
            let mut ok = false;
            if matches!(inner.phase, Phase::Spectating) {
                if let Some(s) = inner.session.as_mut() {
                    ok = s.apply_spec_sync(&data).is_ok();
                }
            }
            drop(g);
            if ok {
                sh.emit_state();
            }
        }
        Msg::SpecJoined { name } => {
            let n = peer_name(&name);
            if inner.session.is_some() {
                drop(g);
                sh.emit("spectator-joined", n);
            }
        }
        Msg::Resume { .. } | Msg::Ping | Msg::Profile { .. } | Msg::ProfileReq | Msg::Challenge { .. } | Msg::Spectate { .. } | Msg::SpecWelcome { .. } => {}
    }
    true
}
