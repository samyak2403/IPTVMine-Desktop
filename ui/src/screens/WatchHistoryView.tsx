import { useEffect, useState } from "react";
import { Play, Trash2 } from "lucide-react";
import { api } from "../api";
import type { PlaybackRequest, WatchHistoryEntry } from "../types";

interface Props {
  onResume: (req: PlaybackRequest) => void;
}

export function WatchHistoryView({ onResume }: Props) {
  const [items, setItems] = useState<WatchHistoryEntry[]>([]);
  const [loading, setLoading] = useState(true);

  async function load() {
    setLoading(true);
    try {
      setItems(await api.getWatchHistory());
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  async function handleDelete(link: string) {
    await api.deleteWatchHistoryEntry(link);
    setItems((prev) => prev.filter((i) => i.link !== link));
  }

  async function handleClearAll() {
    await api.clearWatchHistory();
    setItems([]);
  }

  return (
    <div>
      <div className="page-header">
        <h1 className="page-title">Watch History</h1>
        {items.length > 0 && (
          <button className="btn" onClick={() => void handleClearAll()}>
            <Trash2 size={14} /> Clear All
          </button>
        )}
      </div>

      {!loading && items.length === 0 && (
        <div className="card muted">
          No watch history yet. Play a channel or movie to track resume progress.
        </div>
      )}

      <div style={{ display: "grid", gap: 10 }}>
        {items.map((item) => {
          const pct =
            item.durationMs > 0
              ? Math.min(100, Math.round((item.positionMs / item.durationMs) * 100))
              : 0;
          return (
            <div
              key={item.link}
              className="card"
              style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 16 }}
            >
              {item.imageUrl && (
                <img
                  src={item.imageUrl}
                  alt={item.title}
                  style={{
                    width: 84,
                    height: 52,
                    borderRadius: 10,
                    objectFit: "cover",
                    flexShrink: 0,
                  }}
                />
              )}
              <div style={{ minWidth: 0, flex: 1 }}>
                <div style={{ fontWeight: 600 }}>{item.title}</div>
                <div className="muted" style={{ marginTop: 4 }}>
                  {pct > 0 ? `${pct}% watched` : "Started"}
                </div>
                {pct > 0 && (
                  <div
                    style={{
                      marginTop: 6,
                      height: 4,
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
                      }}
                    />
                  </div>
                )}
              </div>
              <div style={{ display: "flex", gap: 8 }}>
                <button
                  className="btn btn-primary"
                  onClick={() =>
                    onResume({
                      title: item.title,
                      link: item.link,
                      streamUrl: item.streamUrl,
                      imageUrl: item.imageUrl,
                      providerUrl: item.providerUrl,
                      scraperValue: item.scraperValue,
                    })
                  }
                >
                  <Play size={14} /> Resume
                </button>
                <button className="btn" onClick={() => void handleDelete(item.link)}>
                  <Trash2 size={14} />
                </button>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
