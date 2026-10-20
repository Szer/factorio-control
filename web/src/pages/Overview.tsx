import { useQuery } from "@tanstack/react-query";
import {
  ArrowRight,
  ArrowUpRight,
  Box,
  Check,
  Clock3,
  FileArchive,
  Plug,
  Settings2,
  Terminal,
  Users,
} from "lucide-react";
import { api, errorMessage, formatBytes, formatDate } from "../api";
import type { Status } from "../api";
import type { Page } from "../routes";
import { Notice, StatusBadge } from "../components/ui";
import ServerControls from "../components/ServerControls";

export default function Overview({
  status,
  navigate,
}: {
  status: Status;
  navigate: (page: Page) => void;
}) {
  const saves = useQuery({ queryKey: ["saves"], queryFn: api.saves });
  const settings = useQuery({ queryKey: ["settings"], queryFn: api.settings });
  const logs = useQuery({
    queryKey: ["logs"],
    queryFn: api.logs,
    refetchInterval: 2_000,
  });
  const running = status.state === "running";
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Server control</h1>
          <p>Manage your Factorio server.</p>
        </div>
        <ServerControls status={status} />
      </div>
      {status.error && <Notice>{status.error}</Notice>}
      <section className="hero-card">
        <div className="hero-copy">
          <div className="hero-kicker">
            <span className="tiny-dot" /> YOUR FACTORIO SERVER
          </div>
          <h2>{settings.data?.settings.name ?? "Your server"}</h2>
          <p>
            {settings.data?.settings.description || "Factorio headless server"}
          </p>
          <div className="hero-bottom">
            <StatusBadge state={status.state} />
            <span className="divider" />
            <span className="hero-version">
              {status.version
                ? `Factorio ${status.version}`
                : status.binary_available
                  ? "Headless server connected"
                  : "Factorio not connected"}
            </span>
          </div>
        </div>
      </section>
      <div className="stat-grid">
        <div className="stat-card">
          <div className="stat-label">
            <span>{running ? "Current world" : "Last world"}</span>
            <Box size={16} />
          </div>
          <div className="stat-value text-value">
            {status.active_save?.replace(/\.zip$/i, "") ?? "No world loaded"}
          </div>
          <span className="stat-caption">
            {status.active_save
              ? running
                ? "Loaded in this session"
                : "Most recently loaded save"
              : "Choose a save when you start"}
          </span>
        </div>
        <div className="stat-card">
          <div className="stat-label">
            <span>Saved worlds</span>
            <FileArchive size={16} />
          </div>
          <div className="stat-value">
            {saves.data?.length ?? "—"}
            <span>{saves.data?.length === 1 ? "world" : "worlds"}</span>
          </div>
          <span className="stat-caption">
            {saves.data
              ? `${formatBytes(saves.data.reduce((sum, save) => sum + save.size_bytes, 0))} on disk`
              : "Reading your saves…"}
          </span>
        </div>
        <div className="stat-card">
          <div className="stat-label">
            <span>Autosave interval</span>
            <Clock3 size={16} />
          </div>
          <div className="stat-value">
            {settings.data?.settings.autosave_interval === 0
              ? "Off"
              : (settings.data?.settings.autosave_interval ?? "—")}
            {settings.data?.settings.autosave_interval !== 0 && (
              <span>minutes</span>
            )}
          </div>
          <span className="stat-caption">
            {settings.data?.settings.auto_pause
              ? "Auto-pause when nobody is playing"
              : "Server keeps running without players"}
          </span>
        </div>
      </div>
      {settings.isError && <Notice>{errorMessage(settings.error)}</Notice>}
      <div className="overview-grid">
        <section className="panel">
          <div className="section-heading">
            <div>
              <h2>
                {!status.binary_available || saves.data?.length === 0
                  ? "Server setup"
                  : "Server management"}
              </h2>
            </div>
          </div>
          <div className="setup-list">
            <div className="setup-item">
              <span
                className={`step-icon ${status.binary_available ? "complete" : ""}`}
              >
                {status.binary_available ? (
                  <Check size={17} />
                ) : (
                  <Plug size={17} />
                )}
              </span>
              <div>
                <strong>Connect Factorio</strong>
                <p>
                  {status.binary_available
                    ? "Headless server is available."
                    : "Set FC_FACTORIO_BIN to your headless executable."}
                </p>
              </div>
              {status.binary_available && (
                <span className="completed-label">Ready</span>
              )}
            </div>
            <button
              className="setup-item actionable"
              onClick={() => {
                navigate("worlds");
              }}
            >
              <span
                className={`step-icon ${(saves.data?.length ?? 0) > 0 ? "complete" : ""}`}
              >
                <Box size={17} />
              </span>
              <span>
                <strong>Saved worlds</strong>
                <span className="item-description">
                  Upload or select a save.
                </span>
              </span>
              <ArrowRight size={17} />
            </button>
            <button
              className="setup-item actionable"
              onClick={() => {
                navigate("settings");
              }}
            >
              <span className="step-icon">
                <Settings2 size={17} />
              </span>
              <span>
                <strong>Server settings</strong>
                <span className="item-description">
                  Configure access and autosaves.
                </span>
              </span>
              <ArrowRight size={17} />
            </button>
          </div>
        </section>
        <section className="panel activity-panel">
          <div className="section-heading">
            <div>
              <h2>Recent activity</h2>
            </div>
            <button
              className="icon-button"
              aria-label="Open console"
              onClick={() => {
                navigate("console");
              }}
            >
              <ArrowUpRight size={19} />
            </button>
          </div>
          {logs.isError ? (
            <Notice>{errorMessage(logs.error)}</Notice>
          ) : (
            <div className="activity-list">
              {logs.data
                ?.slice(-4)
                .reverse()
                .map((log) => (
                  <div className="activity-item" key={log.id}>
                    <span className={`activity-dot ${log.level}`} />
                    <div>
                      <p>{log.message}</p>
                      <time>{formatDate(log.timestamp)}</time>
                    </div>
                  </div>
                ))}
              {logs.data?.length === 0 && (
                <div className="quiet-activity">
                  <Terminal size={23} />
                  <p>All quiet here.</p>
                  <span>Server events will appear as they happen.</span>
                </div>
              )}
            </div>
          )}
        </section>
      </div>
      <button
        className="invite-strip"
        onClick={() => {
          navigate("players");
        }}
      >
        <span className="invite-icon">
          <Users size={21} />
        </span>
        <span>
          <strong>Administrators</strong>
          <span>Manage who can administer the server.</span>
        </span>
        <ArrowRight size={19} />
      </button>
    </>
  );
}
