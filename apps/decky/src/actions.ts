import type { ActionBlocker, Patch } from "@steam-patch/ui";

export type PrimaryInstallAction = {
  operation: "overwrite";
  label: "install" | "overwrite";
  input: "automatic" | "local_archive" | "unavailable";
  enabled: boolean;
};

export function primaryInstallAction(patch: Patch | null | undefined, archiveSelected: boolean): PrimaryInstallAction {
  const primary = patch?.actions.primary;
  const input = primary?.input ?? "unavailable";
  const enabled = !!patch && (input === "automatic"
    ? patch.actions.automatic.available
    : input === "local_archive" ? patch.actions.local_archive.available && archiveSelected : false);
  return { operation: primary?.operation ?? "overwrite", label: primary?.label ?? "install", input, enabled };
}

/** Resolve an explicit archive request or the backend-selected automatic input. */
export function installInput(patch: Patch | null | undefined, localArchive?: string): string | null {
  if (!patch) return null;
  if (localArchive !== undefined) {
    return localArchive.trim() && patch.actions.local_archive.available ? localArchive : null;
  }
  return patch.actions.primary.input === "automatic" && patch.actions.automatic.available ? "" : null;
}

/** Show blockers for the currently offered action path without unrelated reinstall advice. */
export function primaryActionBlockers(patch: Patch | null | undefined): ActionBlocker[] {
  if (!patch) return [];
  const input = patch.actions.primary.input;
  const blockers = input === "automatic" ? patch.actions.automatic.blockers
    : input === "local_archive" ? [...patch.actions.automatic.blockers, ...patch.actions.local_archive.blockers]
      : patch.actions.overwrite.blockers;
  return blockers.filter((blocker, index, all) => all.findIndex(value => value.code === blocker.code && value.detail === blocker.detail) === index);
}