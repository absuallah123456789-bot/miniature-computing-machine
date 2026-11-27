// Small DOM helpers. Everything user-controlled is inserted as text nodes (never innerHTML).
export function h(tag, props = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props || {})) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k === 'text') el.textContent = v;
    else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2), v);
    else if (v === true) el.setAttribute(k, '');
    else el.setAttribute(k, String(v));
  }
  for (const kid of kids.flat()) {
    if (kid == null || kid === false) continue;
    el.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
  }
  return el;
}

// Only accept image data URLs produced by our backend (png/jpeg, base64 alphabet).
const DATA_URL = /^data:image\/(png|jpeg);base64,[A-Za-z0-9+/=]+$/;
export const safeImg = (u) => (typeof u === 'string' && DATA_URL.test(u) ? u : null);

export function setAvatar(el, player) {
  const src = safeImg(player && player.avatar);
  el.textContent = '';
  if (src) {
    el.style.backgroundImage = `url("${src}")`;
  } else {
    el.style.backgroundImage = '';
    el.textContent = ((player && player.name) || '?').trim().slice(0, 1).toUpperCase();
  }
}

let toastTimer = 0;
export function toast(msg) {
  const t = document.getElementById('toast');
  t.textContent = String(msg);
  t.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove('show'), 3200);
}

// ---- modal manager (idempotent per key) ----
const root = document.getElementById('modal-root');
const open = new Map();

export function isOpen(key) { return open.has(key); }

export function showModal(key, build, { cardClass = '', onClose = null, backdropClose = false } = {}) {
  const existing = open.get(key);
  if (existing) { existing.build = build; existing.render(); return existing; }
  const card = h('div', { class: ('modal ' + cardClass).trim(), role: 'dialog', 'aria-modal': 'true' });
  const back = h('div', { class: 'backdrop' }, card);
  const m = {
    key, card, back, build, onClose,
    render() { card.replaceChildren(...[].concat(this.build()).filter(Boolean)); },
    close() { closeModal(key); },
  };
  if (backdropClose) back.addEventListener('pointerdown', (e) => { if (e.target === back) m.close(); });
  root.append(back);
  open.set(key, m);
  m.render();
  return m;
}

export function closeModal(key) {
  const m = open.get(key);
  if (!m) return;
  open.delete(key);
  m.back.remove();
  if (m.onClose) m.onClose();
}

export function closeAllExcept(keys = []) {
  for (const k of [...open.keys()]) if (!keys.includes(k)) closeModal(k);
}

export const btn = (label, onclick, cls = 'btn', extra = {}) => h('button', { class: cls, type: 'button', onclick, ...extra }, label);
