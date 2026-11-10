import { PARAKEET_V3_ID } from "../backend";

/** Whether the selected model runs on Parakeet, which takes no custom words,
 * language or translate setting. */
export function isParakeetModel(modelId: string | null): boolean {
  return modelId === PARAKEET_V3_ID;
}
