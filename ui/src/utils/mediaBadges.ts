import type {
  DetailEnrichment,
  EpisodeEnrichment,
  VegaDirectLink,
  VegaLink,
} from "../types";

export interface RichCardMeta {
  displayTitle: string;
  thumbnailUrl: string;
  overview?: string;
  seasonNumber?: number;
  episodeNumber?: number;
  seasonLabel?: string;
  episodeLabel?: string;
  qualityBadge: string;
  sizeBadge?: string;
  durationLabel?: string;
  dateLabel?: string;
  ratingLabel?: string;
  formatTags: string[];
}

const QUALITY_RULES: Array<[RegExp, string]> = [
  [/\b2160p\b/i, "2160p"],
  [/\b(?:4k|uhd)\b/i, "4K"],
  [/\b1080p\b/i, "1080p"],
  [/\b720p\b/i, "720p"],
  [/\b480p\b/i, "480p"],
  [/\b360p\b/i, "360p"],
];

const TAG_RULES: Array<[RegExp, string]> = [
  [/\bdual[\s.\-_]*audio\b/i, "Dual Audio"],
  [/\bmulti[\s.\-_]*audio\b/i, "Multi Audio"],
  [/\bhindi\b/i, "Hindi"],
  [/\benglish\b/i, "English"],
  [/\b(?:hevc|x265|h\.?265)\b/i, "HEVC"],
  [/\b(?:x264|h\.?264|avc)\b/i, "x264"],
  [/\b10[\s.\-_]*bit\b/i, "10-bit"],
  [/\bweb[\s.\-_]*dl\b/i, "WEB-DL"],
  [/\bbluray\b/i, "BluRay"],
  [/\besubs?\b/i, "ESubs"],
];

const SEASON_RE = /\b(?:season|s)\s*[-:.]?\s*(\d{1,2})\b/i;
const SE_EP_RE = /\bS(\d{1,2})\s*[-:.]?\s*E(\d{1,3})\b/i;
const EP_RE = /\b(?:episode|ep|e)\s*[-:.#]?\s*(\d{1,3})\b/i;
const SIZE_RE = /\b(\d+(?:\.\d+)?\s*(?:TB|GB|MB|KB)(?:\s*\/\s*E(?:p)?)?)\b/i;

function pad2(n: number): string {
  return n < 10 ? `0${n}` : `${n}`;
}

function parsePositiveInt(raw?: string | number | null): number | undefined {
  if (raw == null) return undefined;
  if (typeof raw === "number") return Number.isFinite(raw) ? raw : undefined;
  const match = String(raw).match(/(\d+)/);
  if (!match) return undefined;
  const parsed = Number.parseInt(match[1], 10);
  return Number.isFinite(parsed) ? parsed : undefined;
}

function isBareEpisodeLabel(title: string): boolean {
  const t = title.trim();
  return (
    !t ||
    /^(?:s\d{1,2}\s*e\d{1,3}|episode\s*\d+|ep\.?\s*\d+|source(?:\s*\d+)?|movie)$/i.test(
      t
    )
  );
}

function findEpisodeEnrichment(
  enrichment: DetailEnrichment | null | undefined,
  seasonNumber?: number,
  episodeNumber?: number
): EpisodeEnrichment | undefined {
  if (!enrichment?.episodes?.length || episodeNumber == null) return undefined;
  return (
    enrichment.episodes.find(
      (e) =>
        e.episode === episodeNumber &&
        (seasonNumber == null || e.season == null || e.season === seasonNumber)
    ) ?? enrichment.episodes.find((e) => e.episode === episodeNumber)
  );
}

export function extractRichCardMeta(params: {
  rawTitle: string;
  linkUrl: string;
  item?: Partial<VegaLink | VegaDirectLink>;
  serverTitle?: string;
  fallbackImage?: string;
  fallbackBackdrop?: string;
  enrichment?: DetailEnrichment | null;
}): RichCardMeta {
  const {
    rawTitle,
    item,
    serverTitle = "",
    fallbackImage = "",
    fallbackBackdrop = "",
    enrichment,
  } = params;

  const cleanRaw = rawTitle.trim();
  const cleanServer = serverTitle.trim();
  const combined = `${cleanRaw} ${cleanServer} ${item?.quality ?? ""}`;

  const seMatch = cleanRaw.match(SE_EP_RE);
  const seasonNumber =
    parsePositiveInt(item?.seasonNumber) ??
    parsePositiveInt(seMatch?.[1]) ??
    parsePositiveInt(cleanRaw.match(SEASON_RE)?.[1]) ??
    parsePositiveInt(cleanServer.match(SEASON_RE)?.[1]);

  const episodeNumber =
    parsePositiveInt(item?.episodeNumber) ??
    parsePositiveInt(seMatch?.[2]) ??
    parsePositiveInt(cleanRaw.match(EP_RE)?.[1]);

  const epEnrich = findEpisodeEnrichment(enrichment, seasonNumber, episodeNumber);

  const explicitQuality = item?.quality?.trim();
  const matchedQuality = QUALITY_RULES.find(([re]) => re.test(combined))?.[1];
  const qualityBadge =
    explicitQuality ||
    matchedQuality ||
    (/\b(?:mkv|mp4|hd)\b/i.test(combined) ? "HD" : "HD");

  const sizeInTitle = cleanRaw.match(SIZE_RE)?.[1];
  const sizeInServer = cleanServer.match(SIZE_RE)?.[1];
  const sizeBadge =
    item?.size?.trim() || sizeInTitle || sizeInServer || undefined;

  const durationLabel =
    item?.duration?.trim() ||
    epEnrich?.duration?.trim() ||
    enrichment?.runtime?.trim() ||
    undefined;

  const dateLabel = item?.date?.trim() || epEnrich?.date?.trim() || undefined;
  const ratingLabel = item?.rating?.trim() || epEnrich?.rating?.trim() || undefined;
  const overview = item?.overview?.trim() || epEnrich?.overview?.trim() || undefined;

  const formatTags: string[] = [];
  for (const [re, label] of TAG_RULES) {
    if (re.test(combined) && !formatTags.includes(label)) {
      formatTags.push(label);
    }
    if (formatTags.length >= 3) break;
  }

  // Prefer enriched episode title when raw scraper title only reads "S1E1" / "Episode 1" / "Movie"
  let displayTitle = cleanRaw;
  if (sizeInTitle) {
    displayTitle = displayTitle
      .replace(new RegExp(`[\\s\\u2022\\-|/(\\[\\]]*${sizeInTitle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}[\\)\\]]*`, "gi"), " ")
      .replace(/\s+/g, " ")
      .trim();
  }
  if (isBareEpisodeLabel(displayTitle) && epEnrich?.title?.trim()) {
    displayTitle = epEnrich.title.trim();
  } else if (displayTitle.toLowerCase() === "movie" && cleanServer) {
    displayTitle = cleanServer;
  } else if (!displayTitle) {
    displayTitle = epEnrich?.title?.trim() || cleanServer || "Playable Source";
  }

  const thumbnailUrl =
    item?.image?.trim() ||
    epEnrich?.image?.trim() ||
    fallbackBackdrop.trim() ||
    fallbackImage.trim() ||
    "";

  return {
    displayTitle,
    thumbnailUrl,
    overview,
    seasonNumber,
    episodeNumber,
    seasonLabel: seasonNumber != null ? `S${pad2(seasonNumber)}` : undefined,
    episodeLabel: episodeNumber != null ? `EP ${pad2(episodeNumber)}` : undefined,
    qualityBadge,
    sizeBadge,
    durationLabel,
    dateLabel,
    ratingLabel,
    formatTags,
  };
}
