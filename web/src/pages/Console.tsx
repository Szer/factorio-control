import { useSessionMutation } from "../useSessionMutation";
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Pause, Play, RefreshCw, Search, Send, Terminal } from "lucide-react";
import { api, errorMessage } from "../api";
import { useAccess } from "../authApi";
import { operations } from "../operationsApi";
import { EmptyState, Loading, Notice } from "../components/ui";

export default function Console({
  interval,
  running = false,
}: {
  interval: number;
  running?: boolean;
}) {
  const { canOperate } = useAccess();
  const [source, setSource] = useState<"live" | "file">("live");
  const [command, setCommand] = useState("");
  const [acknowledge, setAcknowledge] = useState(false);
  const needsAcknowledgement =
    command.trimStart().startsWith("/") &&
    !/^\s*\/(?:help|h|players|p|admins|version|time|evolution|seed)(?:\s|$)/i.test(
      command,
    );
  const send = useSessionMutation({
    mutationFn: operations.command,
    onSuccess: () => {
      setCommand("");
      setAcknowledge(false);
    },
  });
  const file = useQuery({
    queryKey: ["file-log"],
    queryFn: operations.fileLog,
    enabled: source === "file",
    retry: false,
  });
  const [paused, setPaused] = useState(false);
  const [filter, setFilter] = useState("");
  const logs = useQuery({
    queryKey: ["logs"],
    queryFn: api.logs,
    refetchInterval: paused ? false : interval,
    enabled: !paused && source === "live",
    refetchOnWindowFocus: false,
  });
  const visible =
    logs.data?.filter((entry) =>
      entry.message.toLocaleLowerCase().includes(filter.toLocaleLowerCase()),
    ) ?? [];
  return (
    <>
      <div className="tab-buttons settings-tabs">
        <button
          className={source === "live" ? "button primary" : "button secondary"}
          onClick={() => {
            setSource("live");
          }}
        >
          Live console
        </button>
        <button
          className={source === "file" ? "button primary" : "button secondary"}
          onClick={() => {
            setSource("file");
          }}
        >
          Game log file
        </button>
      </div>
      {source === "file" ? (
        <section className="panel console-panel">
          <div className="section-heading">
            <h2>{file.data?.name ?? "factorio-current.log"}</h2>
            <button
              className="button secondary"
              disabled={file.isFetching}
              onClick={() => {
                file.refetch().catch(() => undefined);
              }}
            >
              <RefreshCw size={16} />
              Refresh
            </button>
          </div>
          {file.isPending ? (
            <Loading />
          ) : file.isError ? (
            <Notice>{errorMessage(file.error)}</Notice>
          ) : (
            <>
              <pre className="file-log">
                {file.data.content || "No game log output."}
              </pre>
              {file.data.truncated && (
                <p className="inline-info">
                  Showing the most recent part of the log.
                </p>
              )}
            </>
          )}
        </section>
      ) : (
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
      )}
      {source === "live" && canOperate && (
        <form
          className="panel command-form"
          onSubmit={(event) => {
            event.preventDefault();
            send.mutate({ command, acknowledge_scripting: acknowledge });
          }}
        >
          <div className="command-input">
            <label className="field">
              <span className="visually-hidden">
                Server command or chat message
              </span>
              <input
                aria-label="Server command or chat message"
                placeholder="Message or /command"
                value={command}
                maxLength={4096}
                disabled={!running || send.isPending}
                onChange={(event) => {
                  setCommand(event.target.value);
                  setAcknowledge(false);
                  send.reset();
                }}
              />
            </label>
            <button
              className="button primary"
              disabled={
                !running ||
                send.isPending ||
                !command.trim() ||
                (needsAcknowledgement && !acknowledge)
              }
            >
              <Send size={16} />
              {send.isPending ? "Sending…" : "Send"}
            </button>
          </div>
          {needsAcknowledgement && (
            <label className="command-warning">
              <input
                type="checkbox"
                checked={acknowledge}
                onChange={(event) => {
                  setAcknowledge(event.target.checked);
                }}
                disabled={send.isPending}
              />
              I accept that scripting commands can disable achievements and
              modify this save.
            </label>
          )}
          {!running && (
            <span className="muted">Start the server to send commands.</span>
          )}
          {send.isError && <Notice>{errorMessage(send.error)}</Notice>}
          {send.isSuccess && <Notice success>Command sent.</Notice>}
        </form>
      )}
    </>
  );
}
