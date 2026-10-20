import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Box,
  ChevronRight,
  Factory,
  LayoutDashboard,
  Menu,
  Settings2,
  Terminal,
  Package,
  X,
} from "lucide-react";
import { AuthGate, SessionMenu } from "./components/Auth";
import { useAccess } from "./authApi";
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
  return (
    <AuthGate>
      <AppContent />
    </AuthGate>
  );
}
function AppContent() {
  const { canAdmin } = useAccess();
  const [requestedPage, setPage] = useState(readPage);
  const page =
    requestedPage === "settings" && !canAdmin ? "overview" : requestedPage;
  const [menuOpen, setMenuOpen] = useState(false);
  const manager = useQuery({
    queryKey: ["manager"],
    queryFn: api.manager,
    enabled: canAdmin,
  });
  const interval = (manager.data?.settings.poll_interval_seconds ?? 2) * 1000;
  const status = useQuery({
    queryKey: ["status"],
    queryFn: api.status,
    refetchInterval: interval,
  });
  const client = useQueryClient();
  const previousState = useRef(status.data?.state);
  useEffect(() => {
    const state = status.data?.state;
    if (
      previousState.current &&
      state !== previousState.current &&
      (state === "stopped" || state === "failed")
    ) {
      Promise.all(
        ["mods", "mod-settings", "saves", "selection"].map((key) =>
          client.invalidateQueries({ queryKey: [key] }),
        ),
      ).catch(() => undefined);
    }
    previousState.current = state;
  }, [status.data?.state, client]);
  const settings = useQuery({
    queryKey: ["settings"],
    queryFn: api.settings,
    enabled: canAdmin,
  });
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
        <div className="workspace-card">
          <span className="workspace-symbol">
            {settings.data?.settings.name.charAt(0).toUpperCase() ?? "F"}
          </span>
          <div>
            <strong>{settings.data?.settings.name ?? "Your server"}</strong>
          </div>
        </div>
        <nav aria-label="Main navigation">
          {navigation
            .filter((item) => canAdmin || item.id !== "settings")
            .map(({ id, label, icon: Icon }) => (
              <a
                key={id}
                href={`#${id}`}
                className={page === id ? "nav-item active" : "nav-item"}
                aria-current={page === id ? "page" : undefined}
              >
                <Icon size={18} strokeWidth={1.7} />
                {label}
              </a>
            ))}
        </nav>
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
          <SessionMenu />
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
                    <h1>{title}</h1>
                  </div>
                  <StatusBadge state={status.data.state} />
                </div>
              )}
              {page === "overview" && (
                <Overview status={status.data} interval={interval} />
              )}
              {page === "saves" && <Saves status={status.data} />}
              {canAdmin && (
                <div hidden={page !== "settings"}>
                  <SettingsPage />
                </div>
              )}
              <div hidden={page !== "mods"}>
                <Mods status={status.data} active={page === "mods"} />
              </div>
              {page === "console" && (
                <Console
                  interval={interval}
                  running={
                    status.data.state === "running" ||
                    status.data.state === "paused"
                  }
                />
              )}
            </>
          )}
        </main>
      </div>
    </div>
  );
}
