// Couchside web UI. Plain JavaScript, no build step.
'use strict';

const app = document.getElementById('app');
const topbar = document.getElementById('topbar');
const sidebar = document.getElementById('sidebar');
const mainCol = document.getElementById('main');
const PAGE_SIZE = 120;

let me = null;          // signed-in user
let serverInfo = null;  // { needs_setup, version, ffprobe, ffmpeg, tmdb }
let cleanup = null;     // called before leaving the current page

// ---------- Helpers ----------

class ApiError extends Error {
  constructor(message, status, data) { super(message); this.status = status; this.data = data; }
}

async function api(path, { method = 'GET', body } = {}) {
  let res;
  try {
    res = await fetch('/api/v1' + path, {
      method,
      credentials: 'same-origin',
      cache: 'no-store',
      headers: body !== undefined ? { 'Content-Type': 'application/json' } : {},
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
  } catch (err) {
    setConnected(false);
    throw err;
  }
  setConnected(res.status < 500);
  if (res.status === 401 && path !== '/auth/login') {
    me = null;
    if (!location.hash.startsWith('#/login')) location.hash = '#/login';
    throw new ApiError('Please sign in.', 401);
  }
  if (res.status === 204) return null;
  const data = await res.json().catch(() => null);
  if (!res.ok) throw new ApiError((data && data.message) || res.statusText, res.status, data);
  return data;
}

function esc(value) {
  return String(value == null ? '' : value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}

function toast(message) {
  const el = document.getElementById('toast');
  el.textContent = message;
  el.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => { el.hidden = true; }, 3500);
}

function fmtRuntime(seconds) {
  if (!seconds) return '';
  const m = Math.round(seconds / 60);
  return m >= 60 ? `${Math.floor(m / 60)}h ${m % 60}m` : `${m}m`;
}

function fmtSize(bytes) {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0; let n = bytes;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
  return `${n.toFixed(i >= 3 ? 2 : 0)} ${units[i]}`;
}

function fmtDate(unix) {
  return unix ? new Date(unix * 1000).toLocaleString() : 'never';
}

function pad(n) { return String(n).padStart(2, '0'); }

function epCode(item) {
  if (item.season == null || item.episode == null) return '';
  let code = `S${pad(item.season)}E${pad(item.episode)}`;
  if (item.episode_end) code += `-E${pad(item.episode_end)}`;
  return code;
}

function resolution(item) {
  if (!item.height) return '';
  if (item.height >= 2000) return '4K';
  if (item.height >= 1000) return '1080p';
  if (item.height >= 700) return '720p';
  return `${item.height}p`;
}

function progressPercent(item) {
  const p = item.progress;
  if (!p || p.played || !item.runtime_seconds || !p.position_seconds) return 0;
  return Math.min(100, (p.position_seconds / item.runtime_seconds) * 100);
}

// The TMDB id changes when a movie is re-identified, so it's added to artwork
// URLs to make browsers fetch the new poster instead of showing a cached one.
const itemImg = (item, kind) => `/api/v1/items/${item.id}/image/${kind}${item.tmdb_id ? '?v=' + item.tmdb_id : ''}`;
const showImg = (id, kind) => `/api/v1/shows/${id}/image/${kind}`;

function artHtml(src, hasImage, label) {
  const placeholder = `<div class="placeholder">${esc(label)}</div>`;
  if (!hasImage) return placeholder;
  return `${placeholder}<img src="${src}" alt="" loading="lazy" onerror="this.remove()">`;
}

function overlays(item) {
  const pct = progressPercent(item);
  return (item.progress && item.progress.played ? '<span class="badge" title="Watched">✓</span>' : '') +
    (pct > 0 ? `<div class="progress"><span style="width:${pct.toFixed(1)}%"></span></div>` : '');
}

function movieCard(item) {
  return `<a class="card" href="#/item/${item.id}">
    <div class="art">${artHtml(itemImg(item, 'poster'), item.has_poster, item.title)}${overlays(item)}</div>
    <div class="title" title="${esc(item.title)}">${esc(item.title)}</div>
    <div class="sub">${esc(item.year)}</div>
  </a>`;
}

function episodeCard(item) {
  const hasArt = item.has_backdrop;
  const src = hasArt ? itemImg(item, 'backdrop') : itemImg(item, 'poster');
  return `<a class="card wide" href="#/item/${item.id}">
    <div class="art">${artHtml(src, hasArt || item.has_poster, item.title)}${overlays(item)}</div>
    <div class="title" title="${esc(item.title)}">${esc(item.title)}</div>
    <div class="sub">${esc(epCode(item))}</div>
  </a>`;
}

function itemCard(item) {
  return item.kind === 'episode' ? episodeCard(item) : movieCard(item);
}

function showCard(show) {
  return `<a class="card" href="#/show/${show.id}">
    <div class="art">${artHtml(showImg(show.id, 'poster'), show.has_poster, show.title)}</div>
    <div class="title" title="${esc(show.title)}">${esc(show.title)}</div>
    <div class="sub">${esc(show.year)}${show.year ? ' · ' : ''}${show.episode_count} episode${show.episode_count === 1 ? '' : 's'}</div>
  </a>`;
}

// The sidebar's "Server connected" line, updated by every API call.
function setConnected(ok) {
  const el = document.getElementById('conn');
  if (!el || el.classList.contains('off') === !ok) return;
  el.classList.toggle('off', !ok);
  document.getElementById('conn-text').textContent = ok ? 'Server connected' : 'Server not responding';
}

// Shows or hides the sidebar and top bar (hidden on sign-in screens and in the player).
function setChrome(on) {
  sidebar.hidden = !on;
  topbar.hidden = !on;
  mainCol.classList.toggle('with-sidebar', on);
}

function renderMe() {
  const initial = me.username.charAt(0);
  document.getElementById('side-name').textContent = me.username;
  document.getElementById('side-avatar').textContent = initial;
  document.getElementById('top-avatar').textContent = initial;
}

function setActiveNav(name) {
  document.querySelectorAll('[data-nav]').forEach(a => a.classList.toggle('active', a.dataset.nav === name));
}

function showError(err) {
  if (err instanceof ApiError && err.status === 401) return;
  app.innerHTML = `<div class="empty"><h2>Something went wrong</h2><p>${esc(err.message)}</p></div>`;
}

// ---------- Splash and scan progress ----------

function setBarFraction(el, fraction) {
  const scale = `scaleX(${Math.max(0, Math.min(1, fraction))})`;
  el.style.webkitTransform = scale;
  el.style.transform = scale;
}

// The start-up screen from index.html. `fraction` is how much of start-up is done,
// or null when that isn't known (the bar then just moves).
const splash = {
  el: document.getElementById('splash'),
  set(fraction, text) {
    if (!this.el) return;
    const known = typeof fraction === 'number';
    this.el.classList.toggle('indeterminate', !known);
    if (known) {
      setBarFraction(document.getElementById('splash-bar'), fraction);
      this.el.setAttribute('aria-valuenow', String(Math.round(fraction * 100)));
    } else {
      this.el.removeAttribute('aria-valuenow');
    }
    if (text) document.getElementById('splash-status').textContent = text;
  },
  finish() {
    if (!this.el) return;
    this.set(1, 'Ready');
    const el = this.el;
    this.el = null;
    el.classList.add('done');
    setTimeout(() => { el.remove(); }, 400);
  },
};

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

// GET /setup, retried while the server is still starting or restarting.
// While it starts, the server answers 503 with the step it is on.
async function loadServerInfo() {
  for (let attempt = 1; ; attempt++) {
    try {
      return await api('/setup');
    } catch (err) {
      const boot = err instanceof ApiError ? err.data : null;
      if (boot && boot.error === 'start_failed') throw err;
      if (boot && boot.error === 'starting') {
        attempt = 0;
        splash.set(0.05 + 0.35 * (boot.step / boot.steps), `${boot.message}…`);
        await sleep(250);
        continue;
      }
      if (err instanceof ApiError && err.status < 500) throw err;
      splash.set(null, attempt < 10
        ? 'Waiting for the Couchside server to start…'
        : 'Still waiting for the server. Check that the Couchside service is running.');
      await sleep(Math.min(attempt, 5) * 1000);
    }
  }
}

// Shows a thin bar along the top of the page while a library scan runs.
let scanTimer = null;
function watchScan() {
  if (scanTimer || !me) return;
  const bar = document.getElementById('scanbar');
  const poll = async () => {
    let s = null;
    try { s = await api('/scan'); } catch (_) { /* signed out or offline: stop until the next page */ }
    if (!s || !s.running) {
      bar.hidden = true;
      scanTimer = null;
      return;
    }
    const known = s.steps > 0;
    bar.classList.toggle('indeterminate', !known);
    if (known) {
      const fraction = Math.min(1, s.step / s.steps);
      setBarFraction(bar.querySelector('.scanbar-fill'), Math.max(fraction, 0.02));
      bar.setAttribute('aria-valuenow', String(Math.round(fraction * 100)));
    }
    bar.title = known ? `${s.phase} (step ${Math.min(s.step + 1, s.steps)} of ${s.steps})` : s.phase;
    bar.hidden = false;
    scanTimer = setTimeout(poll, 3000);
  };
  scanTimer = setTimeout(poll, 0);
}

// ---------- Auth screens ----------

function authPage({ title, intro, button, onSubmit, confirm }) {
  setChrome(false);
  app.innerHTML = `<form class="auth" autocomplete="on">
    <div class="logo"><img src="/favicon.svg" width="34" height="34" alt="">Couchside</div>
    <p>${esc(intro)}</p>
    <div class="field"><label for="u">Username</label><input id="u" type="text" autocomplete="username" required></div>
    <div class="field"><label for="p">Password</label><input id="p" type="password" autocomplete="${confirm ? 'new-password' : 'current-password'}" required></div>
    ${confirm ? '<div class="field"><label for="p2">Confirm password</label><input id="p2" type="password" autocomplete="new-password" required></div>' : ''}
    <div class="error" id="err"></div>
    <button class="btn primary" type="submit">${esc(button)}</button>
  </form>`;
  const form = app.querySelector('form');
  form.querySelector('#u').focus();
  form.addEventListener('submit', async e => {
    e.preventDefault();
    const username = form.querySelector('#u').value;
    const password = form.querySelector('#p').value;
    const errEl = form.querySelector('#err');
    if (confirm && password !== form.querySelector('#p2').value) {
      errEl.textContent = 'Passwords do not match.';
      return;
    }
    const btn = form.querySelector('button');
    btn.disabled = true;
    try {
      const res = await onSubmit(username, password);
      me = res.user;
      location.hash = '#/';
    } catch (err) {
      errEl.textContent = err.message;
    } finally {
      btn.disabled = false;
    }
  });
  document.title = `${title} · Couchside`;
}

function loginPage() {
  authPage({
    title: 'Sign in', intro: 'Sign in to watch your movies and shows.', button: 'Sign in',
    onSubmit: (username, password) => api('/auth/login', { method: 'POST', body: { username, password } }),
  });
}

function setupPage() {
  authPage({
    title: 'Welcome', intro: 'Welcome! Create the administrator account for this server.', button: 'Create account', confirm: true,
    onSubmit: async (username, password) => {
      const res = await api('/setup', { method: 'POST', body: { username, password } });
      serverInfo.needs_setup = false;
      return res;
    },
  });
}

// ---------- Pages ----------

const ICONS = {
  play: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 4.5v15l13-7.5z"/></svg>',
  prev: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m15 5-7 7 7 7"/></svg>',
  next: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m9 5 7 7-7 7"/></svg>',
};

// Moods for the home page, each a set of TMDB genre names (in English). When the library's
// genres are in another language and none match, the genres themselves are used as chips.
const MOODS = [
  ['Feel-good', ['Comedy', 'Family', 'Animation', 'Music']],
  ['Edge of your seat', ['Thriller', 'Action', 'Crime']],
  ['Mind-bending', ['Science Fiction', 'Mystery', 'Fantasy']],
  ['Heartfelt', ['Drama', 'Romance']],
  ['Spooky', ['Horror']],
  ['Epic adventures', ['Adventure', 'War', 'History', 'Western']],
  ['True stories', ['Documentary']],
];

// A shuffle that gives the same order all day, so the spotlight doesn't change on every visit.
function dailyShuffle(list) {
  const day = new Date().toDateString();
  let seed = 0;
  for (let i = 0; i < day.length; i++) seed = (seed * 31 + day.charCodeAt(i)) >>> 0;
  const rand = () => { seed = (seed * 1664525 + 1013904223) >>> 0; return seed / 4294967296; };
  const out = list.slice();
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.floor(rand() * (i + 1));
    const t = out[i]; out[i] = out[j]; out[j] = t;
  }
  return out;
}

function playHref(item) {
  const p = item.progress;
  return `#/play/${item.id}${p && !p.played && p.position_seconds > 30 ? '?resume=1' : ''}`;
}

function spotlightHtml(picks) {
  const slides = picks.map((m, i) => {
    const meta = [m.year, m.genres.slice(0, 2).join(' / '), fmtRuntime(m.runtime_seconds)]
      .filter(Boolean).map(t => `<span>${esc(t)}</span>`).join('<span class="sep">·</span>');
    const badge = resolution(m) === '4K' ? '<span class="qbadge">4K</span>' : '';
    return `<div class="spot-slide${i === 0 ? ' active' : ''}">
      <img class="spot-bg" src="${itemImg(m, 'backdrop')}" alt=""${i === 0 ? '' : ' loading="lazy"'} onerror="this.remove()">
      <div class="spot-body">
        <div class="spot-label">Tonight's spotlight</div>
        <h2 class="spot-title${m.title.length > 30 ? ' longer' : m.title.length > 16 ? ' long' : ''}">${esc(m.title)}</h2>
        <div class="spot-meta">${meta}${badge}</div>
        ${m.overview ? `<p class="spot-overview">${esc(m.overview)}</p>` : ''}
        <div class="spot-actions"><a class="btn primary" href="${playHref(m)}">${ICONS.play}Play now</a><a class="btn outline" href="#/item/${m.id}">View details</a></div>
      </div></div>`;
  }).join('');
  const pager = picks.length > 1 ? `<div class="spot-pager">
      <span class="spot-count"><b id="spot-num">01</b> / ${pad(picks.length)}</span>
      <button class="round-btn" type="button" id="spot-prev" aria-label="Previous">${ICONS.prev}</button>
      <button class="round-btn" type="button" id="spot-next" aria-label="Next">${ICONS.next}</button>
    </div>` : '';
  return `<section class="spotlight" id="spotlight" aria-roledescription="carousel" aria-label="Tonight's spotlight">${slides}${pager}</section>`;
}

// Moves the spotlight every 9 seconds, except while the pointer or focus is on it.
function startSpotlight() {
  const root = app.querySelector('#spotlight');
  const slides = root ? root.querySelectorAll('.spot-slide') : [];
  if (slides.length < 2) return () => {};
  let current = 0;
  let paused = false;
  const show = i => {
    current = (i + slides.length) % slides.length;
    slides.forEach((s, n) => s.classList.toggle('active', n === current));
    root.querySelector('#spot-num').textContent = pad(current + 1);
  };
  root.querySelector('#spot-prev').addEventListener('click', () => show(current - 1));
  root.querySelector('#spot-next').addEventListener('click', () => show(current + 1));
  root.addEventListener('mouseenter', () => { paused = true; });
  root.addEventListener('mouseleave', () => { paused = false; });
  root.addEventListener('focusin', () => { paused = true; });
  root.addEventListener('focusout', () => { paused = false; });
  const still = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  const timer = still ? null : setInterval(() => { if (!paused && !document.hidden) show(current + 1); }, 9000);
  return () => clearInterval(timer);
}

function continueCard(item) {
  const src = item.has_backdrop ? itemImg(item, 'backdrop')
    : (item.show_id ? showImg(item.show_id, 'backdrop') : itemImg(item, 'poster'));
  const p = item.progress;
  const left = p && item.runtime_seconds ? item.runtime_seconds - p.position_seconds : 0;
  const sub = [item.kind === 'episode' ? epCode(item) : item.year, left >= 60 ? `${fmtRuntime(left)} left` : '']
    .filter(Boolean).join(' · ');
  const pct = progressPercent(item);
  return `<div class="cw-card">
    <a class="cw-art" href="#/item/${item.id}">${artHtml(src, true, item.title)}<div class="cw-shade"></div>
      <div class="cw-text"><div class="cw-title" title="${esc(item.title)}">${esc(item.title)}</div>
        <div class="cw-line"><div class="cw-bar"><span style="width:${pct.toFixed(1)}%"></span></div><span class="cw-sub">${esc(sub)}</span></div></div></a>
    <a class="cw-play" href="#/play/${item.id}?resume=1" aria-label="Resume ${esc(item.title)}">${ICONS.play}</a>
  </div>`;
}

// The mood chips that have movies in this library, as [label, movies] pairs.
function moodGroups(movies) {
  const has = (m, names) => m.genres.some(g => names.indexOf(g) !== -1);
  const groups = MOODS.map(([label, names]) => [label, movies.filter(m => has(m, names))]).filter(g => g[1].length);
  if (groups.length) return groups;
  const counts = {};
  movies.forEach(m => m.genres.forEach(g => { counts[g] = (counts[g] || 0) + 1; }));
  return Object.keys(counts).sort((a, b) => counts[b] - counts[a]).slice(0, 7)
    .map(g => [g, movies.filter(m => m.genres.indexOf(g) !== -1)]);
}

async function homePage() {
  setActiveNav('home');
  document.title = 'Couchside';
  const results = await Promise.all([api('/home'), api('/movies?sort=added&limit=500')]);
  const home = results[0];
  const movies = dailyShuffle(results[1].items);
  const sections = [];
  const picks = movies.filter(m => m.has_backdrop && m.overview).slice(0, 4);
  if (picks.length) sections.push(spotlightHtml(picks));
  if (home.continue_watching.length) {
    sections.push(`<div class="row-head"><h2 class="row-title">Pick up where you left off</h2></div><div class="row cw">${home.continue_watching.map(continueCard).join('')}</div>`);
  }
  const moods = moodGroups(movies);
  if (moods.length) {
    sections.push(`<div class="row-head mood-head"><h2 class="row-title">Made for your mood</h2>
      <div class="mood-chips" id="moods">${moods.map((g, i) => `<button type="button" class="mood-chip${i === 0 ? ' active' : ''}" data-i="${i}">${esc(g[0])}</button>`).join('')}</div></div>
      <div class="row mood" id="mood-row"></div>`);
  }
  if (home.recent_movies.length) {
    sections.push(`<h2 class="row-title">Recently added movies</h2><div class="row">${home.recent_movies.map(movieCard).join('')}</div>`);
  }
  if (home.recent_episodes.length) {
    sections.push(`<h2 class="row-title">Recently added episodes</h2><div class="row wide">${home.recent_episodes.map(episodeCard).join('')}</div>`);
  }
  if (!sections.length) {
    const hint = me.is_admin
      ? '<p>Add a folder of movies or TV shows to get started.</p><p><a class="btn primary" href="#/settings">Add a library</a></p>'
      : '<p>Ask the server administrator to add some libraries.</p>';
    app.innerHTML = `<div class="empty"><h2>Nothing here yet</h2>${hint}</div>`;
    return;
  }
  app.innerHTML = sections.join('');
  // With a spotlight, the top bar floats over it.
  document.body.classList.toggle('has-spotlight', picks.length > 0);
  cleanup = startSpotlight();

  const chips = app.querySelector('#moods');
  if (chips) {
    const row = app.querySelector('#mood-row');
    const pick = i => {
      chips.querySelectorAll('.mood-chip').forEach(c => c.classList.toggle('active', Number(c.dataset.i) === i));
      row.innerHTML = moods[i][1].slice(0, 30).map(movieCard).join('');
      row.scrollLeft = 0;
    };
    chips.addEventListener('click', e => {
      const chip = e.target.closest('.mood-chip');
      if (chip) pick(Number(chip.dataset.i));
    });
    pick(0);
  }
}

async function moviesPage(params) {
  setActiveNav('movies');
  document.title = 'Movies · Couchside';
  const sort = params.get('sort') || 'title';
  app.innerHTML = `<div class="toolbar"><h1>Movies<span class="count" id="count"></span></h1>
    <select id="sort" aria-label="Sort by">
      <option value="title">A–Z</option><option value="added">Recently added</option><option value="year">Release year</option>
    </select></div>
    <div class="grid" id="grid"></div><div class="more" id="more"></div>`;
  const select = app.querySelector('#sort');
  select.value = sort;
  select.addEventListener('change', () => { location.hash = `#/movies?sort=${select.value}`; });
  await pagedGrid(offset => api(`/movies?sort=${encodeURIComponent(sort)}&limit=${PAGE_SIZE}&offset=${offset}`), movieCard,
    'No movies yet. Add a movie library in Settings.');
}

async function showsPage() {
  setActiveNav('shows');
  document.title = 'Series · Couchside';
  app.innerHTML = `<div class="toolbar"><h1>Series<span class="count" id="count"></span></h1></div>
    <div class="grid" id="grid"></div><div class="more" id="more"></div>`;
  await pagedGrid(offset => api(`/shows?limit=${PAGE_SIZE}&offset=${offset}`), showCard,
    'No series yet. Add a TV library in Settings.');
}

async function pagedGrid(fetchPage, render, emptyText) {
  const grid = app.querySelector('#grid');
  const more = app.querySelector('#more');
  let offset = 0;
  const load = async () => {
    const page = await fetchPage(offset);
    if (!grid.isConnected) return;
    offset += page.items.length;
    grid.insertAdjacentHTML('beforeend', page.items.map(render).join(''));
    app.querySelector('#count').textContent = page.total ? String(page.total) : '';
    if (!page.total) grid.outerHTML = `<div class="empty">${esc(emptyText)}</div>`;
    more.innerHTML = offset < page.total ? '<button class="btn" type="button">Load more</button>' : '';
    const btn = more.querySelector('button');
    if (btn) btn.addEventListener('click', () => { btn.disabled = true; load().catch(showError); });
  };
  await load();
}

async function showPage(id) {
  setActiveNav('shows');
  const { show, seasons } = await api(`/shows/${id}`);
  document.title = `${show.title} · Couchside`;
  const backdrop = show.has_backdrop ? `<div class="backdrop" style="background-image:url('${showImg(show.id, 'backdrop')}')"></div>` : '';
  const firstUnwatched = seasons.reduce((all, s) => all.concat(s.episodes), []).find(e => !(e.progress && e.progress.played));
  app.innerHTML = `<section class="hero">${backdrop}<div class="hero-inner">
      <div class="poster">${show.has_poster ? `<img src="${showImg(show.id, 'poster')}" alt="">` : ''}</div>
      <div>
        <h1>${esc(show.title)}</h1>
        <div class="meta">${show.year ? `<span>${show.year}</span>` : ''}<span>${seasons.length} season${seasons.length === 1 ? '' : 's'}</span><span>${show.episode_count} episodes</span></div>
        ${show.overview ? `<p class="overview">${esc(show.overview)}</p>` : ''}
        <div class="actions">${firstUnwatched ? `<a class="btn primary" href="#/play/${firstUnwatched.id}?resume=1">▶ Play ${esc(epCode(firstUnwatched))}</a>` : ''}</div>
      </div></div></section>
    <div class="tabs" id="tabs">${seasons.map((s, i) => `<button type="button" data-i="${i}">${s.season === 0 ? 'Specials' : `Season ${s.season}`}</button>`).join('')}</div>
    <div class="episodes" id="episodes"></div>`;

  const tabs = app.querySelector('#tabs');
  const list = app.querySelector('#episodes');
  const select = i => {
    tabs.querySelectorAll('button').forEach(b => b.classList.toggle('active', Number(b.dataset.i) === i));
    list.innerHTML = seasons[i].episodes.map(ep => {
      const thumb = ep.has_backdrop ? `<img src="${itemImg(ep, 'backdrop')}" alt="" loading="lazy" onerror="this.remove()">` : '';
      return `<a class="episode" href="#/item/${ep.id}">
        <div class="thumb">${thumb}${overlays(ep)}</div>
        <div class="info">
          <div class="name">${esc(ep.episode)}. ${esc(ep.title)}</div>
          <div class="ep-meta">${esc(epCode(ep))}${ep.runtime_seconds ? ` · ${fmtRuntime(ep.runtime_seconds)}` : ''}</div>
          ${ep.overview ? `<p>${esc(ep.overview)}</p>` : ''}
        </div></a>`;
    }).join('');
  };
  tabs.addEventListener('click', e => {
    const btn = e.target.closest('button');
    if (btn) select(Number(btn.dataset.i));
  });
  if (seasons.length) {
    const start = firstUnwatched ? seasons.findIndex(s => s.episodes.includes(firstUnwatched)) : 0;
    select(Math.max(0, start));
  } else {
    list.innerHTML = '<div class="empty">No episodes found.</div>';
  }
}

async function itemPage(id) {
  const d = await api(`/items/${id}`);
  const item = d.item;
  const isMovie = item.kind === 'movie';
  setActiveNav(isMovie ? 'movies' : 'shows');
  document.title = `${item.title} · Couchside`;
  const backdropSrc = item.has_backdrop ? itemImg(item, 'backdrop')
    : (item.show_id ? showImg(item.show_id, 'backdrop') : null);
  const backdrop = backdropSrc ? `<div class="backdrop" style="background-image:url('${backdropSrc}')"></div>` : '';
  const p = item.progress;
  const canResume = p && !p.played && p.position_seconds > 30;
  const meta = [
    !isMovie && d.show_title ? `<a href="#/show/${item.show_id}"><b>${esc(d.show_title)}</b></a>` : '',
    isMovie ? esc(item.year) : esc(epCode(item)),
    item.certification ? `<span class="cert">${esc(item.certification)}</span>` : '',
    fmtRuntime(item.runtime_seconds),
    item.rating ? `<span class="rating" title="TMDB user rating">★ ${item.rating.toFixed(1)}</span>` : '',
    resolution(item),
    p && p.played ? 'Watched' : '',
  ].filter(Boolean).map(m => `<span>${m}</span>`).join('');
  const genres = item.genres.length
    ? `<div class="chips">${item.genres.map(g => `<span class="chip">${esc(g)}</span>`).join('')}</div>` : '';
  const people = [
    item.directors.length ? `<div><span class="label">Director${item.directors.length > 1 ? 's' : ''}</span>${esc(item.directors.join(', '))}</div>` : '',
    item.cast.length ? `<div><span class="label">Starring</span>${esc(item.cast.slice(0, 6).join(', '))}</div>` : '',
  ].filter(Boolean).join('');
  const tech = [
    ['Original title', item.original_title],
    ['Released', item.release_date && new Date(`${item.release_date}T00:00:00`).toLocaleDateString(undefined, { year: 'numeric', month: 'long', day: 'numeric' })],
    ['File', d.file_name],
    ['Size', fmtSize(d.file_size)],
    ['Container', item.container],
    ['Video', item.video_codec && `${item.video_codec}${item.width ? ` ${item.width}×${item.height}` : ''}`],
    ['Audio', item.audio_codec],
    ['Subtitles', d.subtitles.map(s => s.label).join(', ')],
  ].filter(([, v]) => v).map(([k, v]) => `<dt>${k}</dt><dd>${esc(v)}</dd>`).join('');
  const links = [
    item.tmdb_id ? `<a href="https://www.themoviedb.org/movie/${item.tmdb_id}" target="_blank" rel="noopener noreferrer">TMDB</a>` : '',
    item.imdb_id ? `<a href="https://www.imdb.com/title/${encodeURIComponent(item.imdb_id)}/" target="_blank" rel="noopener noreferrer">IMDb</a>` : '',
  ].filter(Boolean).join(' · ');
  const unmatched = isMovie && me.is_admin && serverInfo && serverInfo.tmdb && !item.tmdb_id
    ? '<div class="notice info">This movie isn\'t matched to TMDB yet. If it stays this way after a scan, use <b>Identify</b> to pick it, or put the year in the file name, e.g. <i>Title (2010).mkv</i>.</div>' : '';

  app.innerHTML = `<section class="hero">${backdrop}<div class="hero-inner">
      <div class="poster">${item.has_poster ? `<img src="${itemImg(item, 'poster')}" alt="" onerror="this.remove()">` : ''}</div>
      <div>
        <h1>${esc(item.title)}</h1>
        ${item.tagline ? `<p class="tagline">${esc(item.tagline)}</p>` : ''}
        <div class="meta">${meta}</div>
        ${genres}
        ${item.overview ? `<p class="overview">${esc(item.overview)}</p>` : ''}
        ${people ? `<div class="people">${people}</div>` : ''}
        <div class="actions">
          ${canResume ? `<a class="btn primary" href="#/play/${item.id}?resume=1">▶ Resume from ${fmtClock(p.position_seconds)}</a>
                         <a class="btn" href="#/play/${item.id}">Play from start</a>`
                      : `<a class="btn primary" href="#/play/${item.id}">▶ Play</a>`}
          <button class="btn" type="button" id="watched">${p && p.played ? 'Mark unwatched' : 'Mark watched'}</button>
          ${isMovie && me.is_admin ? '<button class="btn" type="button" id="identify">Identify…</button>' : ''}
          ${isMovie && me.is_admin ? '<button class="btn" type="button" id="cleanup">Clean up…</button>' : ''}
          ${isMovie && me.is_admin ? '<button class="btn danger" type="button" id="delete-movie">Delete…</button>' : ''}
        </div>
        ${unmatched}
        ${d.browser_playable ? '' : '<div class="notice">Web browsers can\'t play this file\'s format as it is, so Couchside converts it while it plays (this needs FFmpeg).</div>'}
      </div></div></section>
    <dl class="tech">${tech}</dl>
    ${links ? `<p class="hint ext-links">More on ${links}${item.match_locked ? ' · match chosen by hand' : ''}</p>` : ''}`;

  app.querySelector('#watched').addEventListener('click', async () => {
    const played = !(p && p.played);
    await api(`/items/${item.id}/progress`, { method: 'PUT', body: { position_seconds: 0, played } });
    toast(played ? 'Marked as watched' : 'Marked as unwatched');
    itemPage(id).catch(showError);
  });
  const identifyBtn = app.querySelector('#identify');
  if (identifyBtn) identifyBtn.addEventListener('click', () => openIdentify(item, () => itemPage(id).catch(showError)));
  const cleanupBtn = app.querySelector('#cleanup');
  if (cleanupBtn) cleanupBtn.addEventListener('click', () => openCleanup(item, () => itemPage(id).catch(showError)));
  const deleteBtn = app.querySelector('#delete-movie');
  if (deleteBtn) deleteBtn.addEventListener('click', () => openDelete(item));
}

// ---------- Identify dialog ----------

function closeDialog() {
  const existing = document.getElementById('dialog');
  if (existing) existing.remove();
  document.removeEventListener('keydown', dialogKeys);
}

function dialogKeys(e) {
  if (e.key === 'Escape') closeDialog();
}

function openIdentify(item, onDone) {
  closeDialog();
  if (!serverInfo || !serverInfo.tmdb) {
    toast('Add a TMDB API key in Settings first.');
    location.hash = '#/settings';
    return;
  }
  document.body.insertAdjacentHTML('beforeend', `<div class="dialog-backdrop" id="dialog">
    <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="dialog-title">
      <div class="dialog-head"><h2 id="dialog-title">Identify movie</h2><button class="link" type="button" id="dialog-close" aria-label="Close">✕</button></div>
      <p class="hint">Search TMDB and pick the right movie. Couchside keeps your choice on future scans.</p>
      ${serverInfo.ffmpeg ? `<div class="title-scan-bar">
        <button class="btn small" type="button" id="title-scan-btn">Scan video for title</button>
        <button class="btn small" type="button" id="title-scan-end-btn">Scan end credits</button>
        <span class="hint">Reads on-screen text from the first 10 minutes, or the last 12 for movies that show their title in the credits. You choose what to search for.</span>
      </div>
      <div id="title-scan" class="title-scan" hidden></div>` : ''}
      <form id="identify-form" class="identify-form">
        <input id="identify-q" type="text" aria-label="Title" placeholder="Title">
        <input id="identify-year" type="text" inputmode="numeric" aria-label="Year" placeholder="Year">
        <button class="btn primary" type="submit">Search</button>
      </form>
      <div id="identify-results" class="candidates"><div class="loading">Searching…</div></div>
    </div></div>`);
  const dialog = document.getElementById('dialog');
  const qInput = dialog.querySelector('#identify-q');
  const yearInput = dialog.querySelector('#identify-year');
  const results = dialog.querySelector('#identify-results');
  dialog.querySelector('#dialog-close').addEventListener('click', closeDialog);
  dialog.addEventListener('click', e => { if (e.target === dialog) closeDialog(); });
  document.addEventListener('keydown', dialogKeys);

  const render = list => {
    if (!list.length) {
      results.innerHTML = '<div class="empty">No results. Try a shorter title or remove the year.</div>';
      return;
    }
    results.innerHTML = list.map(c => `<button type="button" class="candidate${c.current ? ' current' : ''}" data-id="${c.tmdb_id}">
        <div class="thumb">${c.poster_path ? `<img src="/api/v1/tmdb/image?path=${encodeURIComponent(c.poster_path)}" alt="" loading="lazy" onerror="this.remove()">` : ''}</div>
        <div class="info">
          <div class="name">${esc(c.title)} ${c.year ? `<span class="muted">(${c.year})</span>` : ''}${c.current ? ' <span class="pill ok">Current</span>' : ''}${c.score >= 0.6 && !c.current ? ' <span class="pill">Good match</span>' : ''}</div>
          ${c.original_title ? `<div class="muted">${esc(c.original_title)}</div>` : ''}
          ${c.overview ? `<p>${esc(c.overview)}</p>` : ''}
        </div></button>`).join('');
  };

  const search = async (useDefaults) => {
    results.innerHTML = '<div class="loading">Searching…</div>';
    const params = new URLSearchParams();
    if (!useDefaults) {
      params.set('q', qInput.value.trim());
      if (/^\d{4}$/.test(yearInput.value.trim())) params.set('year', yearInput.value.trim());
    }
    try {
      render(await api(`/items/${item.id}/identify?${params}`));
    } catch (err) {
      results.innerHTML = `<div class="error">${esc(err.message)}</div>`;
    }
  };

  dialog.querySelector('#identify-form').addEventListener('submit', e => { e.preventDefault(); search(false); });
  const scanBtn = dialog.querySelector('#title-scan-btn');
  if (scanBtn) {
    setupTitleScan(item, dialog.querySelector('#title-scan'), scanBtn, dialog.querySelector('#title-scan-end-btn'), text => {
      qInput.value = text;
      yearInput.value = '';
      search(false);
    });
  }
  results.addEventListener('click', async e => {
    const btn = e.target.closest('.candidate');
    if (!btn) return;
    results.querySelectorAll('.candidate').forEach(b => { b.disabled = true; });
    btn.classList.add('busy');
    try {
      await api(`/items/${item.id}/identify`, { method: 'POST', body: { tmdb_id: Number(btn.dataset.id) } });
      closeDialog();
      toast('Movie updated');
      onDone();
    } catch (err) {
      results.querySelectorAll('.candidate').forEach(b => { b.disabled = false; });
      btn.classList.remove('busy');
      toast(err.message);
    }
  });

  // Pre-fill with what the file name says, then search with it.
  api(`/items/${item.id}/identify`).then(render).catch(err => {
    results.innerHTML = `<div class="error">${esc(err.message)}</div>`;
  });
  qInput.value = item.title;
  yearInput.value = item.year == null ? '' : item.year;
  qInput.focus();
  qInput.select();
}

// ---------- Delete dialog ----------

// Shows exactly what will go to the Recycle Bin, then deletes it and removes the movie.
function openDelete(item) {
  closeDialog();
  document.body.insertAdjacentHTML('beforeend', `<div class="dialog-backdrop" id="dialog">
    <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="dialog-title">
      <div class="dialog-head"><h2 id="dialog-title">Delete movie</h2><button class="link" type="button" id="dialog-close" aria-label="Close">✕</button></div>
      <p class="hint">Sends <b>${esc(item.title)}</b> to the Recycle Bin and removes it from Couchside, including everyone's watch progress.</p>
      <div id="delete-body"><div class="loading">Checking…</div></div>
    </div></div>`);
  const dialog = document.getElementById('dialog');
  const body = dialog.querySelector('#delete-body');
  dialog.querySelector('#dialog-close').addEventListener('click', closeDialog);
  dialog.addEventListener('click', e => { if (e.target === dialog) closeDialog(); });
  document.addEventListener('keydown', dialogKeys);

  const render = preview => {
    const what = preview.folder
      ? `<div class="label">Delete this folder and everything in it</div><ul><li>${esc(preview.folder)}</li></ul>`
      : `<div class="label">Delete these files</div><ul>${preview.files.map(f => `<li>${esc(f)}</li>`).join('')}</ul>
         <div class="hint">Other movies in the same folder are not touched.</div>`;
    body.innerHTML = `<div class="rename-preview">${what}</div>
      <div class="hint">Files on a network share have no Recycle Bin and are deleted permanently.</div>
      <div class="error" role="alert"></div>
      <div class="actions dialog-actions">
        <button class="btn danger" type="button" id="delete-confirm">Delete movie</button>
        <button class="btn" type="button" id="delete-cancel">Cancel</button>
      </div>`;
    body.querySelector('#delete-cancel').addEventListener('click', closeDialog);
    const confirmBtn = body.querySelector('#delete-confirm');
    confirmBtn.addEventListener('click', async () => {
      body.querySelectorAll('button').forEach(b => { b.disabled = true; });
      confirmBtn.textContent = 'Deleting…';
      try {
        await api(`/items/${item.id}/delete`, { method: 'POST', body: { folder: preview.folder, files: preview.files } });
        closeDialog();
        toast(`${item.title} deleted`);
        location.hash = '#/movies';
      } catch (err) {
        body.querySelectorAll('button').forEach(b => { b.disabled = false; });
        confirmBtn.textContent = 'Delete movie';
        body.querySelector('.error').textContent = err.message;
      }
    });
    body.querySelector('#delete-cancel').focus();
  };

  api(`/items/${item.id}/delete`).then(render).catch(err => {
    body.innerHTML = `<div class="error">${esc(err.message)}</div>`;
  });
}

// ---------- Clean up dialog ----------

// Rebuilds a movie's details from its TMDB match. When renaming is switched on in config.toml,
// it also shows the old and new file names and only renames if the user picks that button.
function openCleanup(item, onDone) {
  closeDialog();
  if (!serverInfo || !serverInfo.tmdb) {
    toast('Add a TMDB API key in Settings first.');
    location.hash = '#/settings';
    return;
  }
  document.body.insertAdjacentHTML('beforeend', `<div class="dialog-backdrop" id="dialog">
    <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="dialog-title">
      <div class="dialog-head"><h2 id="dialog-title">Clean up movie</h2><button class="link" type="button" id="dialog-close" aria-label="Close">✕</button></div>
      <p class="hint">Deletes the details and artwork Couchside has stored for <b>${esc(item.title)}</b> and downloads them again from TMDB for the movie it is identified as. Watch progress is kept.</p>
      <div id="cleanup-body"><div class="loading">Checking…</div></div>
    </div></div>`);
  const dialog = document.getElementById('dialog');
  const body = dialog.querySelector('#cleanup-body');
  dialog.querySelector('#dialog-close').addEventListener('click', closeDialog);
  dialog.addEventListener('click', e => { if (e.target === dialog) closeDialog(); });
  document.addEventListener('keydown', dialogKeys);

  const run = async (changes, button) => {
    body.querySelectorAll('button').forEach(b => { b.disabled = true; });
    button.textContent = 'Working…';
    try {
      const result = await api(`/items/${item.id}/cleanup`, { method: 'POST', body: changes });
      closeDialog();
      toast(result.message);
      onDone();
    } catch (err) {
      body.querySelectorAll('button').forEach(b => { b.disabled = false; });
      body.querySelector('.error').textContent = err.message;
      button.textContent = button.dataset.label;
    }
  };

  const render = preview => {
    let rename = '';
    if (preview.renames_enabled && preview.new_name) {
      const others = preview.other_steps.map(step =>
        `<li><span class="muted">${esc(step.from)}</span> → ${esc(step.to)}</li>`).join('');
      rename = `<div class="rename-preview">
          <div class="label">Rename the file</div>
          <div class="rename-row"><span class="muted">From</span><code>${esc(preview.current_name)}</code></div>
          <div class="rename-row"><span class="muted">To</span><code>${esc(preview.new_name)}</code></div>
          ${others ? `<div class="label">Also renamed with it</div><ul>${others}</ul>` : ''}
        </div>`;
    } else if (preview.renames_enabled && preview.rename_note) {
      rename = `<div class="notice info">${esc(preview.rename_note)}</div>`;
    }
    const removals = preview.renames_enabled ? preview.remove_files : [];
    const remove = removals.length ? `<div class="rename-preview">
        <div class="label">Remove ${removals.length} image and subtitle file${removals.length === 1 ? '' : 's'}</div>
        <ul>${removals.map(f => `<li>${esc(f)}</li>`).join('')}</ul>
        <div class="hint">Moved to Couchside's <i>removed</i> folder (not deleted). Video files are never removed.</div>
      </div>` : '';
    const changesFiles = preview.renames_enabled && (preview.new_name || removals.length);
    const changes = { rename_to: preview.new_name || null, remove_files: removals };
    body.innerHTML = `${rename}${remove}
      <div class="error" role="alert"></div>
      <div class="actions dialog-actions">
        ${changesFiles ? '<button class="btn primary" type="button" id="cleanup-files" data-label="Clean up files too">Clean up files too</button>' : ''}
        <button class="btn${changesFiles ? '' : ' primary'}" type="button" id="cleanup-only" data-label="${changesFiles ? 'Details only' : 'Clean up'}">${changesFiles ? 'Details only' : 'Clean up'}</button>
        <button class="btn" type="button" id="cleanup-cancel">Cancel</button>
      </div>`;
    body.querySelector('#cleanup-cancel').addEventListener('click', closeDialog);
    const only = body.querySelector('#cleanup-only');
    only.addEventListener('click', () => run({}, only));
    const filesBtn = body.querySelector('#cleanup-files');
    if (filesBtn) filesBtn.addEventListener('click', () => run(changes, filesBtn));
    (filesBtn || only).focus();
  };

  api(`/items/${item.id}/cleanup`).then(render).catch(err => {
    body.innerHTML = `<div class="error">${esc(err.message)}</div>`;
  });
}

// "Scan video for title" / "Scan end credits": shows frames from the start or the end of the movie
// with the text read on them. Nothing is matched automatically; clicking a piece of text only
// fills in the search.
function setupTitleScan(item, panel, startButton, endButton, useText) {
  const pick = text => `<button type="button" class="chip scan-pick" data-text="${esc(text)}">${esc(text)}</button>`;
  const render = (scan, stamp, part) => {
    const notes = [];
    if (!scan.ocr_available) notes.push('Text recognition isn\'t available on this PC, so check the frames yourself.');
    else if (!scan.suggestions.length) {
      notes.push(part === 'start'
        ? 'No title-like text was found. Check the frames below, or try Scan end credits.'
        : 'No title-like text was found. Check the frames below.');
    }
    const suggestions = scan.suggestions.length
      ? `<div class="scan-label">Text that looks like a title. Click one to search for it:</div><div class="chips">${scan.suggestions.map(pick).join('')}</div>` : '';
    const frames = scan.frames.map(f => `<figure class="scan-frame">
        <img src="/api/v1/items/${item.id}/title-scan/frames/${f.index}?t=${stamp}" alt="Frame at ${fmtClock(f.time_seconds)}" loading="lazy">
        <figcaption><span class="muted">${fmtClock(f.time_seconds)}</span>${f.lines.map(pick).join('')}</figcaption>
      </figure>`).join('');
    panel.innerHTML = `${notes.map(n => `<div class="hint">${n}</div>`).join('')}${suggestions}
      <div class="scan-label">${scan.frames.length} of ${scan.frames_read} frames shown. Click a frame to enlarge it.</div>
      <div class="scan-frames">${frames}</div>`;
  };

  const scan = async (button, part) => {
    const label = button.textContent;
    startButton.disabled = endButton.disabled = true;
    button.textContent = 'Scanning…';
    panel.hidden = false;
    panel.innerHTML = `<div class="loading">Reading the ${part === 'start' ? 'start' : 'end credits'} of the video. This can take a minute…</div>`;
    try {
      render(await api(`/items/${item.id}/title-scan?part=${part}`, { method: 'POST' }), Date.now(), part);
    } catch (err) {
      panel.innerHTML = `<div class="error">${esc(err.message)}</div>`;
    } finally {
      startButton.disabled = endButton.disabled = false;
      button.textContent = label;
    }
  };
  startButton.addEventListener('click', () => scan(startButton, 'start'));
  endButton.addEventListener('click', () => scan(endButton, 'end'));
  panel.addEventListener('click', e => {
    const chosen = e.target.closest('.scan-pick');
    if (chosen) { useText(chosen.dataset.text); return; }
    const frame = e.target.closest('.scan-frame');
    if (frame && e.target.tagName === 'IMG') frame.classList.toggle('big');
  });
}

function fmtClock(seconds) {
  const s = Math.floor(seconds);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h ? `${h}:${pad(m)}:${pad(s % 60)}` : `${m}:${pad(s % 60)}`;
}

// ----- Playback: what this device can play, and repackaged (HLS) streams -----

const PLAY_MODES = { direct_play: 'Direct play', remux: 'Repackaging', audio_convert: 'Converting audio', transcode: 'Converting video' };
const VIDEO_ENCODERS = {
  gpu: 'Video conversion: NVIDIA GPU',
  cpu: 'Video conversion: processor',
  gpu_failed: 'Video conversion: processor (GPU failed)',
  unavailable: 'Video conversion: needs FFmpeg',
};
const QUALITIES = [['auto', 'Auto'], ['original', 'Original'], ['1080p', '1080p'], ['720p', '720p'], ['480p', '480p']];

// The quality chosen in the player, remembered per device.
function savedQuality() {
  try {
    const q = localStorage.getItem('couchside.quality');
    if (QUALITIES.some(pair => pair[0] === q)) return q;
  } catch (e) { /* storage blocked: use the default */ }
  return 'auto';
}

const isTv = () => /Tizen|SMART-TV|SmartTV|Web0S/i.test(navigator.userAgent);

// A random id this browser keeps, so a new stream from it replaces its previous one.
function deviceId() {
  const make = () => Array.from({ length: 16 }, () => Math.floor(Math.random() * 16).toString(16)).join('');
  try {
    let id = localStorage.getItem('couchside.device');
    if (!id) { id = make(); localStorage.setItem('couchside.device', id); }
    return id;
  } catch (e) {
    return make();
  }
}

// TV mode (only H.264/AAC in MP4, up to 1080p): on by default on TVs, whose format
// answers can't be fully trusted. `couchside.tvMode` = '1' / '0' overrides it.
function tvMode() {
  try {
    const saved = localStorage.getItem('couchside.tvMode');
    if (saved === '1' || saved === '0') return saved === '1';
  } catch (e) { /* storage blocked: use the default */ }
  return isTv();
}

// The formats this browser says it plays, in the names the server uses.
function playbackProfile(video) {
  const can = type => {
    try { if (window.MediaSource && MediaSource.isTypeSupported(type)) return true; } catch (e) { /* ignore */ }
    return video.canPlayType(type) !== '';
  };
  const pick = table => Object.keys(table).filter(name => can(table[name]));
  const containers = ['mp4'];
  if (can('video/webm')) containers.push('webm');
  // Chrome plays most MKV files when they're served as WebM (Couchside does); TVs and
  // Safari (every iPhone/iPad browser) don't.
  const safari = /AppleWebKit/.test(navigator.userAgent) && !/Chrome|Chromium|Android/.test(navigator.userAgent);
  if (can('video/x-matroska') || (containers.includes('webm') && !isTv() && !safari)) containers.push('mkv');
  return {
    containers,
    video: pick({
      h264: 'video/mp4; codecs="avc1.640028"',
      h264_high10: 'video/mp4; codecs="avc1.6E0028"',
      hevc: 'video/mp4; codecs="hvc1.1.6.L120.90"',
      hevc_main10: 'video/mp4; codecs="hvc1.2.4.L153.90"',
      vp9: 'video/webm; codecs="vp9"',
      av1: 'video/mp4; codecs="av01.0.08M.08"',
    }),
    audio: pick({
      aac: 'audio/mp4; codecs="mp4a.40.2"',
      mp3: 'audio/mpeg',
      opus: 'audio/webm; codecs="opus"',
      vorbis: 'audio/webm; codecs="vorbis"',
      flac: 'audio/mp4; codecs="flac"',
      ac3: 'audio/mp4; codecs="ac-3"',
      eac3: 'audio/mp4; codecs="ec-3"',
      dts: 'audio/mp4; codecs="dtsc"',
      truehd: 'audio/mp4; codecs="mlpa"',
    }),
    tv_mode: tvMode(),
  };
}

let hlsScript = null;
function loadHlsJs() {
  if (window.Hls) return Promise.resolve(window.Hls);
  if (!hlsScript) {
    hlsScript = new Promise((resolve, reject) => {
      const s = document.createElement('script');
      s.src = '/vendor/hls.min.js';
      s.onload = () => resolve(window.Hls);
      s.onerror = () => { hlsScript = null; reject(new Error('Could not load the stream player.')); };
      document.head.appendChild(s);
    });
  }
  return hlsScript;
}

function stopStream(sessionId) {
  if (!sessionId) return;
  const url = `/api/v1/transcode/${sessionId}/stop`;
  if (!(navigator.sendBeacon && navigator.sendBeacon(url))) {
    fetch(url, { method: 'POST', credentials: 'same-origin', keepalive: true }).catch(() => {});
  }
}

async function playerPage(id, params) {
  const d = await api(`/items/${id}`);
  const item = d.item;
  setChrome(false);
  document.title = `${item.title} · Couchside`;
  const isLang = l => /^[a-z]{2,3}(-[A-Za-z0-9]{2,8})?$/.test(l || '');
  const tracks = d.subtitles.map(s =>
    `<track kind="subtitles" src="/api/v1/subtitles/${s.id}" label="${esc(s.label)}"${isLang(s.language) ? ` srclang="${esc(s.language)}"` : ''}>`).join('');
  const title = item.kind === 'episode' && d.show_title ? `${d.show_title} — ${epCode(item)} ${item.title}` : item.title;
  app.innerHTML = `<div class="player" id="player">
      <div class="player-bar"><a class="btn small" href="#/item/${item.id}">← Back</a><span class="ptitle">${esc(title)}</span><span class="pmode" id="pmode" hidden></span>
        <select class="pquality" id="pquality" aria-label="Quality">${QUALITIES.map(pair => `<option value="${pair[0]}">${pair[1]}</option>`).join('')}</select></div>
      <video id="video" controls autoplay playsinline preload="metadata">${tracks}</video>
    </div>`;
  const player = app.querySelector('#player');
  const video = app.querySelector('#video');
  const modeLabel = app.querySelector('#pmode');
  const qualityPick = app.querySelector('#pquality');
  qualityPick.value = savedQuality();
  const resumeAt = params.get('resume') && item.progress && !item.progress.played ? item.progress.position_seconds : 0;

  let hls = null;
  let sessionId = null;
  let plan = null;
  let triedFallback = false;
  let closed = false;

  const showError = (heading, message) => {
    if (closed || player.querySelector('.play-error')) return;
    player.insertAdjacentHTML('beforeend', `<div class="play-error"><h3>${esc(heading)}</h3>
      <p>${esc(message)}</p><a class="btn" href="#/item/${item.id}">Back</a></div>`);
  };
  const formatText = () => `${item.video_codec || 'unknown'} / ${item.audio_codec || 'unknown'} in ${d.file_name.split('.').pop()}`;
  const cantPlay = () => {
    if (plan && plan.needs_video_conversion) {
      showError('This video needs converting', `${plan.reason || ''} Couchside converts video with FFmpeg, which it couldn't find (or the file hasn't been scanned yet). Install FFmpeg, or open the file in a player like VLC.`);
    } else {
      showError("Can't play this file", `The format (${formatText()}) didn't play here.${plan && plan.reason ? ` ${plan.reason}` : ''} Open the file in a player like VLC for now.`);
    }
  };

  const detach = () => {
    if (hls) { hls.destroy(); hls = null; }
    stopStream(sessionId);
    sessionId = null;
  };
  const seekOnLoad = startAt => {
    if (startAt > 0) video.addEventListener('loadedmetadata', () => { video.currentTime = startAt; }, { once: true });
  };

  const attach = async (next, startAt) => {
    detach();
    plan = next;
    sessionId = next.session_id || null;
    modeLabel.hidden = false;
    modeLabel.textContent = (PLAY_MODES[next.mode] || '') + (next.gpu ? ' (GPU)' : '');
    modeLabel.title = next.reason || '';
    if (next.mode === 'direct_play') {
      video.src = next.url;
      seekOnLoad(startAt);
      return;
    }
    const nativeHls = video.canPlayType('application/vnd.apple.mpegurl') !== '';
    const Hls = isTv() && nativeHls ? null : await loadHlsJs().catch(() => null);
    if (closed) { stopStream(sessionId); return; }
    if (Hls && Hls.isSupported()) {
      hls = new Hls({ startPosition: startAt > 0 ? startAt : -1, maxBufferLength: 30 });
      hls.on(Hls.Events.ERROR, (_event, data) => {
        if (data.fatal) { detach(); cantPlay(); }
      });
      hls.loadSource(next.url);
      hls.attachMedia(video);
    } else if (nativeHls) {
      video.src = next.url;
      seekOnLoad(startAt);
    } else {
      cantPlay();
    }
  };

  const request = (directFailed, startAt) => api(`/items/${item.id}/playback`, {
    method: 'POST',
    body: {
      profile: playbackProfile(video),
      device_id: deviceId(),
      start_seconds: startAt,
      direct_failed: directFailed,
      quality: qualityPick.value,
    },
  });

  // A new quality restarts playback where it is.
  qualityPick.addEventListener('change', async () => {
    try { localStorage.setItem('couchside.quality', qualityPick.value); } catch (e) { /* not remembered */ }
    const at = video.currentTime || resumeAt;
    triedFallback = false;
    try {
      await attach(await request(false, at), at);
      video.play().catch(() => {});
    } catch (e) {
      showError("Can't change quality", e.message);
    }
  });

  let lastSave = Date.now();
  const save = played => {
    if (!video.currentTime && !played) return;
    const body = { position_seconds: video.currentTime };
    if (played !== undefined) body.played = played;
    api(`/items/${item.id}/progress`, { method: 'PUT', body }).catch(() => {});
  };
  video.addEventListener('timeupdate', () => {
    if (Date.now() - lastSave > 10000) { lastSave = Date.now(); save(); }
  });
  video.addEventListener('pause', () => save());
  video.addEventListener('ended', () => save(true));
  video.addEventListener('error', async () => {
    if (closed || hls) return; // hls.js reports its own errors
    if (plan && plan.mode === 'direct_play' && !plan.needs_video_conversion && !triedFallback) {
      // The original file didn't play after all: ask for a repackaged stream instead.
      triedFallback = true;
      const at = video.currentTime || resumeAt;
      try {
        const next = await request(true, at);
        if (next.mode !== 'direct_play') { await attach(next, at); return; }
        plan = next;
      } catch (e) {
        showError("Can't play this file", e.message);
        return;
      }
    }
    cantPlay();
  });
  const onPageHide = () => stopStream(sessionId);
  window.addEventListener('pagehide', onPageHide);

  let idleTimer;
  const wake = () => {
    player.classList.remove('idle');
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => player.classList.add('idle'), 2500);
  };
  player.addEventListener('mousemove', wake);
  wake();
  const onKey = e => { if (e.key === 'Escape' && !document.fullscreenElement) location.hash = `#/item/${item.id}`; };
  document.addEventListener('keydown', onKey);

  cleanup = () => {
    closed = true;
    save();
    clearTimeout(idleTimer);
    document.removeEventListener('keydown', onKey);
    window.removeEventListener('pagehide', onPageHide);
    video.pause();
    detach();
    video.removeAttribute('src');
    video.load(); // stops the download
  };

  try {
    await attach(await request(false, resumeAt), resumeAt);
  } catch (e) {
    if (e.status === 503) { showError('Too many videos playing', e.message); return; }
    // Couldn't ask the server how to play it: try the original file.
    await attach({ mode: 'direct_play', url: `/api/v1/items/${item.id}/stream` }, resumeAt);
  }
}

