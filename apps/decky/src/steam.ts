export function gamePresentation(appid: number, fallbackName: string): { name: string; icon?: string } {
  try {
    const store = window.appStore;
    const overview = store?.GetAppOverviewByAppID(appid);
    if (!overview) return { name: fallbackName };
    // Steam resolves display_name for its current interface language already.
    const name = overview.display_name?.trim() || fallbackName;
    try {
      const icon = store.GetIconURLForApp(overview as Parameters<typeof store.GetIconURLForApp>[0]);
      return { name, icon: icon || undefined };
    } catch { return { name }; }
  } catch { return { name: fallbackName }; }
}
