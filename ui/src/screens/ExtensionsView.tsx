import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import {
  Check,
  ChevronDown,
  CloudDownload,
  Compass,
  FolderGit2,
  Plus,
  Puzzle,
  RefreshCw,
  Search,
  Trash2,
} from "lucide-react";
import { api } from "../api";
import type { Provider, VegaProvider } from "../types";

interface Props {
  providers: Provider[];
  initialSourceUrl?: string;
  onProvidersChanged?: (providers: Provider[]) => void;
  onExtensionsChanged?: (exts: VegaProvider[]) => void;
  onOpenMovies?: () => void;
}

function formatRepoBadge(provider?: Provider | null): string {
  if (!provider) return "Source";
  const title = provider.title.trim();
  const url = provider.url.trim();
  try {
    if (url.startsWith("@")) return url.replace(/^@/, "");
    const u = new URL(url);
    const parts = u.pathname.split("/").filter(Boolean);
    if (
      (u.hostname.includes("github") || u.hostname.includes("jsdelivr")) &&
      parts.length >= 2
    ) {
      return parts[1];
    }
    return title || u.hostname;
  } catch {
    return title || "Source";
  }
}

function formatSourceDisplayName(provider?: Provider | null): string {
  if (!provider) return "No provider configured";
  const url = provider.url.trim();
  try {
    if (url.startsWith("@")) return url;
    const u = new URL(url);
    const parts = u.pathname.split("/").filter(Boolean);
    if (
      (u.hostname.includes("github") || u.hostname.includes("jsdelivr")) &&
      parts.length >= 2
    ) {
      return `${parts[0]}/${parts[1]}`;
    }
  } catch {
    // ignore URL parse error and fall back to title/url
  }
  return provider.title || url;
}

function formatTypeLabel(rawType: string): string {
  const t = rawType.trim();
  if (!t) return "Global";
  return t.charAt(0).toUpperCase() + t.slice(1).toLowerCase();
}

function ExtensionCard({
  ext,
  sourceBadge,
  onToggle,
}: {
  ext: VegaProvider;
  sourceBadge: string;
  onToggle: (ext: VegaProvider) => void;
}) {
  const [imgBroken, setImgBroken] = useState(false);

  return (
    <div className={`ott-ext-card ${ext.is_installed ? "installed" : ""}`}>
      <div style={{ display: "flex", alignItems: "center", gap: 13, minWidth: 0 }}>
        {ext.icon && !imgBroken ? (
          <img
            className="ott-ext-icon"
            src={ext.icon}
            alt={ext.display_name || ext.value}
            loading="lazy"
            onError={() => setImgBroken(true)}
          />
        ) : (
          <span className="ott-ext-icon-fallback">
            <Puzzle size={20} color="#ffc062" />
          </span>
        )}
        <div style={{ minWidth: 0 }}>
          <div className="ott-ext-name">{ext.display_name || ext.value}</div>
          <div className="ott-ext-meta">
            <span className="ott-ext-version">v{ext.version || "1.0"}</span>
            <span className="ott-ext-chip">{formatTypeLabel(ext.type)}</span>
            <span className="ott-ext-chip">{sourceBadge}</span>
          </div>
        </div>
      </div>

      <button
        type="button"
        className={`ott-ext-action ${ext.is_installed ? "installed" : ""}`}
        onClick={() => onToggle(ext)}
        aria-label={
          ext.is_installed
            ? `Uninstall ${ext.display_name || ext.value}`
            : `Install ${ext.display_name || ext.value}`
        }
      >
        {ext.is_installed ? (
          <>
            <Trash2 size={14} /> Uninstall
          </>
        ) : (
          <>
            <CloudDownload size={14} /> Install
          </>
        )}
      </button>
    </div>
  );
}

