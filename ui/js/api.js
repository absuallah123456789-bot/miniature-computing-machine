// Thin wrapper over the Tauri IPC bridge (window.__TAURI__ via withGlobalTauri).
const T = window.__TAURI__;
const invoke = (cmd, args) => T.core.invoke(cmd, args);

export const api = {
  getProfile: () => invoke('get_profile'),
  saveProfile: (name, avatarPng, avatarThumb) => invoke('save_profile', { name, avatarPng, avatarThumb }),
  listPeers: () => invoke('list_peers'),
  challenge: (peerId, settings) => invoke('challenge', { peerId, settings }),
  spectate: (peerId) => invoke('spectate', { peerId }),
  challengeIp: (addr, settings) => invoke('challenge_ip', { addr, settings }),
  respondChallenge: (accept) => invoke('respond_challenge', { accept }),
  cancelChallenge: () => invoke('cancel_challenge'),
  getPhase: () => invoke('get_phase'),
  getSession: () => invoke('get_session'),
  makeMove: (uci) => invoke('make_move', { uci }),
  resign: () => invoke('resign'),
  offerDraw: () => invoke('offer_draw'),
  respondDraw: (accept) => invoke('respond_draw', { accept }),
  sendChat: (text) => invoke('send_chat', { text }),
  requestRematch: () => invoke('request_rematch'),
  respondRematch: (accept) => invoke('respond_rematch', { accept }),
  leaveGame: () => invoke('leave_game'),
  getPgn: () => invoke('get_pgn'),
  getReviewInput: () => invoke('get_review_input'),
};

export const listen = (ev, fn) => T.event.listen(ev, (e) => fn(e.payload));
