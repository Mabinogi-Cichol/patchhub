import { gameDisplayName, type Install } from "@steam-patch/ui";

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

export function matchesGameQuery(item: Install, query: string): boolean {
  const search = query.trim().toLocaleLowerCase();
  return !search || [item.game.name, gamePresentation(item.game.appid, gameDisplayName(item.game)).name, String(item.game.appid)]
    .some(value => value.toLocaleLowerCase().includes(search));
}

export function transactionGameName(installs: Install[], instanceId: string): string {
  const game = installs.find(item => item.game.instance_id === instanceId)?.game;
  return game ? gamePresentation(game.appid, gameDisplayName(game)).name : instanceId;
}
