import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Search, RefreshCw } from "lucide-react";
import { api } from "../api";
import { PosterCard, useAdaptiveCardLayout } from "../components/OttUi";
import type { VegaCatalog, VegaPost, VegaProvider } from "../types";

function postKey(post: VegaPost): string {
  return post.link.trim() || post.title.trim();
}

function mergeUniquePosts(existing: VegaPost[], incoming: VegaPost[]): {
  merged: VegaPost[];
  addedCount: number;
} {
  const seen = new Set(existing.map(postKey));
  const added: VegaPost[] = [];
  for (const item of incoming) {
    const k = postKey(item);
    if (!k || seen.has(k)) continue;
    seen.add(k);
    added.push(item);
  }
  return {
    merged: existing.length === 0 ? added : [...existing, ...added],
    addedCount: added.length,
  };
}

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
  const [committedQuery, setCommittedQuery] = useState("");
  const [isSearching, setIsSearching] = useState(false);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [hasMore, setHasMore] = useState(true);
  const [error, setError] = useState("");
  const [loadMoreError, setLoadMoreError] = useState("");

  const postsRef = useRef<VegaPost[]>([]);
  postsRef.current = posts;
  const requestSeqRef = useRef(0);
  const loadMoreInFlightRef = useRef(false);
  const loadMoreSentinelRef = useRef<HTMLDivElement>(null);

  const explicitRatios = useMemo(() => posts.map((p) => p.aspectRatio), [posts]);
  const { layout: gridLayout, recordMeasure: recordGridMeasure } = useAdaptiveCardLayout(
    `${selectedScraper}|${selectedFilter}|${isSearching ? committedQuery : ""}`,
    explicitRatios
  );

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
      const seq = ++requestSeqRef.current;
      loadMoreInFlightRef.current = false;
      setLoading(true);
      setLoadingMore(false);
      setError("");
      setLoadMoreError("");
      setIsSearching(false);
      setSearchQuery("");
      setCommittedQuery("");
      postsRef.current = [];
      setPosts([]);
      setHasMore(true);
      try {
        const res = await api.getVodCatalog(selectedScraper);
        if (seq !== requestSeqRef.current) return;
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
        if (seq !== requestSeqRef.current) return;
        const { merged } = mergeUniquePosts([], initialPosts);
        postsRef.current = merged;
        setPosts(merged);
        setHasMore(merged.length > 0);
      } catch (err) {
        if (seq !== requestSeqRef.current) return;
        setError(String(err));
        postsRef.current = [];
        setPosts([]);
        setHasMore(false);
      } finally {
        if (seq === requestSeqRef.current) {
          setLoading(false);
        }
      }
    }
    void loadCatalog();
  }, [selectedScraper, initialFilter]);

  async function selectCategory(filter: string) {
    const seq = ++requestSeqRef.current;
    loadMoreInFlightRef.current = false;
    setSelectedFilter(filter);
    setIsSearching(false);
    setCommittedQuery("");
    setPage(1);
    postsRef.current = [];
    setPosts([]);
    setLoading(true);
    setLoadingMore(false);
    setError("");
    setLoadMoreError("");
    setHasMore(true);
    try {
      const initialPosts = await api.getVodPosts(selectedScraper, filter, 1);
      if (seq !== requestSeqRef.current) return;
      const { merged } = mergeUniquePosts([], initialPosts);
      postsRef.current = merged;
      setPosts(merged);
      setHasMore(merged.length > 0);
    } catch (err) {
      if (seq !== requestSeqRef.current) return;
      setError(String(err));
      setHasMore(false);
    } finally {
      if (seq === requestSeqRef.current) {
        setLoading(false);
      }
    }
  }

  async function runSearch(e: React.FormEvent) {
    e.preventDefault();
    const query = searchQuery.trim();
    if (!selectedScraper || !query) return;
    const seq = ++requestSeqRef.current;
    loadMoreInFlightRef.current = false;
    setIsSearching(true);
    setCommittedQuery(query);
    setPage(1);
    postsRef.current = [];
    setPosts([]);
    setLoading(true);
    setLoadingMore(false);
    setError("");
    setLoadMoreError("");
    setHasMore(true);
    try {
      const initialPosts = await api.searchVodPosts(selectedScraper, query, 1);
      if (seq !== requestSeqRef.current) return;
      const { merged } = mergeUniquePosts([], initialPosts);
      postsRef.current = merged;
      setPosts(merged);
      setHasMore(merged.length > 0);
    } catch (err) {
      if (seq !== requestSeqRef.current) return;
      setError(String(err));
      setHasMore(false);
    } finally {
      if (seq === requestSeqRef.current) {
        setLoading(false);
      }
    }
  }

  const loadMore = useCallback(async () => {
    if (!selectedScraper || loading || loadMoreInFlightRef.current || !hasMore) return;
    const seq = requestSeqRef.current;
    const nextPage = page + 1;
    const activeScraper = selectedScraper;
    const activeFilter = selectedFilter;
    const activeQuery = committedQuery;
    const searching = isSearching;

    loadMoreInFlightRef.current = true;
    setLoadingMore(true);
    setLoadMoreError("");
    try {
      const more = searching
        ? await api.searchVodPosts(activeScraper, activeQuery, nextPage)
        : await api.getVodPosts(activeScraper, activeFilter, nextPage);
      if (seq !== requestSeqRef.current) return;
      const { merged, addedCount } = mergeUniquePosts(postsRef.current, more);
      if (addedCount > 0) {
        postsRef.current = merged;
        setPosts(merged);
        setPage(nextPage);
      } else {
        setHasMore(false);
      }
    } catch (err) {
      if (seq !== requestSeqRef.current) return;
      setLoadMoreError(String(err));
    } finally {
      if (seq === requestSeqRef.current) {
        loadMoreInFlightRef.current = false;
        setLoadingMore(false);
      }
    }
  }, [selectedScraper, loading, hasMore, page, selectedFilter, searchQuery, isSearching]);

  useEffect(() => {
    const sentinel = loadMoreSentinelRef.current;
    if (!sentinel || !hasMore || loading || loadingMore || loadMoreError || posts.length === 0) {
      return;
    }
    const root = sentinel.closest(".ott-main");
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          void loadMore();
        }
      },
      { root, rootMargin: "360px 0px" }
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [hasMore, loading, loadingMore, loadMoreError, posts.length, loadMore]);

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
        <div className={`grid-posters${gridLayout === "banner" ? " is-banner-grid" : ""}`}>
          {Array.from({ length: 12 }).map((_, i) => (
            <div
              key={i}
              className="shimmer"
              style={{ aspectRatio: gridLayout === "banner" ? "16 / 9" : "2 / 3", borderRadius: 14 }}
            />
          ))}
        </div>
      ) : (
        <div className={`grid-posters${gridLayout === "banner" ? " is-banner-grid" : ""}`}>
          {posts.map((post) => (
            <PosterCard
              key={postKey(post)}
              measureId={postKey(post)}
              className="poster-card"
              title={post.title}
              image={post.image}
              layout={gridLayout}
              onMeasureAspect={recordGridMeasure}
              onClick={() => onSelectPost(post, selectedScraper)}
            />
          ))}
        </div>
      )}

      {posts.length > 0 && (
        <div
          ref={loadMoreSentinelRef}
          style={{
            marginTop: 24,
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            gap: 10,
          }}
        >
          {loadMoreError && (
            <div className="muted" style={{ fontSize: 13 }}>
              Failed to load more: {loadMoreError}
            </div>
          )}
          {hasMore ? (
            <button
              type="button"
              className="btn"
              onClick={() => void loadMore()}
              disabled={loading || loadingMore}
            >
              {loadingMore ? (
                <>
                  <RefreshCw size={14} className="ott-spin" /> Loading more...
                </>
              ) : loadMoreError ? (
                "Retry Load More"
              ) : (
                "Load More"
              )}
            </button>
          ) : (
            <div className="muted" style={{ fontSize: 13 }}>
              All {posts.length} titles loaded
            </div>
          )}
        </div>
      )}
    </div>
  );
}
