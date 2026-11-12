'use strict';
// The manager's window: one card per program — what is installed, what is out, and the buttons to install,
// update, open and remove. The exe does the work (src-tauri/src/apps.rs); progress comes as «job» events.
// In a plain browser (no Tauri) it runs on made-up data, so the look can be checked anywhere:
// index.html?fresh · ?all · ?offline · ?nofx · ?self · ?fail · ?busy=snap · ?ask=voice · ?notes=snap
(() => {
const RM = window.RM, { esc, sleep, $, toast } = RM;
const Q = new URLSearchParams(location.search);

const APPS = [
  { id: 'voice', name: 'Retocker Voice', icon: 'icons/voice.svg',
    desc: 'Диктовка в любую программу: говоришь — текст появляется там, где курсор',
    del: 'Уберу программу, ярлыки и автозапуск.',
    wipe: ['Удалить настройки и ключ Groq', 'Если оставить — после новой установки всё будет как сейчас'] },
  { id: 'snap', name: 'Retocker Snap', icon: 'icons/snap.svg',
    desc: 'Быстрые красивые скриншоты: выделил — скопировал, сохранил или достал текст',
    del: 'Уберу программу, ярлыки и автозапуск.',
    wipe: ['Удалить настройки, историю и ключ Groq', 'Если оставить — после новой установки всё будет как сейчас'] },
  { id: 'zapret', name: 'Zapret Manager', icon: 'icons/zapret.svg',
    desc: 'Менеджер zapret для YouTube и Discord: версии, стратегии и служба в одном окне',
    del: 'Уберу программу и ярлыки. Если включена служба обхода — остановлю и уберу её тоже: Windows спросит разрешение.',
    wipe: ['Удалить настройки и скачанные версии zapret', 'Если оставить — после новой установки всё будет как сейчас'] },
  { id: 'translate', name: 'Retocker Translate', icon: 'icons/translate.png', ext: true,
    desc: 'Нейросетевой перевод выделенного текста и целых страниц с сохранением оформления' },
];
const byId = id => APPS.find(a => a.id === id);
const TAG = '<span class="tag" title="Это не программа, а расширение для браузера Firefox: ставится и удаляется в самом Firefox"><svg viewBox="0 0 16 16"><path d="M6.3 2.6h3.4v1.9a1.2 1.2 0 1 0 2.4 0h1.3v3.6h-1.3a1.4 1.4 0 1 0 0 2.8h1.3v2.5H9.9v-1.2a1.5 1.5 0 1 0-3 0v1.2H2.6V9.9h1.2a1.5 1.5 0 1 0 0-3H2.6V4.5h3.7z"/></svg>Расширение Firefox</span>';
const IC = {
  open: '<svg viewBox="0 0 16 16"><path d="M6.5 3.5H3.5v9h9v-3M9.5 3h3.5v3.5M13 3 7.5 8.5"/></svg>',
  del: '<svg viewBox="0 0 16 16"><path d="M3 4.5h10M6.3 4.5V3h3.4v1.5M4.4 4.5l.6 8.5h6l.6-8.5M6.8 7v3.6M9.2 7v3.6"/></svg>',
};

/* ---------------- talking to the exe (or to made-up data in a browser) */
const LS = {};
const fire = (ev, payload) => (LS[ev] || []).forEach(fn => fn({ payload }));
const mb = n => (n / 1048576).toFixed(1).replace('.', ',');
const M = {
  local: Q.has('fresh') ? {} : Q.has('all')
    ? { voice: '1.0.61', snap: '0.4.23', zapret: '0.3.7', translate: '1.0.3' }
    : { voice: '1.0.61', snap: '0.4.21' },
  remote: {
    voice: { version: '1.0.61', size: '7,2 МБ', bytes: 7514624, notes: 'Исправили «Запускать в трее»: после первого нажатия горячей клавиши экран переставал реагировать на клики, пока мышь не пройдёт по капсуле.' },
    snap: { version: '0.4.23', size: '12,0 МБ', bytes: 12561408, notes: 'Затемнение больше не мигает на первом снимке. Сразу после запуска программы первый снимок открывался с мерцающим затемнением. Теперь оно появляется плавно с первого раза, на всех экранах.' },
    zapret: { version: '0.3.7', size: '71,9 МБ', bytes: 75398356, notes: '• «Подобрать за меня». Менеджер сам перебирает стратегии и оставляет первую, с которой открываются YouTube и Discord.\n• Игровой режим: все порты, только TCP или только UDP.\n• Свой список IP и наборы сайтов.' },
    translate: { version: '1.0.3', size: '', bytes: 0, notes: '' },
  },
  cancel: {},
};
const mlocal = id => ({ id, installed: !!M.local[id], version: M.local[id] || '', firefox: !Q.has('nofx') });
async function mjob(id, steps) {
  for (const [stage, ms, text] of steps) {
    if (stage === 'down') {
      const total = M.remote[id].bytes, n = Q.has('busy') ? 400 : 22;
      for (let i = 0; i <= n; i++) { fire('job', { id, stage, got: Math.round(total * i / n), total }); await sleep(Q.has('busy') ? 300 : 70); }
    } else if (stage === 'wait') {
      fire('job', { id, stage, text });
      for (let i = 0; i < ms / 100; i++) { if (M.cancel[id]) { M.cancel[id] = false; throw ''; } await sleep(100); }
    } else { fire('job', { id, stage, text }); await sleep(ms); }
  }
}
const MOCK = {
  app_info: () => ({ version: '1.0.3', updated: Q.has('updated') ? '1.0.3' : '', installed: true }),
  apps_local: () => APPS.map(a => mlocal(a.id)),
  app_remote: async ({ id }) => { await sleep(350 + 260 * APPS.findIndex(a => a.id === id)); if (Q.has('offline')) throw 'нет связи с сервером'; return { id, ...M.remote[id] }; },
  app_install: async ({ id }) => {
    if (id === 'translate') { if (Q.has('nofx')) throw 'Firefox не найден на этом компьютере. Установи Firefox и попробуй ещё раз'; await mjob(id, [['wait', 3500, 'Открыл Firefox — нажми там «Добавить»']]); }
    else if (id === 'zapret') await mjob(id, [['down'], ['check', 400], ['wait', 4000, 'Открыл установщик — заверши установку в его окне']]);
    else await mjob(id, [['down'], ['check', 400], ['install', 1500]]);
    if (Q.has('fail')) throw 'файл скачался с ошибкой (контрольная сумма не совпала)';
    M.local[id] = M.remote[id].version; return mlocal(id);
  },
  app_uninstall: async ({ id }) => {
    if (id === 'translate') await mjob(id, [['wait', 3500, 'Открыл дополнения Firefox — удали там Retocker Translate']]);
    else if (id === 'zapret') await mjob(id, [['close', 400], ['admin', 1400, 'Убираю службу обхода — Windows спросит разрешение'], ['remove', 1200]]);
    else await mjob(id, [['remove', 1600]]);
    delete M.local[id]; return mlocal(id);
  },
  app_cancel: ({ id }) => { M.cancel[id] = true; },
  app_open: () => {},
  upd_check: async () => ({ version: '1.0.9', notes: 'Карточки программ открываются быстрее.', size: '6,4 МБ', newer: Q.has('self'), can: true }),
  upd_install: async () => { for (let i = 0; i <= 20; i++) { fire('upd', { stage: 'down', p: i / 20 }); await sleep(90); } fire('upd', { stage: 'check' }); await sleep(500); fire('upd', { stage: 'restart' }); },
  win_ctl: () => {}, open_logs: () => {},
};
const call = RM.tauri ? RM.invoke : async (cmd, args) => MOCK[cmd](args || {});
const on = RM.tauri ? RM.listen : (ev, fn) => { (LS[ev] || (LS[ev] = [])).push(fn); };

/* ---------------- state */
const S = { local: {}, remote: {}, job: {} };
let lastCheck = 0;
const newer = (a, b) => {
  const p = s => String(s || '').trim().replace(/^v/, '').split('.').map(x => parseInt(x, 10) || 0);
  const x = p(a), y = p(b);
  for (let i = 0; i < 3; i++) if ((x[i] || 0) !== (y[i] || 0)) return (x[i] || 0) > (y[i] || 0);
  return false;
};
const pl = (n, a, b, c) => { const m = n % 10, h = n % 100; return m === 1 && h !== 11 ? a : m >= 2 && m <= 4 && (h < 12 || h > 14) ? b : c; };
const hasUpdate = a => { const l = S.local[a.id], r = S.remote[a.id]; return !!(l && l.installed && r && r.version && newer(r.version, l.version)); };

const JOB = { start: 'Готовлюсь…', check: 'Проверяю файл…', close: 'Закрываю программу…', install: 'Устанавливаю…', remove: 'Удаляю…' };
function jobText(j) {
  if (j.stage === 'down') return 'Скачиваю… ' + (j.total ? `${mb(j.got)} из ${mb(j.total)} МБ` : j.got ? `${mb(j.got)} МБ` : '');
  return j.text || JOB[j.stage] || '';
}

/* ---------------- the cards */
const ACT = {
  install: () => '<button class="btn pri" data-a="go">Установить</button>',
  update: a => '<button class="btn pri" data-a="go">Обновить</button>' + ACT.ok(a),
  ok: a => (a.ext ? '' : '<button class="btn" data-a="open">Открыть</button>') + `<button class="btn ib del" data-a="del" title="Удалить">${IC.del}</button>`,
  cancel: () => '<button class="btn" data-a="cancel">Отмена</button>',
  pct: () => '<span class="pct"></span>',
};

function view(a) {
  const l = S.local[a.id] || {}, r = S.remote[a.id] || {}, j = S.job[a.id];
  const size = r.size ? ' · ' + r.size : '';
  const v = { notes: '', bar: 0, ind: false, pct: '' };
  const ver = x => `<span class="ver">${esc(x)}</span>`;
  if (l.installed) v.vers = hasUpdate(a) ? `${ver(l.version)}<span class="arr">→</span><span class="ver new">${esc(r.version)}</span>` : ver(l.version);
  else v.vers = r.version ? ver(r.version) : r.loading ? '<span class="ver sk">0.0.00</span>' : '';
  if (j) {
    v.s = 'busy'; v.text = jobText(j); v.act = j.stage === 'wait' ? 'cancel' : 'pct';
    if (j.stage === 'down') { const p = j.total ? j.got / j.total : 0; v.bar = 3 + p * 77; v.pct = j.total ? Math.round(p * 100) + '%' : ''; }
    else if (j.stage === 'check') v.bar = 86;
    else v.ind = true;
  } else if (!l.installed) {
    const nofx = a.ext && l.firefox === false;
    v.s = nofx ? 'warn' : 'none'; v.act = 'install';
    v.text = nofx ? 'Нужен Firefox — на этом компьютере его нет' : 'Не установлено' + size;
    v.notes = r.notes || '';
  } else if (hasUpdate(a)) {
    v.s = 'upd'; v.act = 'update'; v.text = 'Есть обновление' + size; v.notes = r.notes || '';
  } else {
    v.s = 'ok'; v.act = 'ok';
    v.text = 'Установлено · ' + (r.version ? 'последняя версия' : r.loading ? 'проверяю обновления…' : r.error ? 'не получилось проверить обновления' : '…');
  }
  return v;
}

const card = id => document.querySelector(`.app[data-id="${id}"]`);
function paint(a) {
  const el = card(a.id), v = view(a), q = s => el.querySelector(s);
  el.dataset.s = v.s;
  const vs = q('.vs'); if (vs._h !== v.vers) { vs._h = v.vers; vs.innerHTML = v.vers; }
  const st = q('.stt');
  if (st.textContent !== v.text) {
    st.textContent = v.text;
    // a new line slides in; the megabytes of a download just tick
    if (st._s !== v.s + (S.job[a.id] ? S.job[a.id].stage : '')) { st._s = v.s + (S.job[a.id] ? S.job[a.id].stage : ''); st.classList.remove('in'); void st.offsetWidth; st.classList.add('in'); }
  }
  const act = q('.act');
  if (act.dataset.k !== v.act) { act.dataset.k = v.act; act.innerHTML = ACT[v.act](a); act.classList.remove('in'); void act.offsetWidth; act.classList.add('in'); }
  const pct = q('.pct'); if (pct) pct.textContent = v.pct;
  q('[data-a=notes]').hidden = !v.notes;
  q('.notes p').textContent = v.notes;
  if (!v.notes) el.classList.remove('open');
  q('[data-a=notes]').textContent = el.classList.contains('open') ? 'Скрыть' : 'Что нового';
  const bar = q('.pbar'); bar.classList.toggle('ind', v.ind); bar.firstChild.style.width = v.ind ? '' : v.bar + '%';
}

function paintSum() {
  const n = APPS.filter(a => (S.local[a.id] || {}).installed).length, u = APPS.filter(hasUpdate).length;
  const rs = APPS.map(a => S.remote[a.id] || {});
  const tail = u ? `${u} ${pl(u, 'обновление', 'обновления', 'обновлений')}` : rs.some(r => r.loading) ? 'проверяю обновления…' : rs.every(r => r.error) ? 'нет связи с сервером' : n ? 'всё свежее' : '';
  $('#sum').textContent = `Установлено ${n} из ${APPS.length}` + (tail ? ' · ' + tail : '');
  $('#bAll').hidden = u < 2;
}
const paintAll = () => { APPS.forEach(paint); paintSum(); };

function build() {
  $('#list').innerHTML = APPS.map((a, i) => `<article class="app" data-id="${a.id}" data-s="none" style="--i:${i}">
    <div class="ic"><img src="${a.icon}" alt=""></div>
    <div class="info">
      <div class="nm"><b>${a.name}</b>${a.ext ? TAG : ''}<span class="vs"></span></div>
      <p class="ds">${a.desc}</p>
      <div class="st"><i></i><span class="stt"></span><button class="lnk" data-a="notes" hidden>Что нового</button></div>
    </div>
    <div class="act"></div>
    <div class="notes"><div><p></p></div></div>
    <div class="pbar"><b></b></div>
  </article>`).join('');
  $('#list').addEventListener('click', e => {
    const b = e.target.closest('[data-a]'); if (!b) return;
    const a = byId(b.closest('.app').dataset.id);
    ({
      go: () => run(a, 'install'),
      del: () => ask(a),
      open: () => call('app_open', { id: a.id }).catch(err => toast(`${a.name}: ${err}`, true)),
      cancel: () => call('app_cancel', { id: a.id }),
      notes: () => { card(a.id).classList.toggle('open'); paint(a); },
    })[b.dataset.a]();
  });
}

/* ---------------- work */
async function refreshLocal() {
  try { (await call('apps_local')).forEach(l => { S.local[l.id] = l; }); } catch (e) { RM.log('apps_local: ' + e); }
  paintAll();
}

async function run(a, kind, wipe) {
  if (S.job[a.id]) return;
  S.job[a.id] = { stage: 'start' }; paint(a);
  try {
    S.local[a.id] = await (kind === 'install' ? call('app_install', { id: a.id }) : call('app_uninstall', { id: a.id, wipe: !!wipe }));
    delete S.job[a.id];
    if (kind === 'install') { const el = card(a.id); el.classList.remove('flash'); void el.offsetWidth; el.classList.add('flash'); }
    else toast(`${a.name} — удалено`);
  } catch (e) {
    delete S.job[a.id];
    const msg = String(e && e.message || e || '');
    // nothing to say: the user closed the setup window or pressed «Отмена»
    if (msg) toast(`${a.name}: ${msg}`, true);
    await refreshLocal();
  }
  paintAll();
}

async function check(manual) {
  if ($('#bCheck').classList.contains('spin')) return;
  $('#bCheck').classList.add('spin');
  APPS.forEach(a => { S.remote[a.id] = { ...(S.remote[a.id] || {}), loading: true, error: '' }; });
  paintAll();
  await Promise.all([refreshLocal(), selfCheck(), ...APPS.map(async a => {
    try { S.remote[a.id] = await call('app_remote', { id: a.id }); }
    catch (e) { S.remote[a.id] = { ...S.remote[a.id], loading: false, error: String(e) }; }
    paint(a); paintSum();
  })]);
  $('#bCheck').classList.remove('spin'); lastCheck = Date.now();
  if (manual) {
    const u = APPS.filter(hasUpdate).length;
    if (APPS.every(a => S.remote[a.id].error)) toast('Нет связи с сервером — не получилось проверить', true);
    else toast(u ? `Нашёл ${u} ${pl(u, 'обновление', 'обновления', 'обновлений')}` : 'Всё свежее');
  }
}

async function updateAll() {
  for (const a of APPS.filter(hasUpdate)) await run(a, 'install');
}

/* ---------------- the question before removing */
function ask(a) {
  const hd = `<div class="hd"><div class="ic"><img src="${a.icon}" alt=""></div><h2>Удалить ${a.name}?</h2></div>`;
  $('#sheet').innerHTML = a.ext
    ? `${hd}<p class="lead">Расширения удаляет сам Firefox. Открою страницу дополнений — найди там Retocker Translate, нажми «⋯» и выбери «Удалить».</p>
       <div class="foot"><button class="btn big" data-x="no">Отмена</button><button class="btn pri big" data-x="yes">Открыть Firefox</button></div>`
    : `${hd}<p class="lead">${a.del}</p>
       <div class="opts"><label class="row"><div class="lbl"><span>${a.wipe[0]}</span><small>${a.wipe[1]}</small></div><input type="checkbox" class="sw" id="shWipe"></label></div>
       <div class="foot"><button class="btn big" data-x="no">Отмена</button><button class="btn red big" data-x="yes">Удалить</button></div>`;
  $('#sheet').onclick = e => {
    const b = e.target.closest('[data-x]'); if (!b) return;
    const wipe = !!($('#shWipe') && $('#shWipe').checked);
    closeSheet();
    if (b.dataset.x === 'yes') run(a, 'uninstall', wipe);
  };
  $('#veil').classList.add('on');
}
const closeSheet = () => $('#veil').classList.remove('on');

/* ---------------- the manager's own update */
async function selfCheck() {
  try {
    const u = await call('upd_check');
    if (!(u.newer && u.can)) return;
    $('#selfT').textContent = `Вышел Retocker Manager ${u.version}`;
    $('#selfN').textContent = u.notes || 'Обновится сам и откроется заново';
    $('#self').classList.add('on');
  } catch (e) { /* no connection: the cards say so */ }
}
function selfEvent(e) {
  const p = e.payload, bar = $('#selfBar');
  if (p.stage === 'error') { toast('Менеджер не обновился: ' + p.msg, true); $('#selfGo').disabled = false; bar.hidden = true; return; }
  bar.hidden = false;
  if (p.stage === 'down') { bar.firstChild.style.width = Math.round((p.p || 0) * 100) + '%'; $('#selfN').textContent = `Скачиваю… ${Math.round((p.p || 0) * 100)}%`; }
  else { bar.firstChild.style.width = '100%'; $('#selfN').textContent = p.stage === 'check' ? 'Проверяю файл…' : 'Перезапускаюсь…'; }
}

/* ---------------- start */
(async () => {
  build();
  document.querySelectorAll('[data-w]').forEach(b => b.addEventListener('click', () => call('win_ctl', { a: b.dataset.w })));
  $('#bCheck').onclick = () => check(true);
  $('#bAll').onclick = updateAll;
  $('#bLogs').onclick = () => call('open_logs');
  $('#selfGo').onclick = () => { $('#selfGo').disabled = true; call('upd_install'); };
  $('#veil').addEventListener('click', e => { if (e.target === $('#veil')) closeSheet(); });
  addEventListener('keydown', e => { if (e.key === 'Escape') closeSheet(); });
  on('job', e => { const j = e.payload; if (S.job[j.id]) { S.job[j.id] = j; paint(byId(j.id)); } });
  on('upd', selfEvent);
  // something may have changed while the user was in a setup window or in Firefox
  addEventListener('focus', () => { refreshLocal(); if (Date.now() - lastCheck > 30 * 60e3) check(false); });

  const info = await call('app_info').catch(() => ({}));
  $('#ver').textContent = info.version || '';
  if (info.updated) toast(`Менеджер обновлён до ${info.updated}`);
  await check(false);
  if (Q.get('notes')) { card(Q.get('notes')).classList.add('open'); paint(byId(Q.get('notes'))); }
  if (Q.get('ask')) ask(byId(Q.get('ask')));
  if (Q.get('busy')) run(byId(Q.get('busy')), 'install');
})();
})();
