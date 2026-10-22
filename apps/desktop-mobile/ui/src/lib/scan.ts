// QR scanning for joining a library from a phone (M4-07). Only the mobile shell has a scanner.
export const canScan = () => '__TAURI_INTERNALS__' in window && /Android|iPhone|iPad/i.test(navigator.userAgent);

/** Opens the camera and resolves with the first QR code's text, or null if cancelled. */
export async function scanQr(): Promise<string | null> {
  const { scan, Format, checkPermissions, requestPermissions } = await import('@tauri-apps/plugin-barcode-scanner');
  let perm = await checkPermissions();
  if (perm !== 'granted') perm = await requestPermissions();
  if (perm !== 'granted') throw new Error('Camera access is off. Allow it in system settings, or paste the code instead.');
  try {
    return (await scan({ windowed: false, formats: [Format.QRCode] })).content;
  } catch {
    return null;
  }
}
