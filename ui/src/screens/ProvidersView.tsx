import { useCallback, useId, useMemo, useRef, useState } from "react";
import {
  AlertCircle,
  Blocks,
  Check,
  ChevronDown,
  Film,
  Pencil,
  Play,
  RefreshCw,
  Search,
  Trash2,
  Tv,
  Upload,
  X,
} from "lucide-react";
import type { PlaybackRequest, Provider, ProviderType } from "../types";

const PROVIDER_TYPES: { value: ProviderType; label: string }[] = [
  { value: "IPTV", label: "IPTV" },
  { value: "VEGA", label: "SOURCE" },
  { value: "VIDEO", label: "VIDEO" },
];

const PROVIDER_TYPE_BADGE: Record<ProviderType, string> = {
  IPTV: "IPTV",
  VEGA: "SOURCE",
  VIDEO: "VIDEO",
};

const SOURCE_TYPE_COPY: Record<
  ProviderType,
  {
    newHeading: string;
    editHeading: string;
    urlLabel: string;
    urlPlaceholder: string;
    titlePlaceholder: string;
    saveCta: string;
    updateCta: string;
  }
> = {
  IPTV: {
    newHeading: "New IPTV Playlist Source",
    editHeading: "Update IPTV Source",
    urlLabel: "Playlist URL",
    urlPlaceholder: "https://example.com/playlist.m3u or channel JSON URL",
    titlePlaceholder: "Friendly name for Live TV & Home sections",
    saveCta: "Save IPTV Source",
    updateCta: "Update IPTV Source",
  },
  VEGA: {
    newHeading: "New Source",
    editHeading: "Update Source",
    urlLabel: "Manifest URL or GitHub Source",
    urlPlaceholder: "manifest.json URL, GitHub repo URL, or @owner/repo",
    titlePlaceholder: "Friendly name for Movies & Extensions",
    saveCta: "Save Source",
    updateCta: "Update Source",
  },
  VIDEO: {
    newHeading: "New Direct Video Source",
    editHeading: "Update Video Source",
    urlLabel: "Direct Stream URL",
    urlPlaceholder: "Direct .mp4, .mkv, .m3u8, rtmp://, or rtsp:// stream URL",
    titlePlaceholder: "Display name in Connected Sources",
    saveCta: "Save Video Source",
    updateCta: "Update Video Source",
  },
};

interface Props {
  providers: Provider[];
  editingUrl: string | null;
  newTitle: string;
  newUrl: string;
  newUa: string;
  detectedType: ProviderType;
  manualTypeOverride: boolean;
  inspectInfo: string;
  savingProvider: boolean;
  providerError: string;
  providerSuccess: string;
  onUrlChange: (val: string) => void;
  onTitleChange: (val: string) => void;
  onUaChange: (val: string) => void;
  onTypeSelect: (type: ProviderType) => void;
  onInspect: () => void;
  onSubmit: (e: React.FormEvent) => void;
  onImportLocalFile: (e: React.ChangeEvent<HTMLInputElement>) => void;
  onCancelEdit: () => void;
  onEditProvider: (p: Provider) => void;
  onToggleProvider: (p: Provider) => void;
  onRemoveProvider: (url: string) => void;
  onPlayVideo: (req: PlaybackRequest) => void;
}