async function searchPage(params) {
  setActiveNav('');
  const q = params.get('q') || '';
  document.getElementById('search-input').value = q;
  document.title = `Search · Couchside`;
  const res = await api(`/search?q=${encodeURIComponent(q)}`);
  const parts = [];
  if (res.shows.length) parts.push(`<h2 class="row-title">Series</h2><div class="grid">${res.shows.map(showCard).join('')}</div>`);
  if (res.movies.length) parts.push(`<h2 class="row-title">Movies</h2><div class="grid">${res.movies.map(movieCard).join('')}</div>`);
  app.innerHTML = `<h1>Results for “${esc(q)}”</h1>${parts.join('') || '<div class="empty">No matches.</div>'}`;
}

// ---------- Settings ----------

async function settingsPage() {
  setActiveNav('settings');
  document.title = 'Settings · Couchside';
  const adminPanels = me.is_admin ? `
    <section class="panel"><h2>Libraries</h2>
      <div class="list" id="libs"></div>
      <form id="lib-form" class="form-row">
        <div><label for="lib-name">Name</label><input id="lib-name" type="text" placeholder="Movies" required></div>
        <div><label for="lib-kind">Type</label><select id="lib-kind"><option value="movies">Movies</option><option value="shows">TV Shows</option></select></div>
        <div><label for="lib-path">Folder</label><input id="lib-path" type="text" placeholder="D:\\Movies or \\\\NAS\\Video\\Movies" required></div>
        <button class="btn primary" type="submit">Add</button>
      </form>
      <div class="error" id="lib-err"></div>
      <div class="hint">Movies: one file per movie, ideally named <i>Title (Year)</i>. TV: <i>Show Name/Season 01/Show Name - S01E01.mkv</i>. Couchside never changes your files unless you allow it below.</div>
      <label class="check setting-check"><input id="allow-renames" type="checkbox"> Allow file renames and deletes</label>
      <div class="hint">Lets <b>Clean up</b> on a movie's page rename the file to <i>Title (Year)\\Title (Year) 1080p.mkv</i> and move the images and subtitles in its folder to Couchside's <i>removed</i> folder, and lets <b>Delete</b> send a movie and its folder to the Recycle Bin. Other movies' files are left alone, and you always see every change first.</div>
    </section>
    <section class="panel"><h2>Metadata (TMDB)</h2>
      <div class="status-line" id="meta-status"></div>
      <form id="meta-form" class="form-row meta">
        <div><label for="tmdb-key">TMDB API key or Read Access Token</label><input id="tmdb-key" type="password" autocomplete="off" spellcheck="false"></div>
        <div><label for="meta-lang">Language</label><select id="meta-lang"></select></div>
        <button class="btn primary" type="submit">Save</button>
      </form>
      <div class="error" id="meta-err"></div>
      <div class="hint">Couchside matches each movie's file name to The Movie Database and downloads its poster, description, cast and more.
        Get a free key at <a href="https://www.themoviedb.org/settings/api" target="_blank" rel="noopener noreferrer">themoviedb.org/settings/api</a>.
        <span id="meta-remove-wrap" hidden> · <button class="link inline" type="button" id="meta-remove">Remove saved key</button></span></div>
      <div class="hint tmdb-credit">This product uses the TMDB API but is not endorsed or certified by TMDB.</div>
    </section>
    <section class="panel"><h2>Scanning</h2>
      <div class="status-line" id="scan-status"></div>
      <div class="actions">
        <button class="btn" type="button" id="scan-now">Scan all libraries now</button>
        <button class="btn" type="button" id="refresh-meta">Refresh all metadata</button>
      </div>
      <div class="hint" id="features"></div>
    </section>
    <section class="panel"><h2>Users</h2>
      <div class="list" id="users"></div>
      <form id="user-form" class="form-row users">
        <div><label for="new-user">Username</label><input id="new-user" type="text" autocomplete="off" required></div>
        <div><label for="new-pass">Password</label><input id="new-pass" type="password" autocomplete="new-password" required></div>
        <div><label class="check"><input id="new-admin" type="checkbox"> Administrator</label></div>
        <button class="btn primary" type="submit">Add user</button>
      </form>
      <div class="error" id="user-err"></div>
    </section>` : '';

  app.innerHTML = `<h1>Settings</h1>${adminPanels}
    <section class="panel"><h2>Your account</h2>
      <p class="hint">Signed in as <b>${esc(me.username)}</b>${me.is_admin ? ' (administrator)' : ''}. <button class="link inline" type="button" id="settings-logout">Sign out</button></p>
      <form id="pw-form" class="form-row users">
        <div><label for="pw1">New password</label><input id="pw1" type="password" autocomplete="new-password" required></div>
        <div><label for="pw2">Confirm</label><input id="pw2" type="password" autocomplete="new-password" required></div>
        <div></div>
        <button class="btn" type="submit">Change password</button>
      </form>
      <div class="error" id="pw-err"></div>
    </section>
    <p class="hint">Couchside ${esc(serverInfo ? serverInfo.version : '')}</p>`;

  app.querySelector('#settings-logout').addEventListener('click', signOut);
  app.querySelector('#pw-form').addEventListener('submit', async e => {
    e.preventDefault();
    const err = app.querySelector('#pw-err');
    const pw = app.querySelector('#pw1').value;
    if (pw !== app.querySelector('#pw2').value) { err.textContent = 'Passwords do not match.'; return; }
    try {
      await api(`/users/${me.id}/password`, { method: 'PUT', body: { password: pw } });
      toast('Password changed. Please sign in again.');
      me = null;
      location.hash = '#/login';
    } catch (ex) { err.textContent = ex.message; }
  });

  if (!me.is_admin) return;
  await Promise.all([renderLibraries(), renderUsers(), renderScanStatus(), renderMetadataSettings(), renderLibrarySettings()]);

  const renamesBox = app.querySelector('#allow-renames');
  renamesBox.addEventListener('change', async () => {
    renamesBox.disabled = true;
    try {
      const res = await api('/settings/library', { method: 'PUT', body: { allow_file_renames: renamesBox.checked } });
      renamesBox.checked = res.allow_file_renames;
      toast(res.allow_file_renames ? 'File renames and deletes allowed' : 'File renames and deletes switched off');
    } catch (ex) {
      renamesBox.checked = !renamesBox.checked;
      toast(ex.message);
    } finally { renamesBox.disabled = false; }
  });

  app.querySelector('#meta-form').addEventListener('submit', async e => {
    e.preventDefault();
    const err = app.querySelector('#meta-err');
    err.textContent = '';
    const body = { language: app.querySelector('#meta-lang').value };
    const key = app.querySelector('#tmdb-key').value.trim();
    if (key) body.tmdb_api_key = key;
    const btn = e.target.querySelector('button[type=submit]');
    btn.disabled = true;
    try {
      const res = await api('/settings/metadata', { method: 'PUT', body });
      serverInfo.tmdb = res.has_key;
      app.querySelector('#tmdb-key').value = '';
      toast(res.has_key ? 'Saved. Fetching movie details now…' : 'Saved');
      await renderMetadataSettings(res);
      renderScanStatus();
    } catch (ex) { err.textContent = ex.message; } finally { btn.disabled = false; }
  });
  app.querySelector('#meta-remove').addEventListener('click', async () => {
    if (!window.confirm('Remove the TMDB key saved in Settings?')) return;
    const res = await api('/settings/metadata', { method: 'PUT', body: { tmdb_api_key: '' } });
    serverInfo.tmdb = res.has_key;
    toast('Key removed');
    renderMetadataSettings(res);
  });

  app.querySelector('#lib-form').addEventListener('submit', async e => {
    e.preventDefault();
    const err = app.querySelector('#lib-err');
    err.textContent = '';
    const body = {
      name: app.querySelector('#lib-name').value,
      kind: app.querySelector('#lib-kind').value,
      path: app.querySelector('#lib-path').value,
    };
    try {
      await api('/libraries', { method: 'POST', body });
      e.target.reset();
      toast('Library added. Scanning now…');
      await renderLibraries();
      await renderScanStatus();
    } catch (ex) { err.textContent = ex.message; }
  });

  app.querySelector('#scan-now').addEventListener('click', async () => {
    const res = await api('/scan', { method: 'POST' });
    toast(res.started ? 'Scan started' : 'A scan is already running');
    renderScanStatus();
  });
  app.querySelector('#refresh-meta').addEventListener('click', async () => {
    await api('/metadata/refresh', { method: 'POST' });
    toast('Refreshing metadata…');
    renderScanStatus();
  });

  app.querySelector('#user-form').addEventListener('submit', async e => {
    e.preventDefault();
    const err = app.querySelector('#user-err');
    err.textContent = '';
    try {
      await api('/users', { method: 'POST', body: {
        username: app.querySelector('#new-user').value,
        password: app.querySelector('#new-pass').value,
        is_admin: app.querySelector('#new-admin').checked,
      } });
      e.target.reset();
      toast('User added');
      renderUsers();
    } catch (ex) { err.textContent = ex.message; }
  });

  // Poll the scan status while the settings page is open.
  const timer = setInterval(() => { renderScanStatus().catch(() => {}); }, 3000);
  cleanup = () => clearInterval(timer);
}

