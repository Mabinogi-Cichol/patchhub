// SidebarNavigation uses prefix matching, so settings panels need sibling paths.
export const settingsRoutes = {
  sources: "/patchhub/sources/subscriptions",
  cache: "/patchhub/sources/cache",
} as const;

export function settingsPage(panel?: string): string {
  return panel === "cache" ? settingsRoutes.cache : settingsRoutes.sources;
}

// instance_id contains '/', and the Steam router only partially decodes a
// single URL segment, so a raw or encodeURIComponent value is not restored
// byte-for-byte. URL-safe base64 keeps the segment opaque to routing.
export function encodeInstance(instance: string): string {
  let binary = "";
  for (const byte of new TextEncoder().encode(instance)) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

export function decodeInstance(value: string): string {
  try {
    const binary = atob(value.replaceAll("-", "+").replaceAll("_", "/"));
    return new TextDecoder().decode(Uint8Array.from(binary, char => char.charCodeAt(0)));
  } catch {
    // Legacy encodeURIComponent routes and junk fall back to the raw segment.
    return value;
  }
}
