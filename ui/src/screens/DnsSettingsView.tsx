import { useCallback, useId, useMemo } from "react";
import {
  Activity,
  Check,
  Globe,
  Lock,
  RefreshCw,
  Server,
  Shield,
  ShieldOff,
  X,
} from "lucide-react";
import type { DnsMode, DnsTestResult } from "../types";

interface DnsOptionMeta {
  mode: DnsMode;
  label: string;
  subtitle: string;
  endpoint: string;
  badge?: string;
  badgeTone?: "amber" | "green" | "muted";
  icon: typeof Lock;
}

const DNS_OPTIONS: DnsOptionMeta[] = [
  {
    mode: "cloudflare",
    label: "Cloudflare (1.1.1.1)",
    subtitle: "Low-latency encrypted lookups using pinned 1.1.1.1 & 1.0.0.1 bootstrap IPs.",
    endpoint: "cloudflare-dns.com/dns-query",
    badge: "Default",
    badgeTone: "amber",
    icon: Shield,
  },
  {
    mode: "google",
    label: "Google Public DNS",
    subtitle: "High-availability global Anycast DoH resolution via 8.8.8.8 & 8.8.4.4.",
    endpoint: "dns.google/resolve",
    icon: Globe,
  },
  {
    mode: "adguard",
    label: "AdGuard DNS",
    subtitle: "Encrypted DNS that blocks known ad, tracker, and telemetry domains.",
    endpoint: "dns.adguard-dns.com/resolve",
    badge: "Ad Blocking",
    badgeTone: "green",
    icon: Lock,
  },
  {
    mode: "custom",
    label: "Custom DoH Resolver",
    subtitle: "Connect using your own HTTPS JSON-over-DNS resolver endpoint.",
    endpoint: "https://.../dns-query",
    icon: Server,
  },
  {
    mode: "off",
    label: "System DNS (Off)",
    subtitle: "Use your operating system DNS without HTTPS encryption or ISP block bypass.",
    endpoint: "OS Default Resolver",
    badge: "Unencrypted",
    badgeTone: "muted",
    icon: ShieldOff,
  },
];

function validateCustomDohUrl(raw: string): { valid: boolean; reason?: string } {
  const trimmed = raw.trim();
  if (!trimmed) {
    return { valid: false, reason: "Enter an HTTPS DoH URL (for example https://dns.quad9.net:5053/dns-query)." };
  }
  if (!trimmed.toLowerCase().startsWith("https://")) {
    return { valid: false, reason: "DoH endpoints must begin with https:// to encrypt queries." };
  }
  try {
    const parsed = new URL(trimmed);
    if (!parsed.hostname) {
      return { valid: false, reason: "Enter a valid resolver hostname." };
    }
    return { valid: true };
  } catch {
    return { valid: false, reason: "Enter a well-formed URL starting with https://." };
  }
}

function getLatencyTier(ms: number): string {
  if (ms <= 70) return "Optimal";
  if (ms <= 180) return "Good";
  return "Elevated";
}

interface Props {
  dnsMode: DnsMode;
  customDohUrl: string;
  dnsTest: DnsTestResult | null;
  testingDns: boolean;
  onSelectMode: (mode: DnsMode) => void;
  onCustomUrlChange: (url: string) => void;
  onRunTest: (mode: DnsMode) => void;
}