const LANGUAGES = [
  ['en-US', 'English (US)'], ['en-AU', 'English (Australia)'], ['en-GB', 'English (UK)'], ['en-CA', 'English (Canada)'],
  ['en-NZ', 'English (New Zealand)'], ['fr-FR', 'Français'], ['de-DE', 'Deutsch'], ['es-ES', 'Español (España)'],
  ['es-MX', 'Español (México)'], ['it-IT', 'Italiano'], ['nl-NL', 'Nederlands'], ['pt-BR', 'Português (Brasil)'],
  ['sv-SE', 'Svenska'], ['pl-PL', 'Polski'], ['ja-JP', '日本語'], ['ko-KR', '한국어'], ['zh-CN', '中文 (简体)'],
];

async function renderMetadataSettings(current) {
  const m = current || await api('/settings/metadata');
  const status = app.querySelector('#meta-status');
  if (!status) return;
  const source = m.key_source === 'config' ? ' (from config.toml)' : '';
  status.innerHTML = m.has_key
    ? `<span class="pill ok">Connected</span><span class="hint">Key ${esc(m.key_hint || '')}${source}</span>`
    : '<span class="pill warn">Not set up</span><span class="hint">Movies are shown using their file names only.</span>';
  const keyInput = app.querySelector('#tmdb-key');
  keyInput.placeholder = m.has_key ? 'Leave blank to keep the current key' : 'Paste your key here';
  const select = app.querySelector('#meta-lang');
  const options = LANGUAGES.some(([code]) => code === m.language) ? LANGUAGES : [[m.language, m.language], ...LANGUAGES];
  select.innerHTML = options.map(([code, name]) => `<option value="${esc(code)}">${esc(name)}</option>`).join('');
  select.value = m.language;
  app.querySelector('#meta-remove-wrap').hidden = m.key_source !== 'app';
}

