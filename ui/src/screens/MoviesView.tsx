import { useEffect, useState } from "react";
import { Search, RefreshCw } from "lucide-react";
import { api } from "../api";
import { formatOttTitle } from "../components/OttUi";
import type { VegaCatalog, VegaPost, VegaProvider } from "../types";

interface Props {
  initialFilter?: string;
  onSelectPost: (post: VegaPost, scraperValue: string) => void;
  onOpenExtensions: () => void;
}

export function MoviesView({ initialFilter, onSelectPost, onOpenExtensions }: Props) {
  const [extensions, setExtensions] = useState<VegaProvider[]>([]);
  const [selectedScraper, setSelectedScraper] = useState<string>("");
  const [catalog, setCatalog] = useState<VegaCatalog[]>([]);
  const [genres, setGenres] = useState<VegaCatalog[]>([]);
  const [selectedFilter, setSelectedFilter] = useState<string>("");
  const [posts, setPosts] = useState<VegaPost[]>([]);
  const [page, setPage] = useState(1);
  const [searchQuery, setSearchQuery] = useState("");
  const [isSearching, setIsSearching] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    async function init() {
      setLoading(true);
      setError("");
      try {
        const list = await api.listVegaExtensions();
        const installed = list.filter((e) => e.is_installed && !e.disabled);
        const active = installed.length > 0 ? installed : list.filter((e) => !e.disabled);
        setExtensions(active);
        if (active.length > 0) {
          setSelectedScraper(active[0].value);
        }
      } catch (err) {
        setError(String(err));
      } finally {
        setLoading(false);
      }
    }
    void init();
  }, []);

  useEffect(() => {
    if (!selectedScraper) return;
    async function loadCatalog() {
      setLoading(true);
      setError("");
      setIsSearching(false);
      setSearchQuery("");
      try {
        const res = await api.getVodCatalog(selectedScraper);
        setCatalog(res.catalog);
        setGenres(res.genres ?? []);
        const validInitial =
          initialFilter !== undefined &&
          [...res.catalog, ...(res.genres ?? [])].some((c) => c.filter === initialFilter)
            ? initialFilter
            : undefined;
        const firstFilter = validInitial ?? res.catalog[0]?.filter ?? "";
        setSelectedFilter(firstFilter);
        setPage(1);
        const initialPosts = await api.getVodPosts(selectedScraper, firstFilter, 1);
        setPosts(initialPosts);
      } catch (err) {
        setError(String(err));
        setPosts([]);
      } finally {
        setLoading(false);
      }
    }
    void loadCatalog();
  }, [selectedScraper]);

  async function selectCategory(filter: string) {
    setSelectedFilter(filter);
    setIsSearching(false);
    setPage(1);
    setLoading(true);
    setError("");
    try {
      setPosts(await api.getVodPosts(selectedScraper, filter, 1));
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }

  async function runSearch(e: React.FormEvent) {
    e.preventDefault();
    if (!selectedScraper || !searchQuery.trim()) return;
    setIsSearching(true);
    setPage(1);
    setLoading(true);
    setError("");
    try {
      setPosts(await api.searchVodPosts(selectedScraper, searchQuery.trim(), 1));
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }

  async function loadMore() {
    if (!selectedScraper || loading) return;
    const nextPage = page + 1;
    setLoading(true);
    try {
      const more = isSearching
        ? await api.searchVodPosts(selectedScraper, searchQuery.trim(), nextPage)
        : await api.getVodPosts(selectedScraper, selectedFilter, nextPage);
      if (more.length > 0) {
        setPosts((prev) => [...prev, ...more]);
        setPage(nextPage);
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }

  if (extensions.length === 0 && !loading) {
    return (
      <div className="card" style={{ display: "grid", gap: 12 }}>
        <h1 className="page-title">Movies & VOD</h1>
        <div className="muted">
          {error || "No active Vega extensions loaded. Configure a Vega provider and install extensions."}
        </div>
        <div>
          <button type="button" className="btn btn-primary" onClick={onOpenExtensions}>
            Open Extensions Manager
          </button>
        </div>
      </div>
    );
  }

  const allFilters = [...catalog, ...genres.filter((g) => !catalog.some((c) => c.filter === g.filter))];

  return (
    <div>
      <div className="page-header">
        <h1 className="page-title">Movies & VOD</h1>
        <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
          <select
            className="input"
            style={{ width: 210, borderRadius: 999 }}
            value={selectedScraper}
            onChange={(e) => setSelectedScraper(e.target.value)}
          >
            {extensions.map((ext) => (
              <option key={ext.value} value={ext.value}>
                {ext.display_name || ext.value}
              </option>
            ))}
          </select>
          <button type="button" className="btn" onClick={onOpenExtensions}>
            Extensions
          </button>
          <button
            type="button"
            className="btn"
            aria-label="Refresh movies"
            onClick={() => void selectCategory(selectedFilter)}
            disabled={loading}
          >
            <RefreshCw size={14} />
          </button>
        </div>
      </div>

      <form onSubmit={runSearch} style={{ display: "flex", gap: 10, marginBottom: 18 }}>
        <input
          className="input"
          style={{ borderRadius: 999, paddingLeft: 18 }}
          placeholder="Search movies & series across selected source..."
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
        />
        <button type="submit" className="btn btn-primary" disabled={loading}>
          <Search size={14} /> Search
        </button>
      </form>

      {allFilters.length > 0 && (
        <div className="chip-row">
          {allFilters.map((cat) => (
            <button
              type="button"
              key={`${cat.filter}-${cat.title}`}
              className={`chip ${!isSearching && selectedFilter === cat.filter ? "active" : ""}`}
              onClick={() => void selectCategory(cat.filter)}
            >
              {cat.title}
            </button>
          ))}
        </div>
      )}

      {error && (
        <div className="card error-text" style={{ marginBottom: 16 }}>
          {error}
        </div>
      )}

      {loading && posts.length === 0 ? (
        <div className="grid-posters">
          {Array.from({ length: 12 }).map((_, i) => (
            <div key={i} className="shimmer" style={{ aspectRatio: "388 / 580" }} />
          ))}
        </div>
      ) : (
        <div className="grid-posters">
          {posts.map((post) => (
            <button
              type="button"
              key={`${post.link}-${post.title}`}
              className="poster-card"
              onClick={() => onSelectPost(post, selectedScraper)}
            >
              {post.image ? (
                <img className="poster-img" src={post.image} alt={post.title} loading="lazy" />
              ) : (
                <div className="poster-img" />
              )}
              <div className="poster-title">{formatOttTitle(post.title)}</div>
            </button>
          ))}
        </div>
      )}

      {posts.length > 0 && (
        <div style={{ marginTop: 24, display: "flex", justifyContent: "center" }}>
          <button type="button" className="btn" onClick={() => void loadMore()} disabled={loading}>
            {loading ? "Loading..." : "Load More"}
          </button>
        </div>
      )}
    </div>
  );
}