export function ExtensionsView({
  providers,
  initialSourceUrl,
  onProvidersChanged,
  onExtensionsChanged,
  onOpenMovies,
}: Props) {
  const vegaProviders = useMemo(
    () => providers.filter((p) => p.type === "VEGA"),
    [providers]
  );
  const defaultActive = useMemo(
    () => vegaProviders.find((p) => p.isActive) ?? vegaProviders[0] ?? null,
    [vegaProviders]
  );

  const [selectedUrl, setSelectedUrl] = useState<string>(
    () => initialSourceUrl || defaultActive?.url || ""
  );
  const [sourceMenuOpen, setSourceMenuOpen] = useState(false);
  const [showAddSource, setShowAddSource] = useState(false);
  const [newSourceUrl, setNewSourceUrl] = useState("");
  const [newSourceTitle, setNewSourceTitle] = useState("");
  const [savingSource, setSavingSource] = useState(false);
  const [extensions, setExtensions] = useState<VegaProvider[]>([]);
  const [query, setQuery] = useState("");
  const [selectedType, setSelectedType] = useState("all");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const sourceRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (initialSourceUrl) {
      setSelectedUrl(initialSourceUrl);
    }
  }, [initialSourceUrl]);

  useEffect(() => {
    if (!selectedUrl && defaultActive?.url) {
      setSelectedUrl(defaultActive.url);
    }
  }, [defaultActive, selectedUrl]);

  useEffect(() => {
    function handleOutsideClick(ev: MouseEvent) {
      if (sourceRef.current && !sourceRef.current.contains(ev.target as Node)) {
        setSourceMenuOpen(false);
      }
    }
    function handleEsc(ev: KeyboardEvent) {
      if (ev.key === "Escape") {
        setSourceMenuOpen(false);
        setShowAddSource(false);
      }
    }
    window.addEventListener("mousedown", handleOutsideClick);
    window.addEventListener("keydown", handleEsc);
    return () => {
      window.removeEventListener("mousedown", handleOutsideClick);
      window.removeEventListener("keydown", handleEsc);
    };
  }, []);

  const currentProvider = useMemo(
    () =>
      vegaProviders.find((p) => p.url === selectedUrl) ?? defaultActive ?? null,
    [vegaProviders, selectedUrl, defaultActive]
  );

  async function load(force = false, targetUrl?: string) {
    const providerUrl = targetUrl ?? currentProvider?.url;
    if (!providerUrl) {
      setExtensions([]);
      return;
    }
    setLoading(true);
    setError("");
    try {
      const list = await api.listVegaExtensions(providerUrl, force);
      setExtensions(list);
      onExtensionsChanged?.(list);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load(false, currentProvider?.url);
  }, [currentProvider?.url]);

  async function switchActiveSource(target: Provider) {
    setSourceMenuOpen(false);
    setSelectedUrl(target.url);
    if (!target.isActive) {
      try {
        const updated = await api.updateProvider(target.url, {
          ...target,
          isActive: true,
        });
        onProvidersChanged?.(updated);
      } catch (err) {
        setError(String(err));
      }
    }
  }

  async function handleAddSource(e: FormEvent) {
    e.preventDefault();
    const url = newSourceUrl.trim();
    if (!url) {
      setError("Enter a manifest URL, GitHub repo, or @handle.");
      return;
    }
    setSavingSource(true);
    setError("");
    try {
      const updated = await api.addProvider({
        title: newSourceTitle.trim(),
        url,
        userAgent: null,
        isActive: true,
        channelCount: 0,
        type: "VEGA",
      });
      onProvidersChanged?.(updated);
      setNewSourceUrl("");
      setNewSourceTitle("");
      setShowAddSource(false);
      setSelectedUrl(url);
      await load(true, url);
    } catch (err) {
      setError(String(err));
    } finally {
      setSavingSource(false);
    }
  }

  async function toggleInstall(ext: VegaProvider) {
    const nextInstalled = !ext.is_installed;
    await api.setExtensionInstalled(ext.value, nextInstalled);
    const updated = extensions.map((item) =>
      item.value === ext.value ? { ...item, is_installed: nextInstalled } : item
    );
    setExtensions(updated);
    onExtensionsChanged?.(updated);
  }

  const availableTypes = useMemo(() => {
    const set = new Set<string>();
    extensions.forEach((ext) => {
      if (!ext.disabled && ext.type.trim()) {
        set.add(ext.type.trim().toLowerCase());
      }
    });
    return ["all", ...Array.from(set).sort()];
  }, [extensions]);

  const { installedList, availableList } = useMemo(() => {
    const q = query.trim().toLowerCase();
    const matches = extensions.filter((ext) => {
      if (ext.disabled && !ext.is_installed) return false;
      const typeOk =
        selectedType === "all" ||
        ext.type.trim().toLowerCase() === selectedType;
      const qOk =
        !q ||
        ext.display_name.toLowerCase().includes(q) ||
        ext.value.toLowerCase().includes(q) ||
        ext.type.toLowerCase().includes(q);
      return typeOk && qOk;
    });
    return {
      installedList: matches.filter((e) => e.is_installed),
      availableList: matches.filter((e) => !e.is_installed),
    };
  }, [extensions, query, selectedType]);

  const sourceBadge = formatRepoBadge(currentProvider);
  const totalInstalled = useMemo(
    () => extensions.filter((e) => e.is_installed).length,
    [extensions]
  );

  return (
    <div className="ott-ext-shell">
      <div className="ott-ext-header">
        <div>
          <h1 className="page-title" style={{ letterSpacing: "-0.028em" }}>
            Extensions
          </h1>
          <p className="ott-ext-subtitle">
            Install and choose streaming sources
          </p>
        </div>

        <div style={{ display: "flex", gap: 10, alignItems: "center" }}>
          {totalInstalled > 0 && onOpenMovies && (
            <button type="button" className="btn" onClick={onOpenMovies}>
              <Compass size={14} /> Browse Movies ({totalInstalled})
            </button>
          )}
          <button
            type="button"
            className="ott-icon-btn"
            aria-label="Refresh provider manifest"
            title="Refresh manifest"
            disabled={loading || !currentProvider}
            onClick={() => void load(true)}
          >
            <RefreshCw size={16} className={loading ? "ott-spin" : ""} />
          </button>
        </div>
      </div>

      {/* Provider Source Dropdown & Quick Add */}
      <div className="ott-source-section" ref={sourceRef}>
        <span className="ott-source-label">Provider source</span>
        <div className="ott-source-bar-row">
          <button
            type="button"
            className="ott-source-trigger"
            aria-expanded={sourceMenuOpen}
            onClick={() => setSourceMenuOpen((prev) => !prev)}
          >
            <div style={{ display: "flex", alignItems: "center", gap: 12, minWidth: 0 }}>
              <span className="ott-source-badge-icon">
                <FolderGit2 size={17} />
              </span>
              <div style={{ minWidth: 0 }}>
                <div style={{ fontSize: 11, color: "var(--text-muted)", fontWeight: 500 }}>
                  Active source
                </div>
                <div
                  style={{
                    fontSize: 14,
                    fontWeight: 600,
                    color: "var(--text-primary)",
                    whiteSpace: "nowrap",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                  }}
                >
                  {formatSourceDisplayName(currentProvider)}
                </div>
              </div>
            </div>
            <ChevronDown
              size={16}
              style={{
                color: "var(--text-secondary)",
                flexShrink: 0,
                transform: sourceMenuOpen ? "rotate(180deg)" : "rotate(0deg)",
                transition: "transform 200ms var(--ease-spring)",
              }}
            />
          </button>

          <button
            type="button"
            className="ott-icon-btn"
            style={{ width: 54, height: "auto", borderRadius: 14 }}
            aria-label="Add extension source"
            title="Add source"
            onClick={() => {
              setSourceMenuOpen(false);
              setShowAddSource((prev) => !prev);
            }}
          >
            <Plus size={18} />
          </button>
        </div>

        {sourceMenuOpen && (
          <div className="ott-source-menu" role="listbox" aria-label="Extension sources">
            {vegaProviders.length === 0 ? (
              <div className="muted" style={{ padding: "10px 12px" }}>
                No source added yet. Click + to add a manifest URL or GitHub repo.
              </div>
            ) : (
              vegaProviders.map((prov) => {
                const isSelected = prov.url === currentProvider?.url;
                return (
                  <button
                    key={prov.url}
                    type="button"
                    role="option"
                    aria-selected={isSelected}
                    className={`ott-source-option ${isSelected ? "active" : ""}`}
                    onClick={() => void switchActiveSource(prov)}
                  >
                    <div style={{ minWidth: 0 }}>
                      <div style={{ fontWeight: 600, fontSize: 13.5 }}>
                        {formatSourceDisplayName(prov)}
                      </div>
                      <div
                        className="muted"
                        style={{
                          fontSize: 11.5,
                          whiteSpace: "nowrap",
                          overflow: "hidden",
                          textOverflow: "ellipsis",
                        }}
                      >
                        {prov.url}
                      </div>
                    </div>
                    {isSelected && <Check size={15} color="#ffc062" />}
                  </button>
                );
              })
            )}
          </div>
        )}

        {showAddSource && (
          <form className="ott-source-add-sheet" onSubmit={handleAddSource}>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <strong style={{ fontSize: 13.5 }}>Add Extension Source</strong>
              <button
                type="button"
                className="btn"
                style={{ padding: "4px 10px", fontSize: 12 }}
                onClick={() => setShowAddSource(false)}
              >
                Close
              </button>
            </div>
            <div style={{ display: "flex", gap: 10, flexWrap: "wrap" }}>
              <input
                className="input"
                style={{ flex: "1 1 280px" }}
                placeholder="Manifest URL, GitHub repo, or @owner/repo"
                value={newSourceUrl}
                onChange={(e) => setNewSourceUrl(e.target.value)}
                autoFocus
              />
              <input
                className="input"
                style={{ flex: "0 1 180px" }}
                placeholder="Label (optional)"
                value={newSourceTitle}
                onChange={(e) => setNewSourceTitle(e.target.value)}
              />
              <button type="submit" className="btn btn-primary" disabled={savingSource}>
                {savingSource ? "Adding..." : "Add Source"}
              </button>
            </div>
          </form>
        )}
      </div>

      {/* Filter Toolbar */}
      {extensions.length > 0 && (
        <div className="ott-ext-toolbar">
          <div className="ott-ext-search">
            <Search size={15} className="ott-ext-search-icon" />
            <input
              className="input"
              style={{ borderRadius: 999, paddingLeft: 38, paddingTop: 9, paddingBottom: 9 }}
              placeholder="Filter extensions..."
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </div>
          {availableTypes.length > 2 && (
            <div className="chip-row" style={{ marginBottom: 0 }}>
              {availableTypes.map((typeKey) => (
                <button
                  key={typeKey}
                  type="button"
                  className={`chip ${selectedType === typeKey ? "active" : ""}`}
                  onClick={() => setSelectedType(typeKey)}
                >
                  {typeKey === "all" ? "All" : formatTypeLabel(typeKey)}
                </button>
              ))}
            </div>
          )}
        </div>
      )}

      {error && <div className="card error-text">{error}</div>}

      {!loading && extensions.length === 0 && !error && (
        <div className="card muted">
          No extension source found. Click <strong>+</strong> above to add a manifest URL or GitHub repo.
        </div>
      )}

      {/* Installed Section */}
      {installedList.length > 0 && (
        <section aria-label="Installed Extensions">
          <div className="ott-ext-section-head">
            <h2 className="ott-ext-section-title">Installed</h2>
            <span className="ott-ext-section-count">{installedList.length}</span>
          </div>
          <div className="ott-ext-grid">
            {installedList.map((ext) => (
              <ExtensionCard
                key={ext.value}
                ext={ext}
                sourceBadge={sourceBadge}
                onToggle={(item) => void toggleInstall(item)}
              />
            ))}
          </div>
        </section>
      )}

      {/* Available Section */}
      {availableList.length > 0 && (
        <section aria-label="Available Extensions">
          <div className="ott-ext-section-head">
            <h2 className="ott-ext-section-title">Available</h2>
            <span className="ott-ext-section-count">{availableList.length}</span>
          </div>
          <div className="ott-ext-grid">
            {availableList.map((ext) => (
              <ExtensionCard
                key={ext.value}
                ext={ext}
                sourceBadge={sourceBadge}
                onToggle={(item) => void toggleInstall(item)}
              />
            ))}
          </div>
        </section>
      )}

      {!loading && extensions.length > 0 && installedList.length === 0 && availableList.length === 0 && (
        <div className="card muted">
          No extensions matched "{query}".
        </div>
      )}
    </div>
  );
}