async function renderLibrarySettings() {
  const s = await api('/settings/library');
  const box = app.querySelector('#allow-renames');
  if (box) box.checked = s.allow_file_renames;
}

async function renderLibraries() {
  const libs = await api('/libraries');
  const el = app.querySelector('#libs');
  if (!el) return;
  el.innerHTML = libs.length ? libs.map(l => `<div class="list-item">
      <div class="grow"><div><b>${esc(l.name)}</b> <span class="pill">${l.kind === 'shows' ? 'TV Shows' : 'Movies'}</span></div>
      <div class="muted" title="${esc(l.path)}">${esc(l.path)}</div></div>
      <button class="btn small danger" type="button" data-del-lib="${l.id}" data-name="${esc(l.name)}">Remove</button>
    </div>`).join('') : '<div class="hint">No libraries yet.</div>';
  el.querySelectorAll('[data-del-lib]').forEach(btn => btn.addEventListener('click', async () => {
    if (!window.confirm(`Remove the library "${btn.dataset.name}"? Your files are not deleted.`)) return;
    await api(`/libraries/${btn.dataset.delLib}`, { method: 'DELETE' });
    toast('Library removed');
    renderLibraries();
  }));
}

async function renderUsers() {
  const users = await api('/users');
  const el = app.querySelector('#users');
  if (!el) return;
  el.innerHTML = users.map(u => `<div class="list-item">
      <div class="grow"><b>${esc(u.username)}</b> ${u.is_admin ? '<span class="pill">Admin</span>' : ''}</div>
      <button class="btn small" type="button" data-pw="${u.id}" data-name="${esc(u.username)}">Set password</button>
      ${u.id === me.id ? '' : `<button class="btn small danger" type="button" data-del-user="${u.id}" data-name="${esc(u.username)}">Delete</button>`}
    </div>`).join('');
  el.querySelectorAll('[data-del-user]').forEach(btn => btn.addEventListener('click', async () => {
    if (!window.confirm(`Delete the user "${btn.dataset.name}"?`)) return;
    try {
      await api(`/users/${btn.dataset.delUser}`, { method: 'DELETE' });
      toast('User deleted');
      renderUsers();
    } catch (ex) { toast(ex.message); }
  }));
  el.querySelectorAll('[data-pw]').forEach(btn => btn.addEventListener('click', async () => {
    const pw = window.prompt(`New password for ${btn.dataset.name}:`);
    if (!pw) return;
    try {
      await api(`/users/${btn.dataset.pw}/password`, { method: 'PUT', body: { password: pw } });
      toast('Password updated');
      if (Number(btn.dataset.pw) === me.id) { me = null; location.hash = '#/login'; }
    } catch (ex) { toast(ex.message); }
  }));
}

