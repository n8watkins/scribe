import type { ModelInfo, Transcript } from "../backend";

/** What deleting one history entry removes, for its confirm dialog. */
export function deleteTranscriptMessage(item: Transcript | null): string {
  const what = item?.isNote ? "note" : "transcript";
  const removed = item?.audioPath
    ? "Its text and audio clip are removed"
    : "Its text is removed";
  return `${removed} from this device. This ${what} can't be recovered.`;
}

/** What deleting a downloaded model means, for its confirm dialog. */
export function deleteModelMessage(
  model: Pick<ModelInfo, "id" | "diskSizeLabel"> | null,
  selectedModelId: string | null,
): string {
  const frees = model?.diskSizeLabel
    ? `This frees ${model.diskSizeLabel} on this device.`
    : "This frees its space on this device.";
  const inUse =
    model && model.id === selectedModelId
      ? " It's the model Scribe uses now, so pick another one before dictating."
      : "";
  return `${frees} You can download it again any time.${inUse}`;
}
