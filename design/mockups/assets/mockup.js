// Shoebox mockups — placeholder content helpers.
//
//   <div class="grid" data-photos="24" data-seed="3" data-badges="risk:2,7;sync:5"></div>
//     Fills a grid with gradient "photos". Optional badges per tile index:
//     risk → "1 copy", sync → "↻", copies → "2×". data-video / data-fav take index lists.
//     data-selected marks tiles as selected (multi-select mode); data-select shows empty circles.
//
//   <div class="tile" data-photo="5"></div>   a single placeholder photo
//   <div class="qr" data-qr="seed"></div>      a fake QR code

(function () {
  // Palettes loosely evoking real photos: sky, beach, forest, sunset, city night, snow, food, etc.
  const PALETTES = [
    ["#8ec5e8", "#e9d8b4", "#c49a6c"],
    ["#f6c58b", "#e2735a", "#523a5c"],
    ["#5f8b5a", "#a9c48a", "#2f4a2c"],
    ["#1f2a44", "#3e4f7a", "#f4c46a"],
    ["#dfe7ee", "#b6c6d4", "#6e7f8f"],
    ["#e9c9a7", "#c2865a", "#7a4b2e"],
    ["#a7d3d6", "#5aa0a8", "#2c5a66"],
    ["#f3d9dc", "#d99aa4", "#8b5b6a"],
    ["#c9b79c", "#8f7a5a", "#40362a"],
    ["#ffd76a", "#ff9f43", "#c0392b"],
    ["#b8d8b0", "#6aa67a", "#e8e1c4"],
    ["#2d2d34", "#56566a", "#d6c2a0"],
  ];

  function rng(seed) {
    let s = seed * 9301 + 49297;
    return () => ((s = (s * 9301 + 49297) % 233280) / 233280);
  }

  function photoBg(n) {
    const r = rng(n + 1);
    const p = PALETTES[n % PALETTES.length];
    const angle = Math.floor(r() * 360);
    const x = Math.floor(r() * 100), y = Math.floor(r() * 100);
    return `radial-gradient(circle at ${x}% ${y}%, ${p[2]}cc 0%, transparent 55%),` +
           `linear-gradient(${angle}deg, ${p[0]}, ${p[1]})`;
  }
  window.photoBg = photoBg;

  function parseList(s) {
    return (s || "").split(",").filter(Boolean).map(Number);
  }

  function fillGrid(el) {
    const count = Number(el.dataset.photos);
    const seed = Number(el.dataset.seed || 0);
    const badges = {};
    (el.dataset.badges || "").split(";").filter(Boolean).forEach(part => {
      const [kind, list] = part.split(":");
      parseList(list).forEach(i => (badges[i] = kind));
    });
    const videos = new Set(parseList(el.dataset.video));
    const favs = new Set(parseList(el.dataset.fav));
    const selected = new Set(parseList(el.dataset.selected));
    const selectMode = el.hasAttribute("data-select") || selected.size > 0;
    const rounded = el.hasAttribute("data-rounded");

    for (let i = 0; i < count; i++) {
      const t = document.createElement("div");
      t.className = "tile" + (rounded ? " r" : "") + (selected.has(i) ? " selected" : "");
      t.style.background = photoBg(seed * 31 + i);
      const b = badges[i];
      if (b === "risk") t.insertAdjacentHTML("beforeend", '<span class="b risk">1 copy</span>');
      if (b === "sync") t.insertAdjacentHTML("beforeend", '<span class="b sync">↻</span>');
      if (b === "copies") t.insertAdjacentHTML("beforeend", '<span class="b">2×</span>');
      if (videos.has(i)) t.insertAdjacentHTML("beforeend", '<span class="vid">▶ 0:' + (12 + i * 7) % 60 + "</span>");
      if (favs.has(i)) t.insertAdjacentHTML("beforeend", '<span class="fav">♥</span>');
      if (selectMode) t.insertAdjacentHTML("beforeend", '<span class="sel"></span>');
      el.appendChild(t);
    }
  }

  function fakeQr(el) {
    const N = 29;
    const r = rng([...el.dataset.qr].reduce((a, c) => a + c.charCodeAt(0), 7));
    let rects = "";
    const finder = (x, y) =>
      `<rect x="${x}" y="${y}" width="7" height="7" fill="#1d1c1a"/>` +
      `<rect x="${x + 1}" y="${y + 1}" width="5" height="5" fill="#fff"/>` +
      `<rect x="${x + 2}" y="${y + 2}" width="3" height="3" fill="#1d1c1a"/>`;
    const inFinder = (x, y) =>
      (x < 8 && y < 8) || (x > N - 9 && y < 8) || (x < 8 && y > N - 9);
    for (let y = 0; y < N; y++)
      for (let x = 0; x < N; x++)
        if (!inFinder(x, y) && r() > 0.52) rects += `<rect x="${x}" y="${y}" width="1" height="1" fill="#1d1c1a"/>`;
    el.innerHTML =
      `<svg viewBox="0 0 ${N} ${N}" shape-rendering="crispEdges">${rects}` +
      finder(0, 0) + finder(N - 7, 0) + finder(0, N - 7) + "</svg>";
  }

  document.querySelectorAll("[data-photos]").forEach(fillGrid);
  document.querySelectorAll("[data-photo]").forEach(el => (el.style.background = photoBg(Number(el.dataset.photo))));
  document.querySelectorAll("[data-qr]").forEach(fakeQr);
})();