async function renderScanStatus() {
  const s = await api('/scan');
  if (s.running) watchScan();
  const el = app.querySelector('#scan-status');
  if (!el) return;
  el.innerHTML = `<span class="dot ${s.running ? 'on' : ''}"></span>
    <b>${esc(s.running ? s.phase : 'Idle')}</b>
    <span class="hint">Last finished: ${esc(fmtDate(s.last_finished))}</span>
    ${s.errors.map(e => `<div class="error" style="flex-basis:100%">${esc(e)}</div>`).join('')}`;
  const features = app.querySelector('#features');
  if (features && serverInfo) {
    features.innerHTML = `
      <span class="pill ${serverInfo.ffprobe ? 'ok' : 'warn'}">ffprobe ${serverInfo.ffprobe ? 'found' : 'not found'}</span>
      <span class="pill ${serverInfo.tmdb ? 'ok' : 'warn'}">TMDB ${serverInfo.tmdb ? 'enabled' : 'not configured'}</span>
      <span class="pill ${serverInfo.video_encoder === 'gpu' || serverInfo.video_encoder === 'cpu' ? 'ok' : 'warn'}">${esc(VIDEO_ENCODERS[serverInfo.video_encoder] || 'Video conversion unknown')}</span>
      ${serverInfo.video_encoder === 'gpu_failed' ? '<div>The NVIDIA card failed its test at startup, so video is converted on the processor (slower). See the log for details.</div>' : ''}
      ${serverInfo.ffprobe ? '' : '<div>Install FFmpeg (or put ffprobe.exe next to couchside.exe) to see durations and codecs.</div>'}
      ${serverInfo.tmdb ? '' : '<div>Add a free TMDB key in the Metadata section above for posters, titles and descriptions.</div>'}`;
  }
}

