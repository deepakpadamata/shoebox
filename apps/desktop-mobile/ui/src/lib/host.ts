// Things only the native shell can do (Android: keep the screen on while backup finishes).
type Host = { setKeepAwake(on: boolean): void };

export function keepAwake(on: boolean): void {
  (window as unknown as { ShoeboxHost?: Host }).ShoeboxHost?.setKeepAwake(on);
}

export const hasKeepAwake = () => !!(window as unknown as { ShoeboxHost?: Host }).ShoeboxHost;