export function ProvidersView({
  providers,
  editingUrl,
  newTitle,
  newUrl,
  newUa,
  detectedType,
  manualTypeOverride,
  inspectInfo,
  savingProvider,
  providerError,
  providerSuccess,
  onUrlChange,
  onTitleChange,
  onUaChange,
  onTypeSelect,
  onInspect,
  onSubmit,
  onImportLocalFile,
  onCancelEdit,
  onEditProvider,
  onToggleProvider,
  onRemoveProvider,
  onPlayVideo,
}: Props) {
  const headingId = useId();
  const urlInputId = useId();
  const titleInputId = useId();
  const uaInputId = useId();
  const composerRef = useRef<HTMLFormElement>(null);

  const [uaExpandedManual, setUaExpandedManual] = useState(false);
  const [confirmDeleteUrl, setConfirmDeleteUrl] = useState<string | null>(null);

  const showUaDrawer = uaExpandedManual || Boolean(newUa.trim());
  const isInspecting = inspectInfo === "Inspecting...";
  const sourceCopy = SOURCE_TYPE_COPY[detectedType];

  const activeStats = useMemo(() => {
    const activeCount = providers.filter((p) => p.isActive).length;
    const totalChannels = providers
      .filter((p) => p.isActive && p.type === "IPTV")
      .reduce((acc, p) => acc + (p.channelCount || 0), 0);
    return { activeCount, totalChannels };
  }, [providers]);

  const handleTypeKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLButtonElement>, idx: number) => {
      if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
      e.preventDefault();
      const delta = e.key === "ArrowRight" ? 1 : -1;
      const nextIdx = (idx + delta + PROVIDER_TYPES.length) % PROVIDER_TYPES.length;
      onTypeSelect(PROVIDER_TYPES[nextIdx].value);
      const group = e.currentTarget.closest('[role="radiogroup"]');
      const nextBtn = group?.querySelectorAll<HTMLButtonElement>('[role="radio"]')[nextIdx];
      nextBtn?.focus();
    },
    [onTypeSelect]
  );

  function renderTypeIcon(type: ProviderType) {
    if (type === "IPTV") return <Tv size={17} strokeWidth={2} />;
    if (type === "VEGA") return <Blocks size={17} strokeWidth={2} />;
    return <Film size={17} strokeWidth={2} />;
  }

  return (
    <section className="prov-screen" aria-labelledby={headingId}>
      <header className="pref-header">
        <div className="pref-title-block">
          <h1 id={headingId} className="pref-title">
            Providers
          </h1>
          <p className="pref-subtitle">
            Connect IPTV playlists, extension sources, GitHub repos, or direct streams. Types detect automatically as you paste.
          </p>
        </div>

        <div className="pref-summary-pill">
          <span
            className={`pref-summary-dot ${activeStats.activeCount > 0 ? "is-live" : "is-amber"}`}
            aria-hidden="true"
          />
          <span>
            {activeStats.activeCount} Active {activeStats.activeCount === 1 ? "Source" : "Sources"}
          </span>
          {activeStats.totalChannels > 0 && (
            <>
              <span className="pref-summary-sep" aria-hidden="true" />
              <span>{activeStats.totalChannels.toLocaleString()} Channels</span>
            </>
          )}
        </div>
      </header>

      {/* Section 1: Add / Edit Source Composer */}
      <div className="pref-section">
        <div className="pref-section-label">
          {editingUrl ? "Editing Source" : "Add Media Source"}
        </div>

        <form ref={composerRef} className="pref-group prov-composer" onSubmit={onSubmit}>
          <div className="prov-composer-head">
            <div className="prov-composer-title">
              <span>{editingUrl ? sourceCopy.editHeading : sourceCopy.newHeading}</span>
              {!manualTypeOverride && newUrl.trim().length > 0 && (
                <span className="prov-auto-pill">Auto-detected</span>
              )}
            </div>

            <div className="pref-seg-control" role="radiogroup" aria-label="Provider source type">
              {PROVIDER_TYPES.map((t, idx) => {
                const active = detectedType === t.value;
                return (
                  <button
                    key={t.value}
                    type="button"
                    role="radio"
                    aria-checked={active}
                    className={`pref-seg-btn ${active ? "active" : ""}`}
                    onClick={() => onTypeSelect(t.value)}
                    onKeyDown={(e) => handleTypeKeyDown(e, idx)}
                  >
                    {t.label}
                  </button>
                );
              })}
            </div>
          </div>

          <div className="prov-form-grid">
            <div className="prov-field">
              <label htmlFor={urlInputId} className="prov-field-label">
                {sourceCopy.urlLabel}
              </label>
              <div className="pref-input-wrap">
                <input
                  id={urlInputId}
                  className="input pref-path-input"
                  placeholder={sourceCopy.urlPlaceholder}
                  value={newUrl}
                  onChange={(e) => onUrlChange(e.target.value)}
                />
                {newUrl.length > 0 && (
                  <button
                    type="button"
                    className="dns-clear-btn"
                    aria-label="Clear source URL"
                    onClick={() => onUrlChange("")}
                  >
                    <X size={14} />
                  </button>
                )}
              </div>
            </div>

            <div className="prov-field">
              <label htmlFor={titleInputId} className="prov-field-label">
                Display Title <span className="prov-opt">(Optional)</span>
              </label>
              <div className="pref-input-wrap">
                <input
                  id={titleInputId}
                  className="input pref-path-input"
                  placeholder={sourceCopy.titlePlaceholder}
                  value={newTitle}
                  onChange={(e) => onTitleChange(e.target.value)}
                />
                {newTitle.length > 0 && (
                  <button
                    type="button"
                    className="dns-clear-btn"
                    aria-label="Clear display title"
                    onClick={() => onTitleChange("")}
                  >
                    <X size={14} />
                  </button>
                )}
              </div>
            </div>
          </div>

          <div className="prov-advanced-wrap">
            <button
              type="button"
              className="prov-disclosure-btn"
              aria-expanded={showUaDrawer}
              onClick={() => setUaExpandedManual((v) => !v)}
            >
              <ChevronDown
                size={14}
                className={`prov-disclosure-chevron ${showUaDrawer ? "open" : ""}`}
              />
              <span>Advanced Options (Custom User-Agent)</span>
            </button>

            <div className={`dns-custom-drawer ${showUaDrawer ? "open" : ""}`} aria-hidden={!showUaDrawer}>
              <div className="prov-ua-inner">
                <label htmlFor={uaInputId} className="prov-field-label">
                  Custom HTTP User-Agent
                </label>
                <div className="pref-input-wrap">
                  <input
                    id={uaInputId}
                    tabIndex={showUaDrawer ? 0 : -1}
                    className="input pref-path-input"
                    placeholder="Optional HTTP User-Agent header sent when fetching this source"
                    value={newUa}
                    onChange={(e) => onUaChange(e.target.value)}
                  />
                  {newUa.length > 0 && (
                    <button
                      type="button"
                      tabIndex={showUaDrawer ? 0 : -1}
                      className="dns-clear-btn"
                      aria-label="Clear custom User-Agent"
                      onClick={() => onUaChange("")}
                    >
                      <X size={14} />
                    </button>
                  )}
                </div>
              </div>
            </div>
          </div>

          {(inspectInfo || providerSuccess || providerError) && (
            <div className="prov-status-banners" aria-live="polite">
              {inspectInfo && (
                <div className="prov-banner is-info">
                  <RefreshCw size={14} className={isInspecting ? "dns-spin" : ""} />
                  <span>{inspectInfo}</span>
                </div>
              )}
              {providerSuccess && (
                <div className="prov-banner is-ok">
                  <Check size={14} />
                  <span>{providerSuccess}</span>
                </div>
              )}
              {providerError && (
                <div className="prov-banner is-error" role="alert">
                  <AlertCircle size={14} />
                  <span>{providerError}</span>
                </div>
              )}
            </div>
          )}

          <div className="prov-composer-actions">
            <div className="prov-secondary-actions">
              <button
                type="button"
                className="btn"
                disabled={savingProvider || !newUrl.trim()}
                onClick={onInspect}
              >
                <Search size={14} />
                <span>{isInspecting ? "Inspecting..." : "Inspect URL"}</span>
              </button>
              <label className="btn prov-upload-label">
                <Upload size={14} />
                <span>Import .m3u / .json</span>
                <input
                  type="file"
                  accept=".m3u,.m3u8,.json,.txt"
                  style={{ display: "none" }}
                  onChange={onImportLocalFile}
                />
              </label>
            </div>

            <div className="prov-primary-actions">
              {editingUrl && (
                <button
                  type="button"
                  className="btn"
                  disabled={savingProvider}
                  onClick={onCancelEdit}
                >
                  Cancel Edit
                </button>
              )}
              <button
                type="submit"
                className="btn btn-primary"
                disabled={savingProvider}
              >
                {savingProvider
                  ? "Saving..."
                  : editingUrl
                    ? sourceCopy.updateCta
                    : sourceCopy.saveCta}
              </button>
            </div>
          </div>
        </form>
      </div>

      {/* Section 2: Connected Providers List */}
      <div className="pref-section">
        <div className="pref-section-label">
          Connected Sources ({providers.length})
        </div>

        <div className="pref-group">
          {providers.length === 0 ? (
            <div className="prov-empty">
              No providers added yet. Paste a playlist or extension URL above, or import a local `.m3u` or `.json` file.
            </div>
          ) : (
            providers.map((p) => {
              const confirmingDelete = confirmDeleteUrl === p.url;
              return (
                <div
                  key={p.url}
                  className={`prov-row ${!p.isActive ? "is-paused" : ""} ${
                    editingUrl === p.url ? "is-editing" : ""
                  }`}
                >
                  <span
                    className={`pref-row-icon ${p.isActive ? "is-active" : ""}`}
                    aria-hidden="true"
                  >
                    {renderTypeIcon(p.type)}
                  </span>

                  <div className="prov-row-meta">
                    <div className="prov-row-top">
                      <span className="pref-row-title">{p.title}</span>
                      <span className="ott-meta-pill amber">{PROVIDER_TYPE_BADGE[p.type]}</span>
                      {p.type === "IPTV" && (
                        <span className="ott-meta-pill dns-tabular">
                          {p.channelCount.toLocaleString()} channels
                        </span>
                      )}
                      {!p.isActive && <span className="ott-meta-pill">Paused</span>}
                    </div>
                    <div className="prov-row-url" title={p.url}>
                      {p.url}
                    </div>
                  </div>

                  <div className="prov-row-controls">
                    {p.type === "VIDEO" && (
                      <button
                        type="button"
                        className="btn btn-primary prov-compact-btn"
                        onClick={() =>
                          onPlayVideo({
                            title: p.title,
                            link: p.url,
                            streamUrl: p.url,
                            headers: p.userAgent ? { "User-Agent": p.userAgent } : undefined,
                          })
                        }
                      >
                        <Play size={14} />
                        <span>Play</span>
                      </button>
                    )}

                    <input
                      type="checkbox"
                      className="apple-switch"
                      aria-label={`Toggle ${p.title} active`}
                      checked={p.isActive}
                      onChange={() => onToggleProvider(p)}
                    />

                    <button
                      type="button"
                      className="btn prov-icon-btn"
                      aria-label={`Edit ${p.title}`}
                      title="Edit source"
                      onClick={() => {
                        setConfirmDeleteUrl(null);
                        onEditProvider(p);
                        composerRef.current?.scrollIntoView({ behavior: "smooth", block: "nearest" });
                      }}
                    >
                      <Pencil size={14} />
                    </button>

                    {confirmingDelete ? (
                      <div className="prov-delete-confirm">
                        <button
                          type="button"
                          className="btn prov-confirm-danger"
                          onClick={() => {
                            setConfirmDeleteUrl(null);
                            onRemoveProvider(p.url);
                          }}
                        >
                          Remove
                        </button>
                        <button
                          type="button"
                          className="btn prov-compact-btn"
                          onClick={() => setConfirmDeleteUrl(null)}
                        >
                          Cancel
                        </button>
                      </div>
                    ) : (
                      <button
                        type="button"
                        className="btn prov-icon-btn"
                        aria-label={`Remove ${p.title}`}
                        title="Remove source"
                        onClick={() => setConfirmDeleteUrl(p.url)}
                      >
                        <Trash2 size={14} />
                      </button>
                    )}
                  </div>
                </div>
              );
            })
          )}
        </div>
      </div>
    </section>
  );
}
