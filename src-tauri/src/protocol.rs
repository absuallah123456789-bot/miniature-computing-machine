//! Wire protocol. Transport: TCP, each frame = u32 big-endian length + UTF-8 JSON body.
//! Discovery: UDP broadcast of a `Beacon` JSON datagram.
use cheslan_core::types::Color;
use serde::{Deserialize, Serialize};

pub const PROTO_VERSION: u32 = 3;
pub const DISCOVERY_PORT: u16 = 47474;
pub const GAME_PORT: u16 = 47475;
pub const MAX_FRAME: usize = 256 * 1024;
pub const MAX_CHAT_CHARS: usize = 500;
pub const MAX_NAME_CHARS: usize = 24;
pub const MAX_THUMB_BYTES: usize = 64 * 1024;
/// Auto-reconnect: poll this many times, then give up.
pub const RECONNECT_POLLS: u8 = 10;

/// `initial_ms == 0` means "no timer".
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TimeControl {
    pub initial_ms: u64,
    pub bonus_ms: u64,
}

impl TimeControl {
    pub fn enabled(&self) -> bool {
        self.initial_ms > 0
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.initial_ms != 0 && !(60_000..=7_200_000).contains(&self.initial_ms) {
            return Err("Initial time must be between 1 and 120 minutes.".into());
        }
        if self.bonus_ms > 60_000 {
            return Err("Bonus must be at most 60 seconds.".into());
        }
        Ok(())
    }
}

pub fn valid_color_pref(c: &str) -> bool {
    matches!(c, "white" | "black" | "random")
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Beacon {
    pub app: String,
    pub v: u32,
    pub id: String,
    pub name: String,
    pub os: String,
    pub port: u16,
    pub avatar_hash: String,
    /// True while this player is in a game (so others can offer to spectate).
    #[serde(default)]
    pub in_game: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WireProfile {
    pub id: String,
    pub name: String,
    pub os: String,
    /// base64 JPEG thumbnail (<= 64 KiB)
    pub avatar: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Msg {
    ProfileReq,
    Profile { profile: WireProfile },
    /// `color` = the challenger's preference: "white" | "black" | "random".
    /// `pgn` = optional PGN to start the game from (e.g. to continue after a connection loss).
    Challenge { profile: WireProfile, v: u32, tc: TimeControl, color: String, #[serde(default)] pgn: Option<String> },
    /// `your_color` is the colour assigned to the challenger; `sid` identifies the session for resuming.
    Accept { your_color: Color, profile: WireProfile, tc: TimeControl, sid: String },
    Decline,
    Busy,
    /// `ply` = number of moves already played before this one (desync guard).
    /// `clock_ms` = the mover's remaining time after the move (increment included).
    Move {
        uci: String,
        ply: u32,
        #[serde(default)]
        clock_ms: Option<i64>,
    },
    /// Sender's clock ran out.
    Timeout,
    /// Auto-reconnect handshake: full move list + sender's own remaining clock.
    Resume { sid: String, moves: Vec<String>, clock_ms: Option<i64> },
    ResumeOk { moves: Vec<String>, clock_ms: Option<i64> },
    Chat { text: String },
    DrawOffer,
    DrawAccept,
    DrawDecline,
    Resign,
    RematchReq,
    RematchAccept,
    RematchDecline,
    Ping,
    Leave,
    /// Spectator -> player: "let me watch your game".
    Spectate { profile: WireProfile, v: u32 },
    /// Player -> spectator: both players' profiles (`host` = the player the spectator connected to).
    SpecWelcome { host: WireProfile, other: WireProfile, tc: TimeControl, #[serde(default)] start_fen: Option<String> },
    /// Player -> spectator: full mirror of the game (sent after every change).
    SpecSync { data: SpecSyncData },
    /// Player -> other player: a spectator joined (for the toast).
    SpecJoined { name: String },
    /// Group chat line. Spectator -> host: `from` is ignored (host fills in the real name).
    GroupChat { from: String, text: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SpecSyncData {
    pub moves: Vec<String>,
    /// Colour the host (the player the spectator is connected to) currently plays.
    pub host_color: Color,
    pub white_ms: i64,
    pub black_ms: i64,
    /// Whose clock is running, if any.
    pub running: Option<Color>,
    /// "resigned" | "draw_agreed" | "timeout" when the game was ended off-board.
    pub forced_kind: Option<String>,
    pub forced_winner: Option<Color>,
    pub game_no: u32,
}
