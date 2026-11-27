import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { FolderOpen, Pause, Play, RotateCw, Trash2 } from "lucide-react";
import { api } from "../api";
import type { DownloadTask, PlaybackRequest } from "../types";

interface Props {
  onPlayLocal: (req: PlaybackRequest) => void;
  onTasksChange?: (tasks: DownloadTask[]) => void;
}

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

export function DownloadsView({ onPlayLocal, onTasksChange }: Props) {
  const [tasks, setTasks] = useState<DownloadTask[]>([]);
  const [defaultDir, setDefaultDir] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    void api.listDownloads().then((list) => {
      setTasks(list);
      onTasksChange?.(list);
    });
    void api.getDefaultDownloadDir().then(setDefaultDir);

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
      unlisten?.();
    };
  }, []);

  async function handlePause(id: string) {
    const next = await api.pauseDownload(id);
    setTasks(next);
    onTasksChange?.(next);
  }

  async function handleResume(id: string) {
    const next = await api.resumeDownload(id);
    setTasks(next);
    onTasksChange?.(next);
  }

  async function handleDelete(id: string) {
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

  return (
    <div style={{ display: "grid", gap: 16 }}>
      <div className="page-header" style={{ marginBottom: 0 }}>
        <div>
          <h1 className="page-title">Downloads</h1>
          {defaultDir && <div className="muted">Save directory: {defaultDir}</div>}
        </div>
      </div>

      {error && <div className="card error-text">{error}</div>}

      {tasks.length === 0 && (
        <div className="card muted">
          No downloads yet. Open any movie or series episode and click Download to save for offline playback.
        </div>
      )}

      <div style={{ display: "grid", gap: 10 }}>
        {tasks.map((task) => {
          const pct = Math.min(100, Math.max(0, Math.round(task.progress || 0)));
          return (
            <div key={task.id} className="card" style={{ display: "grid", gap: 10 }}>
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 12 }}>
                <div style={{ minWidth: 0 }}>
                  <div style={{ fontWeight: 600 }}>{task.title}</div>
                  <div className="muted">
                    <span className="badge" style={{ marginRight: 8 }}>
                      {task.status}
                    </span>
                    {formatBytes(task.downloadedBytes)}
                    {task.totalBytes > 0 ? ` / ${formatBytes(task.totalBytes)}` : ""}
                    {task.speed ? ` • ${task.speed}` : ""}
                  </div>
                </div>
                <div style={{ display: "flex", gap: 8 }}>
                  {task.status === "COMPLETED" && (
                    <button
                      className="btn btn-primary"
                      onClick={() =>
                        onPlayLocal({
                          title: task.title,
                          link: task.savePath,
                          streamUrl: task.savePath,
                        })
                      }
                    >
                      <Play size={14} /> Play
                    </button>
                  )}
                  {(task.status === "DOWNLOADING" || task.status === "PENDING") && (
                    <button className="btn" onClick={() => void handlePause(task.id)}>
                      <Pause size={14} /> Pause
                    </button>
                  )}
                  {(task.status === "PAUSED" || task.status === "FAILED") && (
                    <button className="btn btn-primary" onClick={() => void handleResume(task.id)}>
                      <RotateCw size={14} /> {task.status === "FAILED" ? "Retry" : "Resume"}
                    </button>
                  )}
                  <button className="btn" onClick={() => void handleOpenFolder(task.savePath)} title="Open Folder">
                    <FolderOpen size={14} />
                  </button>
                  <button className="btn" onClick={() => void handleDelete(task.id)} title="Delete">
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>

              {(task.status === "DOWNLOADING" || task.status === "PAUSED" || task.status === "PENDING") && (
                <div
                  style={{
                    height: 6,
                    borderRadius: 999,
                    background: "rgba(255, 255, 255, 0.14)",
                    overflow: "hidden",
                  }}
                >
                  <div
                    style={{
                      width: `${pct}%`,
                      height: "100%",
                      background: "var(--accent-amber)",
                      transition: "width 0.2s ease",
                    }}
                  />
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