export function DnsSettingsView({
  dnsMode,
  customDohUrl,
  dnsTest,
  testingDns,
  onSelectMode,
  onCustomUrlChange,
  onRunTest,
}: Props) {
  const headingId = useId();
  const customInputId = useId();
  const activeOption = useMemo(
    () => DNS_OPTIONS.find((o) => o.mode === dnsMode) ?? DNS_OPTIONS[0],
    [dnsMode]
  );

  const customValidation = useMemo(
    () => validateCustomDohUrl(customDohUrl),
    [customDohUrl]
  );

  const handleGroupKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLButtonElement>, idx: number) => {
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      e.preventDefault();
      const delta = e.key === "ArrowDown" ? 1 : -1;
      const nextIdx = (idx + delta + DNS_OPTIONS.length) % DNS_OPTIONS.length;
      onSelectMode(DNS_OPTIONS[nextIdx].mode);
      const group = e.currentTarget.closest('[role="radiogroup"]');
      const nextRadio = group?.querySelectorAll<HTMLButtonElement>('[role="radio"]')[nextIdx];
      nextRadio?.focus();
    },
    [onSelectMode]
  );

  const canRunTest =
    !testingDns && (dnsMode !== "custom" || customValidation.valid);

  return (
    <section className="dns-screen" aria-labelledby={headingId}>
      <header className="dns-header">
        <div className="dns-title-block">
          <h1 id={headingId} className="dns-title">
            DNS over HTTPS
          </h1>
          <p className="dns-subtitle">
            Encrypt hostname lookups inside IPTVMine Desktop to prevent ISP DNS tampering and speed up stream resolution.
          </p>
        </div>

        <div
          className={`dns-status-pill ${
            dnsMode === "off" || (dnsMode === "custom" && !customValidation.valid)
              ? "is-unencrypted"
              : "is-encrypted"
          }`}
          aria-live="polite"
        >
          <span className="dns-status-dot" aria-hidden="true" />
          <span className="dns-status-text">
            {dnsMode === "off"
              ? "System DNS Active"
              : dnsMode === "custom" && !customValidation.valid
                ? "Custom URL Required"
                : `Encrypted • ${activeOption.label}`}
          </span>
          {dnsTest?.success && (
            <span className="dns-status-ms">{dnsTest.latencyMs} ms</span>
          )}
        </div>
      </header>

      <div
        className="dns-group-surface"
        role="radiogroup"
        aria-label="DNS over HTTPS resolver"
      >
        {DNS_OPTIONS.map((opt, idx) => {
          const selected = dnsMode === opt.mode;
          const Icon = opt.icon;
          const isCustomRow = opt.mode === "custom";
          const displayEndpoint =
            isCustomRow && customDohUrl.trim()
              ? customDohUrl.trim()
              : opt.endpoint;

          return (
            <div
              key={opt.mode}
              className={`dns-row-wrap ${selected ? "is-selected" : ""}`}
            >
              <button
                type="button"
                role="radio"
                aria-checked={selected}
                className="dns-row"
                onClick={() => onSelectMode(opt.mode)}
                onKeyDown={(e) => handleGroupKeyDown(e, idx)}
              >
                <span className="dns-row-icon" aria-hidden="true">
                  <Icon size={18} strokeWidth={2} />
                </span>

                <span className="dns-row-body">
                  <span className="dns-row-top">
                    <span className="dns-row-title">{opt.label}</span>
                    {opt.badge && (
                      <span
                        className={`dns-row-badge tone-${opt.badgeTone ?? "amber"}`}
                      >
                        {opt.badge}
                      </span>
                    )}
                  </span>
                  <span className="dns-row-desc">{opt.subtitle}</span>
                  <span className="dns-row-endpoint">{displayEndpoint}</span>
                </span>

                <span
                  className={`dns-radio-indicator ${selected ? "checked" : ""}`}
                  aria-hidden="true"
                >
                  <Check size={13} strokeWidth={3} />
                </span>
              </button>

              {isCustomRow && (
                <div
                  className={`dns-custom-drawer ${selected ? "open" : ""}`}
                  aria-hidden={!selected}
                >
                  <div className="dns-custom-drawer-inner">
                    <label className="dns-custom-label" htmlFor={customInputId}>
                      Resolver Endpoint (HTTPS JSON)
                    </label>
                    <div className="dns-custom-input-row">
                      <input
                        id={customInputId}
                        type="url"
                        className="input dns-custom-input"
                        tabIndex={selected ? 0 : -1}
                        placeholder="https://dns.example.com/dns-query"
                        value={customDohUrl}
                        onChange={(e) => onCustomUrlChange(e.target.value)}
                      />
                      {customDohUrl.length > 0 && (
                        <button
                          type="button"
                          className="dns-clear-btn"
                          tabIndex={selected ? 0 : -1}
                          aria-label="Clear custom resolver URL"
                          onClick={() => onCustomUrlChange("")}
                        >
                          <X size={14} />
                        </button>
                      )}
                    </div>
                    {!customValidation.valid && customValidation.reason && (
                      <div className="dns-inline-hint is-warn" role="status">
                        {customValidation.reason}
                      </div>
                    )}
                  </div>
                </div>
              )}
            </div>
          );
        })}
      </div>

      <div className="dns-diagnostics-card">
        <div className="dns-diagnostics-info" aria-live="polite">
          <div className="dns-diagnostics-label">
            <Activity size={16} aria-hidden="true" />
            <span>Resolver Diagnostics</span>
          </div>

          {testingDns ? (
            <p className="dns-diagnostics-msg">
              Sending test query to {activeOption.label}...
            </p>
          ) : dnsTest ? (
            dnsTest.success ? (
              <p className="dns-diagnostics-msg is-ok">
                Connected in <strong className="dns-tabular">{dnsTest.latencyMs} ms</strong>
                {" "}({getLatencyTier(dnsTest.latencyMs)} response)
              </p>
            ) : (
              <p className="dns-diagnostics-msg is-error">
                {dnsTest.latencyMs === -1
                  ? "Enter a valid custom HTTPS resolver URL before testing."
                  : "DNS lookup failed — verify connectivity or try another resolver."}
              </p>
            )
          ) : dnsMode === "off" ? (
            <p className="dns-diagnostics-msg is-warn">
              DoH encryption is turned off. Queries are sent over your network's standard DNS.
            </p>
          ) : (
            <p className="dns-diagnostics-msg">
              Verify end-to-end resolution speed and reachability for {activeOption.label}.
            </p>
          )}
        </div>

        <button
          type="button"
          className="btn btn-primary dns-test-btn"
          disabled={!canRunTest}
          onClick={() => onRunTest(dnsMode)}
        >
          <RefreshCw size={14} className={testingDns ? "dns-spin" : ""} />
          <span>{testingDns ? "Testing..." : "Test DNS Latency"}</span>
        </button>
      </div>

      <footer className="dns-footnote">
        Bootstrap endpoints are pre-pinned to IP addresses and results are cached in memory for 5 minutes so blocked ISP DNS cannot interrupt discovery or streaming.
      </footer>
    </section>
  );
}