// ---------- Router ----------

const routes = [
  [/^\/$/, () => homePage()],
  [/^\/movies$/, (m, p) => moviesPage(p)],
  [/^\/shows$/, () => showsPage()],
  [/^\/show\/(\d+)$/, m => showPage(m[1])],
  [/^\/item\/(\d+)$/, m => itemPage(m[1])],
  [/^\/play\/(\d+)$/, (m, p) => playerPage(m[1], p)],
  [/^\/search$/, (m, p) => searchPage(p)],
  [/^\/settings$/, () => settingsPage()],
];

async function route() {
  if (cleanup) { try { cleanup(); } catch (_) { /* ignore */ } cleanup = null; }
  closeDialog();
  const hash = location.hash.replace(/^#/, '') || '/';
  const [path, query] = hash.split('?');
  const params = new URLSearchParams(query || '');

  try {
    if (!serverInfo) {
      splash.set(0.05, 'Connecting…');
      serverInfo = await loadServerInfo();
    }
    if (serverInfo.needs_setup) { setupPage(); splash.finish(); return; }
    if (path === '/login') { loginPage(); splash.finish(); return; }
    if (!me) {
      splash.set(0.5, 'Signing in…');
      me = await api('/auth/me');
    }
  } catch (err) {
    if (err.status !== 401) { showError(err); splash.finish(); }
    return;
  }

  setChrome(true);
  renderMe();
  document.body.classList.remove('has-spotlight');
  window.scrollTo(0, 0);
  for (const [re, handler] of routes) {
    const m = path.match(re);
    if (m) {
      app.innerHTML = '<div class="loading">Loading…</div>';
      splash.set(0.75, 'Loading your library…');
      try { await handler(m, params); } catch (err) { showError(err); }
      splash.finish();
      watchScan();
      return;
    }
  }
  app.innerHTML = '<div class="empty"><h2>Page not found</h2><p><a class="btn" href="#/">Go home</a></p></div>';
  splash.finish();
}

document.getElementById('search-form').addEventListener('submit', e => {
  e.preventDefault();
  const q = document.getElementById('search-input').value.trim();
  if (q) location.hash = `#/search?q=${encodeURIComponent(q)}`;
});

async function signOut() {
  await api('/auth/logout', { method: 'POST' }).catch(() => {});
  me = null;
  location.hash = '#/login';
}

document.getElementById('logout').addEventListener('click', signOut);

window.addEventListener('hashchange', route);
route();
