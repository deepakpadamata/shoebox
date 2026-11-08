// Fills <div class="tiles" data-n="24" data-seed="3" data-sel="0,1" data-live="2" data-video="5"> with generated photo tiles.
(function () {
  const theme = new URLSearchParams(location.search).get('theme');
  if (theme === 'light' || theme === 'dark') document.documentElement.dataset.theme = theme;
  function rnd(seed) { let s = (Math.imul(seed + 1, 2654435761) ^ 0x9e3779b9) >>> 0; for (let i = 0; i < 4; i++) s = (s * 1664525 + 1013904223) >>> 0; return () => (s = (s * 1664525 + 1013904223) >>> 0) / 4294967296; }
  const palettes = [[30, 60], [190, 220], [95, 140], [10, 35], [260, 300], [40, 200], [160, 30]];
  document.querySelectorAll('.tiles[data-n]').forEach((el) => {
    const n = +el.dataset.n, r = rnd(+(el.dataset.seed || 7));
    const list = (k) => (el.dataset[k] || '').split(',').filter(Boolean).map(Number);
    const sel = list('sel'), live = list('live'), vid = list('video'), fav = list('fav');
    const selecting = el.dataset.selecting !== undefined;
    for (let i = 0; i < n; i++) {
      const p = palettes[Math.floor(r() * palettes.length)];
      const h1 = p[0] + r() * 20, h2 = p[1] + r() * 20, a = Math.floor(r() * 180);
      const l1 = 35 + r() * 30, l2 = 45 + r() * 30;
      const t = document.createElement('div');
      t.className = 'tile' + (sel.includes(i) ? ' on' : '');
      t.style.background = `linear-gradient(${a}deg, hsl(${h1} 45% ${l1}%), hsl(${h2} 50% ${l2}%))`;
      let inner = '';
      if (live.includes(i)) inner += '<span class="b bl">◎ LIVE</span>';
      if (vid.includes(i)) inner += '<span class="b bl">▶ 0:1' + (i % 10) + '</span>';
      if (fav.includes(i)) inner += '<span class="b tl">♥</span>';
      if (selecting) inner += '<span class="sel"></span>';
      t.innerHTML = inner;
      el.appendChild(t);
    }
  });
  document.querySelectorAll('.photo[data-seed]').forEach((el) => {
    const r = rnd(+el.dataset.seed); const h = r() * 360;
    el.style.background = `linear-gradient(${Math.floor(r() * 180)}deg, hsl(${h} 45% 40%), hsl(${(h + 60) % 360} 55% 62%))`;
  });
})();
