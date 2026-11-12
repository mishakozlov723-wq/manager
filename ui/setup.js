'use strict';
// The installer and the uninstaller window of the manager. It is the same exe as the app (see
// src-tauri/src/setup.rs): named «…Setup….exe» it installs itself, started with --uninstall it removes itself.
// In a plain browser (no Tauri) it runs on made-up data, so the look can be checked anywhere:
// setup.html?mode=uninstall · ?was · ?fail · ?auto · ?at=prog|done|undone
(() => {
const RM = window.RM, { sleep, $ } = RM;
const Q = new URLSearchParams(location.search);
const UN = ((window.RM_SETUP && window.RM_SETUP.mode) || Q.get('mode')) === 'uninstall';

const LS = [];
const MOCK = {
  setup_info: () => ({ version: '1.0.3', was: Q.has('was') ? '1.0.1' : '', installed: Q.has('was'), desktop: true, auto: Q.has('auto') }),
  setup_run: async () => {
    for (const [p, t] of [[10, 'Закрываю Retocker Manager…'], [40, 'Копирую программу…'], [70, 'Создаю ярлыки…'], [88, 'Записываю в Windows…'], [100, 'Готово']]) { LS.forEach(f => f({ payload: { p, t } })); await sleep(Q.get('at') === 'prog' ? 1e6 : 420); }
    if (Q.has('fail')) throw 'Не получилось скопировать программу: Отказано в доступе. (os error 5)';
    return 'C:\\Users\\Миша\\AppData\\Local\\Programs\\Retocker Manager\\rt-manager.exe';
  },
  uninst_run: async () => { for (const [p, t] of [[15, 'Закрываю Retocker Manager…'], [40, 'Убираю ярлыки…'], [60, 'Убираю из списка программ…'], [100, 'Готово']]) { LS.forEach(f => f({ payload: { p, t } })); await sleep(420); } },
  setup_launch: () => {}, setup_close: () => {}, setup_min: () => {},
};
const call = RM.tauri ? RM.invoke : async (cmd, args) => MOCK[cmd](args || {});
const on = RM.tauri ? RM.listen : (ev, fn) => { LS.push(fn); };

const show = id => document.querySelectorAll('.scr').forEach(s => s.classList.toggle('on', s.id === 's-' + id));
const prog = ({ p, t }) => { $('#gBar').style.width = p + '%'; $('#gStep').textContent = t; };
let info = {};

async function install() {
  $('#gTitle').textContent = info.installed ? 'Обновляю…' : 'Устанавливаю…';
  prog({ p: 2, t: 'Готовлюсь…' }); show('prog');
  try {
    await call('setup_run', { desktop: $('#wDesk').checked });
    await sleep(350);
    $('#dTxt').textContent = info.installed ? `Retocker Manager обновлён до версии ${info.version}.` : 'Retocker Manager установлен. Ярлык — в меню «Пуск»' + ($('#wDesk').checked ? ' и на рабочем столе.' : '.');
    $('#stage').classList.add('done');
    show('done');
  } catch (e) { $('#eTitle').textContent = 'Не получилось установить'; $('#eTxt').textContent = String(e); show('err'); }
}

async function uninstall() {
  $('#gTitle').textContent = 'Удаляю…';
  prog({ p: 2, t: 'Готовлюсь…' }); show('prog');
  try { await call('uninst_run'); await sleep(350); show('undone'); }
  catch (e) { $('#eTitle').textContent = 'Не получилось удалить'; $('#eTxt').textContent = String(e); show('err'); }
}

// the build check: the window walks through its screens by itself
async function auto() {
  await sleep(2500);
  if (UN) { $('#uGo').click(); } else { $('#wGo').click(); }
  for (let i = 0; i < 100 && !$('#s-done.on,#s-undone.on,#s-err.on'); i++) await sleep(200);
  await sleep(2500);
  call('setup_close');
}

(async () => {
  const close = () => call('setup_close');
  $('#wMin').onclick = () => call('setup_min');
  ['#wClose', '#dClose', '#eClose', '#uCancel', '#udClose'].forEach(s => { $(s).onclick = close; });
  $('#wGo').onclick = install;
  $('#eRetry').onclick = () => (UN ? uninstall() : install());
  $('#dRun').onclick = () => call('setup_launch');
  $('#uGo').onclick = uninstall;
  on('setup', e => prog(e.payload));
  info = await call('setup_info').catch(() => ({}));
  if (UN) {
    $('#ttl').textContent = document.title = 'Удаление Retocker Manager';
    $('#stage').classList.add('un');
    show(Q.get('at') || 'un');
  } else {
    $('#wVer').textContent = info.version || '';
    $('#wDesk').checked = info.desktop !== false;
    if (info.installed) {
      $('#wWas').hidden = false;
      $('#wWas').lastChild.textContent = info.was && info.was !== info.version ? `Уже установлен (${info.was}) — обновлю` : 'Уже установлен — поставлю заново поверх';
      $('#wGo').textContent = 'Обновить';
    }
    show('welcome');
    if (Q.get('at') === 'prog') install();
    if (Q.get('at') === 'done') { $('#dTxt').textContent = 'Retocker Manager установлен. Ярлык — в меню «Пуск» и на рабочем столе.'; $('#stage').classList.add('done'); show('done'); }
  }
  if (info.auto) auto();
})();
})();
