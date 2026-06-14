import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Check,
  ChevronDown,
  Download,
  FolderOpen,
  HardDrive,
  RefreshCw,
  Square,
  Trash2,
} from "lucide-react";
import {
  cancelModelDownload,
  commandErrorMessage,
  deleteModel,
  downloadModel,
  listModels,
  openModelsFolder,
  retryModelDownload,
  selectModel,
  type AppSettings,
  type ModelDownloadProgress,
  type ModelInfo,
} from "../backend";
import type { ViewActions } from "../types";
import {
  diskUsedLabel,
  isModelDownloaded,
  modelStatusClass,
  modelStatusLabel,
  progressPercent,
} from "../lib/format";
import { EmptyState, InlineError } from "../components/feedback";
import { IconButton } from "../components/primitives";

export function ModelsView({
  actions,
  settings,
}: {
  actions: ViewActions;
  settings: AppSettings;
}) {
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [progressByModel, setProgressByModel] = useState<
    Record<string, ModelDownloadProgress>
  >({});
  const [modelsLoading, setModelsLoading] = useState(true);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [busyModelId, setBusyModelId] = useState<string | null>(null);
  const [catalogOpen, setCatalogOpen] = useState(false);
  // Auto-expand the catalog once on first load if nothing is downloaded yet, so
  // a fresh user immediately sees models to download. Manual toggles win after.
  const autoExpandedRef = useRef(false);

  const loadModels = useCallback(async () => {
    setModelsLoading(true);
    setModelsError(null);

    try {
      setModels(await listModels());
    } catch (error) {
      setModelsError(commandErrorMessage(error));
    } finally {
      setModelsLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadModels();
  }, [loadModels, settings.selectedModelId]);

  useEffect(() => {
    let disposed = false;
    let unlistenProgress: (() => void) | null = null;

    const setup = async () => {
      const unlisten = await listen<ModelDownloadProgress>(
        "model://download-progress",
        (event) => {
          setProgressByModel((current) => ({
            ...current,
            [event.payload.modelId]: event.payload,
          }));

          setModels((current) =>
            current.map((model) =>
              model.id === event.payload.modelId
                ? { ...model, status: event.payload.status }
                : model,
            ),
          );

          if (
            event.payload.status === "downloaded" ||
            event.payload.status === "selected" ||
            event.payload.status === "failed"
          ) {
            void loadModels();
          }
        },
      );
      unlistenProgress = unlisten;

      if (disposed) {
        unlisten();
      }
    };

    void setup();

    return () => {
      disposed = true;
      unlistenProgress?.();
    };
  }, [loadModels]);

  const runModelAction = useCallback(
    async (modelId: string, action: () => Promise<unknown>) => {
      setBusyModelId(modelId);
      setModelsError(null);

      try {
        await action();
        // Drop any live progress entry for this model so a stale terminal
        // status (e.g. a prior "downloaded") can't shadow the fresh
        // `model.status` via `effectiveStatus` — e.g. a just-deleted model must
        // fall back to `not_downloaded` rather than keep showing Delete.
        setProgressByModel((current) => {
          if (!(modelId in current)) {
            return current;
          }
          const next = { ...current };
          delete next[modelId];
          return next;
        });
        await loadModels();
        await actions.refresh();
      } catch (error) {
        setModelsError(commandErrorMessage(error));
      } finally {
        setBusyModelId(null);
      }
    },
    [actions, loadModels],
  );

  // The effective download-state per model, merged with any live progress event
  // so the row reflects an in-flight download before listModels refreshes.
  const effectiveStatus = useCallback(
    (model: ModelInfo) => progressByModel[model.id]?.status ?? model.status,
    [progressByModel],
  );

  // Selection is the single source of truth: prefer the backend `selected`
  // flag, falling back to the persisted setting before the list loads.
  const isSelectedModel = useCallback(
    (model: ModelInfo) =>
      model.selected || model.id === settings.selectedModelId,
    [settings.selectedModelId],
  );

  const selectedModel = useMemo(
    () => models.find((model) => isSelectedModel(model)) ?? null,
    [models, isSelectedModel],
  );

  // Downloaded models first (then by catalog order) so the most relevant
  // entries sit at the top of the scrollable catalog.
  const orderedModels = useMemo(() => {
    return models
      .map((model, index) => ({ model, index }))
      .sort((a, b) => {
        const aDownloaded = isModelDownloaded(effectiveStatus(a.model));
        const bDownloaded = isModelDownloaded(effectiveStatus(b.model));
        if (aDownloaded !== bDownloaded) {
          return aDownloaded ? -1 : 1;
        }
        return a.index - b.index;
      })
      .map((entry) => entry.model);
  }, [models, effectiveStatus]);

  const downloadedCount = useMemo(
    () => models.filter((model) => isModelDownloaded(effectiveStatus(model))).length,
    [models, effectiveStatus],
  );

  // Total disk used by downloaded models (sum of known sizeBytes).
  const totalDiskBytes = useMemo(
    () =>
      models.reduce(
        (sum, model) =>
          isModelDownloaded(effectiveStatus(model)) && model.sizeBytes
            ? sum + model.sizeBytes
            : sum,
        0,
      ),
    [models, effectiveStatus],
  );

  useEffect(() => {
    if (autoExpandedRef.current || modelsLoading || models.length === 0) {
      return;
    }
    autoExpandedRef.current = true;
    if (downloadedCount === 0) {
      setCatalogOpen(true);
    }
  }, [modelsLoading, models.length, downloadedCount]);

  const openFolder = useCallback(() => {
    void openModelsFolder().catch((error) =>
      setModelsError(commandErrorMessage(error)),
    );
  }, []);

  return (
    <section className="view-grid">
      <article className="panel-card span-2">
        <div className="section-heading compact">
          <div className="row-actions">
            <button
              className="secondary-button"
              disabled={modelsLoading}
              onClick={() => void loadModels()}
              type="button"
            >
              <RefreshCw aria-hidden="true" size={15} />
              Refresh
            </button>
            <button
              className="secondary-button"
              onClick={openFolder}
              type="button"
            >
              <FolderOpen aria-hidden="true" size={15} />
              Open models folder
            </button>
          </div>
        </div>
        {modelsError ? (
          <InlineError message={modelsError} onRetry={loadModels} />
        ) : null}

        <div className="models-summary">
          <div className="models-summary-item">
            <span className="models-summary-label">Active model</span>
            <strong className="models-summary-value" title={selectedModel?.name}>
              {selectedModel?.name ??
                settings.selectedModelId ??
                "No model selected"}
            </strong>
            <span className="muted">
              {selectedModel
                ? isModelDownloaded(effectiveStatus(selectedModel))
                  ? "Ready for dictation"
                  : "Download this model to use it"
                : "Select a downloaded model below"}
            </span>
          </div>
          <div className="models-summary-item">
            <span className="models-summary-label">Storage used</span>
            <strong className="models-summary-value">
              {diskUsedLabel(totalDiskBytes)}
            </strong>
            <span className="muted">
              {downloadedCount === 1
                ? "1 model downloaded"
                : `${downloadedCount} models downloaded`}
            </span>
          </div>
        </div>

        <p className="muted models-summary-note">
          Scribe uses its own app data models first, then compatible files from
          SCRIBE_MODEL_DIR when set.
        </p>
      </article>

      <article className="panel-card span-2">
        <button
          aria-expanded={catalogOpen}
          className="accordion-toggle"
          onClick={() => setCatalogOpen((open) => !open)}
          type="button"
        >
          <span className="accordion-toggle-label">
            <HardDrive aria-hidden="true" size={15} />
            Browse models
          </span>
          <span className="accordion-toggle-meta">
            <span className="muted">
              {downloadedCount} of {models.length} downloaded
            </span>
            <ChevronDown
              aria-hidden="true"
              className={catalogOpen ? "accordion-chevron is-open" : "accordion-chevron"}
              size={16}
            />
          </span>
        </button>

        {catalogOpen ? (
          <>
            {modelsLoading ? (
              <div className="pending-panel">
                <RefreshCw aria-hidden="true" size={16} />
                <span>Loading model catalog...</span>
              </div>
            ) : null}
            {!modelsLoading && models.length === 0 ? (
              <EmptyState message="No Whisper models are available from the local catalog." />
            ) : null}
            {models.length > 0 ? (
              <div className="model-scroll">
                <div className="model-table">
                  {orderedModels.map((model) => {
                    const progress = progressByModel[model.id];
                    const status = effectiveStatus(model);
                    const percent = progressPercent(model, progress);
                    const selected = isSelectedModel(model);
                    const downloaded = isModelDownloaded(status);
                    const isDownloading = status === "downloading";
                    const isBusy = busyModelId === model.id;
                    const isManagedDownload = model.source === "app_data";
                    const statusLabel = modelStatusLabel(status);
                    const statusClass = modelStatusClass(status);
                    // Download-state pill is interesting on its own when the
                    // model is not in use, or when the state is actionable
                    // (downloading / failed / update available) even if it is.
                    const actionableState =
                      status === "downloading" ||
                      status === "failed" ||
                      status === "update_available";
                    const showStatusPill =
                      statusLabel !== "" && (!selected || actionableState);
                    const showRestingState = !selected && statusLabel === "";
                    return (
                      <div
                        className={selected ? "model-row is-selected" : "model-row"}
                        key={model.id}
                      >
                        <label className="model-select" title="Use this model">
                          <input
                            checked={selected}
                            disabled={!downloaded || isBusy || selected}
                            name="selected-model"
                            onChange={() =>
                              void runModelAction(model.id, () =>
                                selectModel(model.id),
                              )
                            }
                            type="radio"
                          />
                        </label>
                        <div className="model-row-main">
                          <strong>{model.name}</strong>
                          <span className="model-row-sub">
                            {model.diskSizeLabel}
                            {" · "}
                            {model.multilingual
                              ? "Multilingual"
                              : "English-only"}
                            {" · "}
                            {model.filename}
                            {model.source === "external_cache"
                              ? " · external model dir"
                              : ""}
                          </span>
                          {isDownloading ? (
                            <div className="progress-track">
                              <div style={{ width: `${percent}%` }} />
                            </div>
                          ) : null}
                        </div>
                        <div className="model-row-status">
                          {selected ? (
                            <span className="pill selected">
                              <Check aria-hidden="true" size={11} />
                              Active
                            </span>
                          ) : null}
                          {/* Download-state pill. When the model is active, a
                              plain "Downloaded"/"Loaded" is implied by "Active",
                              so the pill only appears for actionable states. */}
                          {showStatusPill ? (
                            <span className={statusClass}>{statusLabel}</span>
                          ) : null}
                          {showRestingState ? (
                            <span className="model-row-resting">Not downloaded</span>
                          ) : null}
                        </div>
                        <div className="row-actions">
                          {!downloaded && !isDownloading && status !== "failed" ? (
                            <button
                              className="secondary-button"
                              disabled={isBusy}
                              onClick={() =>
                                void runModelAction(model.id, () =>
                                  downloadModel(model.id),
                                )
                              }
                              type="button"
                            >
                              <Download aria-hidden="true" size={15} />
                              {isBusy ? "Starting..." : "Download"}
                            </button>
                          ) : null}
                          {status === "failed" ? (
                            <button
                              className="secondary-button"
                              disabled={isBusy}
                              onClick={() =>
                                void runModelAction(model.id, () =>
                                  retryModelDownload(model.id),
                                )
                              }
                              type="button"
                            >
                              <RefreshCw aria-hidden="true" size={15} />
                              Retry
                            </button>
                          ) : null}
                          {isDownloading ? (
                            <button
                              className="secondary-button"
                              disabled={isBusy}
                              onClick={() =>
                                void runModelAction(model.id, () =>
                                  cancelModelDownload(model.id),
                                )
                              }
                              type="button"
                            >
                              <Square aria-hidden="true" size={15} />
                              Cancel
                            </button>
                          ) : null}
                          {downloaded && isManagedDownload ? (
                            <IconButton
                              danger
                              disabled={isBusy || isDownloading}
                              label="Delete model"
                              onClick={() => {
                                if (
                                  window.confirm(
                                    `Delete ${model.name} from local model storage?`,
                                  )
                                ) {
                                  void runModelAction(model.id, () =>
                                    deleteModel(model.id),
                                  );
                                }
                              }}
                            >
                              <Trash2 aria-hidden="true" size={15} />
                            </IconButton>
                          ) : null}
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>
            ) : null}
          </>
        ) : null}
      </article>
    </section>
  );
}
