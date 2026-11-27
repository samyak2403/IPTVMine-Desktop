import { useEffect, useState } from "react";
import { ArrowLeft, Check, Copy, RefreshCw } from "lucide-react";
import { api } from "../api";
import type { UpdateCheckResult } from "../types";

export function AboutView({ onBack }: { onBack: () => void }) {
  const [checking, setChecking] = useState(false);
  const [result, setResult] = useState<UpdateCheckResult | null>(null);
  const [error, setError] = useState("");

  async function checkUpdates() {
    setChecking(true);
    setError("");
    try {
      setResult(await api.checkForUpdates());
    } catch (err) {
      setError(String(err));
    } finally {
      setChecking(false);
    }
  }

  return (
    <div style={{ display: "grid", gap: 16 }}>
      <div className="page-header" style={{ marginBottom: 0 }}>
        <h1 className="page-title">About IPTVMine Desktop</h1>
        <button className="btn" onClick={onBack}>
          <ArrowLeft size={14} /> Back to Settings
        </button>
      </div>

      <div className="card" style={{ display: "grid", gap: 10 }}>
        <div>
          <strong>Version:</strong> 0.1.0 (Rust + Tauri v2 + React + TypeScript)
        </div>
        <div className="muted">
          Desktop edition of IPTVMine Pro supporting M3U/JSON Live TV, Vega JS/TS extensions, DoH DNS bypass, local downloads, and watch history.
        </div>
        <div>
          <button className="btn btn-primary" disabled={checking} onClick={() => void checkUpdates()}>
            <RefreshCw size={14} /> {checking ? "Checking GitHub..." : "Check for Updates"}
          </button>
        </div>
        {error && <div className="error-text">{error}</div>}
        {result && (
          <div className="card" style={{ background: "var(--bg-surface-elevated)", display: "grid", gap: 6 }}>
            <div>
              <strong>
                {result.updateAvailable
                  ? `Update Available: v${result.latestVersion}`
                  : `Up to date (latest v${result.latestVersion})`}
              </strong>
            </div>
            {result.releaseName && <div className="muted">{result.releaseName}</div>}
            {result.releaseNotes && (
              <pre style={{ margin: 0, whiteSpace: "pre-wrap", fontSize: 12 }} className="muted">
                {result.releaseNotes}
              </pre>
            )}
            {result.downloadUrl && (
              <a
                href={result.downloadUrl}
                target="_blank"
                rel="noreferrer"
                style={{ color: "var(--accent-secondary)" }}
              >
                Download {result.assetName || "Installer"}
              </a>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

export function BugReportView({ onBack }: { onBack: () => void }) {
  const [diagnostics, setDiagnostics] = useState("");
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    void api.getDiagnosticsBundle().then(setDiagnostics);
  }, []);

  async function copyDiagnostics() {
    await navigator.clipboard.writeText(diagnostics);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 2000);
  }

  return (
    <div style={{ display: "grid", gap: 16 }}>
      <div className="page-header" style={{ marginBottom: 0 }}>
        <h1 className="page-title">Diagnostics & Bug Report</h1>
        <button className="btn" onClick={onBack}>
          <ArrowLeft size={14} /> Back to Settings
        </button>
      </div>

      <div className="card" style={{ display: "grid", gap: 12 }}>
        <div className="muted">
          Redacted diagnostic summary (all provider URLs, cookies, and custom headers are omitted):
        </div>
        <pre
          style={{
            margin: 0,
            padding: 12,
            borderRadius: 8,
            background: "var(--bg-surface-elevated)",
            fontSize: 13,
            overflowX: "auto",
          }}
        >
          {diagnostics || "Loading..."}
        </pre>
        <div style={{ display: "flex", gap: 10 }}>
          <button className="btn btn-primary" onClick={() => void copyDiagnostics()}>
            {copied ? <Check size={14} /> : <Copy size={14} />}
            {copied ? "Copied" : "Copy Diagnostics"}
          </button>
          <a
            className="btn"
            href="https://github.com/samyak2403/IPTVMine-Pro/issues/new"
            target="_blank"
            rel="noreferrer"
            style={{ textDecoration: "none" }}
          >
            Open GitHub Issue
          </a>
        </div>
      </div>
    </div>
  );
}

export function LegalView({ onBack }: { onBack: () => void }) {
  const [tab, setTab] = useState<"disclaimer" | "privacy" | "terms">("disclaimer");

  return (
    <div style={{ display: "grid", gap: 16 }}>
      <div className="page-header" style={{ marginBottom: 0 }}>
        <h1 className="page-title">Legal & Disclaimers</h1>
        <button className="btn" onClick={onBack}>
          <ArrowLeft size={14} /> Back to Settings
        </button>
      </div>

      <div className="chip-row" style={{ marginBottom: 0 }}>
        <button className={`chip ${tab === "disclaimer" ? "active" : ""}`} onClick={() => setTab("disclaimer")}>
          Disclaimer
        </button>
        <button className={`chip ${tab === "privacy" ? "active" : ""}`} onClick={() => setTab("privacy")}>
          Privacy Policy
        </button>
        <button className={`chip ${tab === "terms" ? "active" : ""}`} onClick={() => setTab("terms")}>
          Terms & Conditions
        </button>
      </div>

      <div className="card" style={{ lineHeight: 1.6 }}>
        {tab === "disclaimer" && (
          <p style={{ margin: 0 }}>
            IPTVMine Desktop does not host, store, index, or distribute any media content. All playlists, Vega extension scripts, and video streams are configured and sourced solely by the user. Users are responsible for verifying that they have the right to access any third-party sources they configure.
          </p>
        )}
        {tab === "privacy" && (
          <p style={{ margin: 0 }}>
            IPTVMine Desktop stores all providers, watch history, download queues, and preferences locally on your computer in SQLite. The application includes zero analytics SDKs, zero tracking, and zero telemetry uploaders. Network requests are sent only to the provider/DNS endpoints you configure and optionally to GitHub Releases when you check for updates.
          </p>
        )}
        {tab === "terms" && (
          <p style={{ margin: 0 }}>
            IPTVMine Desktop is provided &ldquo;as is&rdquo; under the MIT License without warranty of any kind. Third-party Vega JavaScript extensions execute inside a sandboxed runtime, and you are solely responsible for trusting any external provider repositories you install.
          </p>
        )}
      </div>
    </div>
  );
}
