import { primaryInstallAction } from "@patchhub/ui";
import type { SourceCandidate } from "@patchhub/ui";
import type { ManualDownloadInput } from "./task-controller";
export { primaryInstallAction, installInput, actionBlockers as primaryActionBlockers } from "@patchhub/ui";
export type PrimaryInstallAction = ReturnType<typeof primaryInstallAction>;

export function manualDownloadInput(candidate: SourceCandidate | undefined, value: string): ManualDownloadInput | null {
  const input = value.trim();
  if (!candidate || !input) return null;
  try {
    const url = new URL(input);
    if (!["http:", "https:"].includes(url.protocol)) return null;
    return { source_id: candidate.source_id, candidate_id: candidate.candidate_id, url: input };
  } catch { return null; }
}

export function manualDownloadRequestKey(instance: string, input: ManualDownloadInput): string {
  return `${instance}\0${input.source_id}\0${input.candidate_id}\0overwrite\0${input.url}`;
}
