// Plain words for what a device does (U1-13); the role name itself is only a small label.
export const ROLE_DOES: Record<string, string> = {
  keeper: 'Keeps every original',
  companion: 'Keeps recent and favorite photos',
  viewer: 'Browses only',
};

export const ROLE_HELP: Record<string, string> = {
  companion: 'Keeps previews of everything and the full-size photos you pin, favorite or took recently; the rest lives on your keepers. For phones, tablets and laptops.',
  keeper: 'Keeps every original: the backstop. Best for an always-on computer or a small box at home.',
  viewer: 'Sees previews only and cannot add or change photos. Good for a shared tablet or a TV.',
};

export const roleLabel = (r: string) => r.charAt(0).toUpperCase() + r.slice(1);
export const roleDoes = (r: string) => ROLE_DOES[r] ?? r;
