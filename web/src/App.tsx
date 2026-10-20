import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  ArrowUpRight,
  Box,
  ChevronRight,
  CircleHelp,
  Factory,
  LayoutDashboard,
  Menu,
  Settings2,
  ShieldCheck,
  Terminal,
  Package,
  X,
} from "lucide-react";
import { api, errorMessage } from "./api";
import { Loading, Notice, StatusBadge } from "./components/ui";
import Overview from "./pages/Overview";
import Saves from "./pages/Saves";
import Mods from "./pages/Mods";
import SettingsPage from "./pages/Settings";
import Console from "./pages/Console";
import { readPage } from "./routes";
import type { Page } from "./routes";

const navigation = [
  { id: "overview", label: "Overview", icon: LayoutDashboard },
  { id: "saves", label: "Saves", icon: Box },
  { id: "mods", label: "Mods", icon: Package },
  { id: "settings", label: "Settings", icon: Settings2 },
  { id: "console", label: "Console", icon: Terminal },
] satisfies { id: Page; label: string; icon: typeof Factory }[];

export default function App() {
  const [page, setPage] = useState(readPage);
  const [menuOpen, setMenuOpen] = useState(false);
  const manager = useQuery({ queryKey: ["manager"], queryFn: api.manager });
  const interval = (manager.data?.settings.poll_interval_seconds ?? 2) * 1000;
  const status = useQuery({
    queryKey: ["status"],
    queryFn: api.status,
    refetchInterval: interval,
  });
  const settings = useQuery({ queryKey: ["settings"], queryFn: api.settings });
  useEffect(() => {
    const change = () => {
      setPage(readPage());
      setMenuOpen(false);
    };
    window.addEventListener("hashchange", change);
    return () => {
      window.removeEventListener("hashchange", change);
    };
  }, []);
  const title =
    navigation.find((item) => item.id === page)?.label ?? "Overview";
  return (
    <div className="app-shell">
      <aside className={`sidebar ${menuOpen ? "open" : ""}`}>
        <a
          className="brand"
          href="#overview"
          aria-label="Factorio Control home"
        >
          <span className="brand-mark">
            <Factory size={23} strokeWidth={1.6} />
          </span>
          <span>
            FACTORIO<span className="brand-sub">CONTROL</span>
          </span>
        </a>
        <div className="workspace-label">SERVER</div>
        <div className="workspace-card">
          <span className="workspace-symbol">
            {settings.data?.settings.name.charAt(0).toUpperCase() ?? "F"}
          </span>
          <div>
            <strong>{settings.data?.settings.name ?? "Your server"}</strong>
            <span>Local instance</span>
          </div>
          <ChevronRight size={14} />
        </div>
        <nav aria-label="Main navigation">
          {navigation.map(({ id, label, icon: Icon }) => (
            <a
              key={id}
              href={`#${id}`}
              className={page === id ? "nav-item active" : "nav-item"}
              aria-current={page === id ? "page" : undefined}
            >
              <Icon size={18} strokeWidth={1.7} />
              {label}
              {id === "console" && <span className="nav-shortcut">&gt;_</span>}
            </a>
          ))}
        </nav>
        <div className="sidebar-bottom">
          <div className="local-note">
            <ShieldCheck size={16} />
            <span>
              Local control
              <br />
              <small>Local instance</small>
            </span>
          </div>
          <a
            className="help-link"
            href="https://wiki.factorio.com/Multiplayer"
            target="_blank"
            rel="noreferrer"
          >
            <CircleHelp size={16} />
            Factorio server guide
            <ArrowUpRight size={13} />
          </a>
          <div className="build-label">
            <span className="tiny-dot" /> EARLY ACCESS <span>v0.1.0</span>
          </div>
        </div>
      </aside>
      {menuOpen && (
        <button
          className="mobile-backdrop"
          onClick={() => {
            setMenuOpen(false);
          }}
          aria-label="Close navigation"
        />
      )}
      <div className="main-shell">
        <header className="topbar">
          <div className="breadcrumb">
            <button
              className="icon-button mobile-menu"
              onClick={() => {
                setMenuOpen(!menuOpen);
              }}
              aria-label={
                menuOpen ? "Close navigation menu" : "Open navigation menu"
              }
            >
              {menuOpen ? <X size={20} /> : <Menu size={20} />}
            </button>
            <span>Server</span>
            <ChevronRight size={13} />
            <strong>{title}</strong>
          </div>
          <div className="topbar-right">
            <span className="local-badge">
              <span className={`tiny-dot ${status.isError ? "offline" : ""}`} />
              {status.isError ? "Connection lost" : "Local connection"}
            </span>
            <div className="avatar" aria-label="Local operator">
              OP
            </div>
          </div>
        </header>
        <main id="main-content">
          {status.data === undefined ? (
            status.isPending ? (
              <Loading />
            ) : (
              <div className="connection-error">
                <h1>Let’s reconnect.</h1>
                <Notice>{errorMessage(status.error)}</Notice>
                <p>
                  The control service is unavailable. Your saved files stay on
                  disk.
                </p>
                <button
                  onClick={() => {
                    status.refetch().catch(() => undefined);
                  }}
                  className="button primary"
                >
                  Try again
                </button>
              </div>
            )
          ) : (
            <>
              {status.isError && (
                <Notice>
                  Connection lost. Your edits are still here; actions will be
                  confirmed when the service responds.
                </Notice>
              )}
              {page !== "overview" && (
                <div className="page-heading">
                  <div>
                    <div className="eyebrow">CONTROL ROOM</div>
                    <h1>{title}</h1>
                    <p>
                      {page === "saves"
                        ? "Upload, select, and download saves."
                        : page === "settings"
                          ? "Server configuration, administrators, and manager preferences."
                          : page === "mods"
                            ? "Versions, updates, and compatibility."
                            : "Live server output and events."}
                    </p>
                  </div>
                  <StatusBadge state={status.data.state} />
                </div>
              )}
              {page === "overview" && (
                <Overview status={status.data} interval={interval} />
              )}
              {page === "saves" && <Saves status={status.data} />}
              <div hidden={page !== "settings"}>
                <SettingsPage />
              </div>
              <div hidden={page !== "mods"}>
                <Mods status={status.data} active={page === "mods"} />
              </div>
              {page === "console" && <Console interval={interval} />}
            </>
          )}
          <footer className="page-footer">
            <span>Factorio server management</span>
            <span>
              FACTORIO CONTROL <span className="footer-dot">·</span> LOCAL
            </span>
          </footer>
        </main>
      </div>
    </div>
  );
}
