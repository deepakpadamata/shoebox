// Narrow windows (phones) get the mobile layout: tab bar, bottom sheets, safe areas.
const query = matchMedia('(max-width: 720px)');

export const viewport = $state({ mobile: query.matches });

query.addEventListener('change', (e) => {
  viewport.mobile = e.matches;
});
