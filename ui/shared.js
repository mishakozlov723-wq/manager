'use strict';
// Shared by the manager's window and its installer: talking to the exe, small helpers.
(() => {
  const T = window.__TAURI__;
  const invoke = (cmd, args) => T ? T.core.invoke(cmd, args) : Promise.reject(new Error('no tauri'));
  const listen = (ev, fn) => T ? T.event.listen(ev, fn) : Promise.resolve(() => {});
  const log = m => { try { invoke('log', { msg: String(m) }).catch(() => {}); } catch (e) {} };
  const esc = s => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  const $ = s => document.querySelector(s);

  let toastT = 0;
  function toast(text, bad) {
    const t = $('#toast'); if (!t) return;
    t.textContent = text; t.classList.toggle('bad', !!bad); t.classList.add('on');
    clearTimeout(toastT); toastT = setTimeout(() => t.classList.remove('on'), bad ? 6000 : 2600);
  }

  // no reloading, zooming or the browser menu in these windows
  addEventListener('keydown', e => { if ((e.ctrlKey && /^[rpfgjsu]$/i.test(e.key)) || /^F(5|7)$/.test(e.key)) e.preventDefault(); }, true);
  addEventListener('wheel', e => { if (e.ctrlKey) e.preventDefault(); }, { passive: false });
  addEventListener('contextmenu', e => { if (!e.target.closest('input,.notes')) e.preventDefault(); });

  window.RM = { invoke, listen, log, esc, sleep, $, toast, tauri: !!T };
})();
