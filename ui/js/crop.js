// Square (1:1) image cropper: drag to pan, wheel / slider to zoom. Pure canvas, no dependencies.
import { h, showModal, closeModal, btn } from './ui.js';

const OUT = 256;   // saved avatar size (PNG)
const THUMB = 96;  // network thumbnail size (JPEG)

export function openCropper(file) {
  return new Promise((resolve) => {
    const url = URL.createObjectURL(file);
    const img = new Image();
    img.onerror = () => { URL.revokeObjectURL(url); resolve(null); };
    img.onload = () => {
      const V = Math.min(320, window.innerWidth - 72);
      const canvas = h('canvas', { width: V, height: V });
      const ctx = canvas.getContext('2d');
      const minS = V / Math.min(img.naturalWidth, img.naturalHeight);
      let s = minS, x = (V - img.naturalWidth * s) / 2, y = (V - img.naturalHeight * s) / 2;

      const clamp = () => {
        x = Math.min(0, Math.max(V - img.naturalWidth * s, x));
        y = Math.min(0, Math.max(V - img.naturalHeight * s, y));
      };
      const draw = () => {
        clamp();
        ctx.clearRect(0, 0, V, V);
        ctx.imageSmoothingQuality = 'high';
        ctx.drawImage(img, x, y, img.naturalWidth * s, img.naturalHeight * s);
      };
      const zoomTo = (ns) => {
        ns = Math.min(minS * 5, Math.max(minS, ns));
        const cx = (V / 2 - x) / s, cy = (V / 2 - y) / s; // keep centre fixed
        s = ns; x = V / 2 - cx * s; y = V / 2 - cy * s; draw();
      };

      let drag = null;
      const box = h('div', { class: 'crop-box', style: `width:${V}px;height:${V}px` }, canvas);
      box.addEventListener('pointerdown', (e) => { box.setPointerCapture(e.pointerId); drag = { px: e.clientX, py: e.clientY, x, y }; });
      box.addEventListener('pointermove', (e) => { if (!drag) return; x = drag.x + e.clientX - drag.px; y = drag.y + e.clientY - drag.py; draw(); });
      const end = () => { drag = null; };
      box.addEventListener('pointerup', end);
      box.addEventListener('pointercancel', end);
      box.addEventListener('wheel', (e) => { e.preventDefault(); zoomTo(s * (e.deltaY < 0 ? 1.08 : 1 / 1.08)); slider.value = s; }, { passive: false });

      const slider = h('input', { type: 'range', min: minS, max: minS * 5, step: minS / 100, value: minS, 'aria-label': 'Zoom' });
      slider.addEventListener('input', () => zoomTo(parseFloat(slider.value)));

      const finish = (result) => { URL.revokeObjectURL(url); closeModal('crop'); resolve(result); };
      const confirm = () => {
        const sx = -x / s, sw = V / s;
        const out = document.createElement('canvas');
        out.width = out.height = OUT;
        const o = out.getContext('2d');
        o.imageSmoothingQuality = 'high';
        o.drawImage(img, sx, -y / s, sw, sw, 0, 0, OUT, OUT);
        const th = document.createElement('canvas');
        th.width = th.height = THUMB;
        th.getContext('2d').drawImage(out, 0, 0, THUMB, THUMB);
        const b64 = (u) => u.slice(u.indexOf(',') + 1);
        finish({
          png: b64(out.toDataURL('image/png')),
          thumb: b64(th.toDataURL('image/jpeg', 0.8)),
          dataUrl: out.toDataURL('image/png'),
        });
      };

      showModal('crop', () => [
        h('h2', { text: 'Crop picture' }),
        h('p', { class: 'sub', text: 'Square 1:1 — drag to move, slider to zoom.' }),
        h('div', { class: 'crop-wrap' }, box, slider),
        h('div', { class: 'btn-grid' }, btn('Cancel', () => finish(null)), btn('Use picture', confirm, 'btn primary')),
      ]);
      draw();
    };
    img.src = url;
  });
}
