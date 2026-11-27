import { useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  AlertCircle,
  ArrowDownCircle,
  Check,
  CheckCircle2,
  Clock,
  Compass,
  FolderOpen,
  Pause,
  Play,
  RotateCw,
  Trash2,
} from "lucide-react";
import { api } from "../api";
import noDownloadsSvg from "../assets/illustrations/no_downloads.svg";
import type { DownloadTask, PlaybackRequest } from "../types";

interface Props {
  onPlayLocal: (req: PlaybackRequest) => void;
  onTasksChange?: (tasks: DownloadTask[]) => void;
  onBrowseMovies?: () => void;
}

const STATUS_META: Record<
  string,
  { label: string; tone: "active" | "done" | "paused" | "error" | "queued" }
> = {
  DOWNLOADING: { label: "Downloading", tone: "active" },
  COMPLETED: { label: "Ready Offline", tone: "done" },
  PAUSED: { label: "Paused", tone: "paused" },
  FAILED: { label: "Failed", tone: "error" },
  PENDING: { label: "Queued", tone: "queued" },
};

function formatBytes(bytes: number): string {
  if (!bytes || bytes <= 0) return "0 B";
  if (bytes >= 1024 * 1024 * 1024) {
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }
  if (bytes >= 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
  if (bytes >= 1024) {
    return `${Math.round(bytes / 1024)} KB`;
  }
  return `${bytes} B`;
}

function StatusGlyph({ tone }: { tone: "active" | "done" | "paused" | "error" | "queued" }) {
  if (tone === "done") return <CheckCircle2 size={18} />;
  if (tone === "active") return <ArrowDownCircle size={18} />;
  if (tone === "paused") return <Pause size={18} />;
  if (tone === "error") return <AlertCircle size={18} />;
  return <Clock size={18} />;
}

interface GroupProps {
  label: string;
  items: DownloadTask[];
  confirmDeleteId: string | null;
  onPlayLocal: (req: PlaybackRequest) => void;
  onPause: (id: string) => void;
  onResume: (id: string) => void;
  onOpenFolder: (path: string) => void;
  onRequestDelete: (id: string | null) => void;
  onConfirmDelete: (id: string) => void;
}

function DownloadGroup({
  label,
  items,
  confirmDeleteId,
  onPlayLocal,
  onPause,
  onResume,
  onOpenFolder,
  onRequestDelete,
  onConfirmDelete,
}: GroupProps) {
  if (items.length === 0) return null;

  return (
    <section className="dl-section">
      <div className="dl-section-label">{label}</div>
      <div className="dl-group">
        {items.map((task) => {
          const pct = Math.min(100, Math.max(0, Math.round(task.progress || 0)));
          const meta = STATUS_META[task.status] ?? {
            label: task.status || "Unknown",
            tone: "queued" as const,
          };
          const showProgress =
            task.status === "DOWNLOADING" ||
            task.status === "PAUSED" ||
            task.status === "PENDING" ||
            task.status === "FAILED";
          const isConfirming = confirmDeleteId === task.id;

          return (
            <div key={task.id} className="dl-row">
              <div className={`dl-status-tile ${meta.tone}`} aria-hidden="true">
                <StatusGlyph tone={meta.tone} />
              </div>

              <div className="dl-row-body">
                <div className="dl-row-title-line">
                  <span className="dl-row-title" title={task.title}>
                    {task.title}
                  </span>
                  <span className={`dl-status-badge ${meta.tone}`}>{meta.label}</span>
                </div>

                <div className="dl-row-meta">
                  <span>
                    {formatBytes(task.downloadedBytes)}
                    {task.totalBytes > 0 ? ` / ${formatBytes(task.totalBytes)}` : ""}
                  </span>
                  {showProgress && <span>{pct}%</span>}
                  {task.speed && <span>{task.speed}</span>}
                </div>

                {showProgress && (
                  <div
                    className="dl-progress-track"
                    role="progressbar"
                    aria-valuenow={pct}
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-label={`${task.title} download progress`}
                  >
                    <div
                      className={`dl-progress-fill ${meta.tone}`}
                      style={{ width: `${pct}%` }}
                    />
                  </div>
                )}
              </div>

              <div className="dl-actions">
                {task.status === "COMPLETED" && (
                  <button
                    type="button"
                    className="btn btn-primary dl-action-btn"
                    onClick={() =>
                      onPlayLocal({
                        title: task.title,
                        link: task.savePath,
                        streamUrl: task.savePath,
                      })
                    }
                  >
                    <Play size={14} fill="currentColor" /> Play
                  </button>
                )}

                {(task.status === "DOWNLOADING" || task.status === "PENDING") && (
                  <button
                    type="button"
                    className="btn dl-action-btn"
                    onClick={() => onPause(task.id)}
                  >
                    <Pause size={14} /> Pause
                  </button>
                )}

                {(task.status === "PAUSED" || task.status === "FAILED") && (
                  <button
                    type="button"
                    className="btn btn-primary dl-action-btn"
                    onClick={() => onResume(task.id)}
                  >
                    <RotateCw size={14} /> {task.status === "FAILED" ? "Retry" : "Resume"}
                  </button>
                )}

                <button
                  type="button"
                  className="btn dl-icon-btn"
                  onClick={() => onOpenFolder(task.savePath)}
                  title="Open Containing Folder"
                  aria-label={`Open folder for ${task.title}`}
                >
                  <FolderOpen size={15} />
                </button>

                {isConfirming ? (
                  <div className="prov-delete-confirm">
                    <button
                      type="button"
                      className="btn prov-confirm-danger"
                      onClick={() => onConfirmDelete(task.id)}
                    >
                      <Check size={13} /> Delete
                    </button>
                    <button
                      type="button"
                      className="btn prov-compact-btn"
                      onClick={() => onRequestDelete(null)}
                    >
                      Cancel
                    </button>
                  </div>
                ) : (
                  <button
                    type="button"
                    className="btn dl-icon-btn danger"
                    onClick={() => onRequestDelete(task.id)}
                    title="Delete Download"
                    aria-label={`Delete ${task.title}`}
                  >
                    <Trash2 size={15} />
                  </button>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}

/**
 * Apple-style offline Downloads manager with live progress telemetry and dark-adapted empty artwork.
 */
export function DownloadsView({ onPlayLocal, onTasksChange, onBrowseMovies }: Props) {
  const [tasks, setTasks] = useState<DownloadTask[]>([]);
  const [loading, setLoading] = useState(true);
  const [defaultDir, setDefaultDir] = useState("");
  const [error, setError] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  useEffect(() => {
    let mounted = true;
    void api
      .listDownloads()
      .then((list) => {
        if (!mounted) return;
        setTasks(list);
        onTasksChange?.(list);
      })
      .catch((err) => {
        if (mounted) setError(String(err));
      })
      .finally(() => {
        if (mounted) setLoading(false);
      });

    void api
      .getDefaultDownloadDir()
      .then((dir) => {
        if (mounted) setDefaultDir(dir);
      })
      .catch(() => {});

    let unlisten: (() => void) | undefined;
    void listen<DownloadTask>("downloads://progress", (e) => {
      const updated = e.payload;
      setTasks((prev) => {
        const exists = prev.some((t) => t.id === updated.id);
        const next = exists
          ? prev.map((t) => (t.id === updated.id ? updated : t))
          : [updated, ...prev];
        onTasksChange?.(next);
        return next;
      });
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      mounted = false;
      unlisten?.();
    };
  }, []);

  const activeTasks = useMemo(
    () => tasks.filter((t) => t.status !== "COMPLETED"),
    [tasks]
  );
  const completedTasks = useMemo(
    () => tasks.filter((t) => t.status === "COMPLETED"),
    [tasks]
  );

  async function handlePause(id: string) {
    setError("");
    const next = await api.pauseDownload(id);
    setTasks(next);
    onTasksChange?.(next);
  }

  async function handleResume(id: string) {
    setError("");
    const next = await api.resumeDownload(id);
    setTasks(next);
    onTasksChange?.(next);
  }

  async function handleConfirmDelete(id: string) {
    setError("");
    setConfirmDeleteId(null);
    const next = await api.deleteDownload(id, true);
    setTasks(next);
    onTasksChange?.(next);
  }

  async function handleOpenFolder(path: string) {
    setError("");
    try {
      await api.openDownloadFolder(path);
    } catch (err) {
      setError(String(err));
    }
  }

  const summaryText =
    tasks.length === 0
      ? "0 Files"
      : activeTasks.length > 0 && completedTasks.length > 0
        ? `${activeTasks.length} Active • ${completedTasks.length} Saved`
        : tasks.length === 1
          ? "1 File"
          : `${tasks.length} Files`;

  return (
    <div className="dl-screen">
      <header className="dl-header">
        <div>
          <h1 className="hist-title">Downloads</h1>
          {defaultDir && (
            <div className="dl-subtitle" title={defaultDir}>
              <FolderOpen size={14} aria-hidden="true" />
              <span>{defaultDir}</span>
            </div>
          )}
        </div>

        <div className="dl-header-actions">
          {!loading && (
            <div className="pref-summary-pill">
              <span
                className={`pref-summary-dot ${tasks.length > 0 ? "is-live" : "is-amber"}`}
                aria-hidden="true"
              />
              <span>{summaryText}</span>
            </div>
          )}
        </div>
      </header>

      {error && <div className="card error-text">{error}</div>}

      {loading && (
        <div className="hist-skeleton-list" aria-busy="true" aria-label="Loading downloads">
          <div className="ott-skeleton hist-skeleton-row" />
          <div className="ott-skeleton hist-skeleton-row" />
          <div className="ott-skeleton hist-skeleton-row" />
        </div>
      )}

      {!loading && tasks.length === 0 && (
        <div className="hist-empty-card dl-empty-card">
          <div className="dl-empty-art">
            <img
              src={noDownloadsSvg}
              alt="No downloads illustration"
              width={196}
              height={196}
            />
          </div>
          <div className="hist-empty-title">No Offline Downloads</div>
          <div className="hist-empty-desc">
            Open any movie or series episode and click Download to save it locally for offline playback.
          </div>
          {onBrowseMovies && (
            <button
              type="button"
              className="btn btn-primary dl-empty-cta"
              onClick={onBrowseMovies}
            >
              <Compass size={15} /> Browse Movies &amp; Series
            </button>
          )}
        </div>
      )}

      {!loading && tasks.length > 0 && (
        <div className="dl-sections">
          <DownloadGroup
            label="Active & Queue"
            items={activeTasks}
            confirmDeleteId={confirmDeleteId}
            onPlayLocal={onPlayLocal}
            onPause={(id) => void handlePause(id)}
            onResume={(id) => void handleResume(id)}
            onOpenFolder={(p) => void handleOpenFolder(p)}
            onRequestDelete={setConfirmDeleteId}
            onConfirmDelete={(id) => void handleConfirmDelete(id)}
          />
          <DownloadGroup
            label="Saved Offline"
            items={completedTasks}
            confirmDeleteId={confirmDeleteId}
            onPlayLocal={onPlayLocal}
            onPause={(id) => void handlePause(id)}
            onResume={(id) => void handleResume(id)}
            onOpenFolder={(p) => void handleOpenFolder(p)}
            onRequestDelete={setConfirmDeleteId}
            onConfirmDelete={(id) => void handleConfirmDelete(id)}
          />
        </div>
      )}
    </div>
  );
}
