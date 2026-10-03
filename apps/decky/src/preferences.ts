import type { Filter } from "@steam-patch/ui";

// Opening a Steam menu can unmount QAM content before its selection callback.
// Keep list preferences outside that component and notify the new mount too.
let preferences: { filter: Filter; query: string } = { filter: "matched", query: "" };
const listeners = new Set<() => void>();

export function getListPreferences() { return preferences; }
export function subscribeListPreferences(listener: () => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
export function setListPreferences(update: Partial<typeof preferences>) {
  preferences = { ...preferences, ...update };
  listeners.forEach(listener => listener());
}
