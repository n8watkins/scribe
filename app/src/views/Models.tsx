import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  CheckCircle2,
  Download,
  FolderOpen,
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

  return (
    <section className="view-grid">
      <article className="panel-card span-2">
        <div className="section-heading compact">
          <h2>Whisper models</h2>
          <button
            className="secondary-button"
            disabled={modelsLoading}
            onClick={() => void loadModels()}
            type="button"
          >
            <RefreshCw aria-hidden="true" size={15} />
            Refresh
          </button>
        </div>
        {modelsError ? (
          <InlineError message={modelsError} onRetry={loadModels} />
        ) : null}
        {modelsLoading ? (
          <div className="pending-panel">
            <RefreshCw aria-hidden="true" size={16} />
            <span>Loading model catalog...</span>
          </div>
        ) : null}
        {!modelsLoading && models.length === 0 ? (
          <EmptyState message="No Whisper models are available from the local catalog." />
        ) : null}
        <div className="model-table">
          <div className="model-table-header" aria-hidden="true">
            <span>Model</span>
            <span>Size</span>
            <span>Status</span>
            <span>Action</span>
          </div>
          {models.map((model) => {
            const progress = progressByModel[model.id];
            const status = progress?.status ?? model.status;
            const percent = progressPercent(model, progress);
            const isSelected = model.selected || model.id === settings.selectedModelId;
            const isDownloaded =
              status === "downloaded" ||
              status === "selected" ||
              status === "loaded";
            const isDownloading = status === "downloading";
            const isBusy = busyModelId === model.id;
            const isManagedDownload = model.source === "app_data";
            return (
              <div className="model-row" key={model.id}>
                <div>
                  <strong>{model.name}</strong>
                  <span>
                    {model.filename}
                    {model.source === "external_cache"
                      ? " - external model dir"
                      : ""}
                  </span>
                  <div className="progress-track">
                    <div style={{ width: `${percent}%` }} />
                  </div>
                </div>
                <span>{model.diskSizeLabel}</span>
                <span className={modelStatusClass(status, isSelected)}>
                  {isSelected ? "Selected" : modelStatusLabel(status)}
                </span>
                <div className="row-actions">
                  {!isDownloaded && !isDownloading && status !== "failed" ? (
                    <button
                      className="secondary-button"
                      disabled={isBusy}
                      onClick={() =>
                        void runModelAction(model.id, () => downloadModel(model.id))
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
                  {isDownloaded ? (
                    <button
                      className="secondary-button"
                      disabled={isBusy || isSelected}
                      onClick={() =>
                        void runModelAction(model.id, () => selectModel(model.id))
                      }
                      type="button"
                    >
                      <CheckCircle2 aria-hidden="true" size={15} />
                      Select
                    </button>
                  ) : null}
                  {isDownloaded && isManagedDownload ? (
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
      </article>

      <article className="panel-card">
        <div className="section-heading compact">
          <h2>Default model</h2>
        </div>
        <strong className="standout">
          {settings.selectedModelId ?? "No model selected"}
        </strong>
        <p className="muted">
          Scribe uses its own app data models first, then compatible
          files from SCRIBE_MODEL_DIR when set.
        </p>
      </article>

      <article className="panel-card">
        <div className="section-heading compact">
          <h2>Storage</h2>
        </div>
        <div className="button-row">
          <button
            className="secondary-button"
            onClick={() => {
              void openModelsFolder().catch((error) =>
                setModelsError(commandErrorMessage(error)),
              );
            }}
            type="button"
          >
            <FolderOpen aria-hidden="true" size={15} />
            Open models folder
          </button>
        </div>
      </article>
    </section>
  );
}
