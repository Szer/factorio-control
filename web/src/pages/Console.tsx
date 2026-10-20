import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Pause, Play, Search, Terminal } from "lucide-react";
import { api, errorMessage } from "../api";
import { EmptyState, Loading, Notice } from "../components/ui";

export default function Console({ interval }: { interval: number }) {
  const [paused, setPaused] = useState(false);
  const [filter, setFilter] = useState("");
  const logs = useQuery({
    queryKey: ["logs"],
    queryFn: api.logs,
    refetchInterval: paused ? false : interval,
    enabled: !paused,
    refetchOnWindowFocus: false,
  });
  const visible =
    logs.data?.filter((entry) =>
      entry.message.toLocaleLowerCase().includes(filter.toLocaleLowerCase()),
    ) ?? [];
  return (
    <>
      <section className="panel console-panel">
        <div className="console-toolbar">
          <span className="console-title">
            <Terminal size={18} />
            Server output
            <span className={`live-label ${paused ? "paused" : ""}`}>
              <span className="tiny-dot" />
              {paused ? "PAUSED" : "LIVE"}
            </span>
          </span>
          <div>
            <label className="search-box">
              <Search size={16} />
              <input
                aria-label="Filter logs"
                placeholder="Filter output…"
                value={filter}
                onChange={(event) => {
                  setFilter(event.target.value);
                }}
              />
            </label>
            <button
              className="icon-button"
              aria-label={paused ? "Resume updates" : "Pause updates"}
              title={paused ? "Resume updates" : "Pause updates"}
              onClick={() => {
                setPaused(!paused);
              }}
            >
              {paused ? <Play size={17} /> : <Pause size={17} />}
            </button>
          </div>
        </div>
        {logs.isError && <Notice>{errorMessage(logs.error)}</Notice>}
        {logs.isPending ? (
          <Loading />
        ) : visible.length === 0 ? (
          <EmptyState
            icon={<Terminal size={28} />}
            title={filter ? "No matching output." : "No output yet."}
          />
        ) : (
          <div
            className="log-output"
            role="log"
            aria-label="Server log"
            aria-live="off"
          >
            {visible.map((entry) => (
              <div className="log-line" key={entry.id}>
                <time>
                  {new Date(entry.timestamp * 1000).toLocaleTimeString([], {
                    hour12: false,
                  })}
                </time>
                <span className={`log-level ${entry.level}`}>
                  {entry.level.toUpperCase()}
                </span>
                <span>{entry.message}</span>
              </div>
            ))}
          </div>
        )}
        <div className="table-footer">
          <span>{visible.length} entries</span>
          <span>
            {paused
              ? "Updates paused. The server keeps running."
              : "Recent output · up to 2,000 entries"}
          </span>
        </div>
      </section>
    </>
  );
}
