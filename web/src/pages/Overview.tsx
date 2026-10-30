import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Activity, FlaskConical, Users } from "lucide-react";
import { useAccess } from "../authApi";
import { api, errorMessage, formatDate } from "../api";
import type { Status, Telemetry } from "../api";
import { Notice, StatusBadge } from "../components/ui";
import ServerControls from "../components/ServerControls";

function duration(since: number, now: number): string {
  const seconds = Math.max(0, now - since);
  return seconds < 60
    ? `${String(seconds)}s`
    : seconds < 3600
      ? `${String(Math.floor(seconds / 60))}m`
      : seconds < 86400
        ? `${String(Math.floor(seconds / 3600))}h ${String(Math.floor((seconds % 3600) / 60))}m`
        : `${String(Math.floor(seconds / 86400))}d ${String(Math.floor((seconds % 86400) / 3600))}h`;
}
export function UpsChart({
  telemetry,
  now,
}: {
  telemetry: Telemetry;
  now: number;
}) {
  const [minutes, setMinutes] = useState(5);
  const end = now;
  const start = end - minutes * 60;
  const first = telemetry.history.findIndex(
    (point) => point.timestamp >= start,
  );
  const samples = telemetry.history.filter(
    (point) => point.timestamp >= start && point.timestamp <= end,
  );
  const preceding = first > 0 ? telemetry.history[first - 1] : undefined;
  const firstVisible = samples[0];
  const targetSamples =
    preceding && firstVisible && firstVisible.timestamp > start
      ? [{ ...preceding, timestamp: start }, ...samples]
      : samples;
  const max = Math.max(
    60,
    ...samples.map((point) => point.ups ?? 0),
    ...targetSamples.map((point) => point.target_ups ?? 0),
  );
  const width = 900,
    height = 160,
    left = 0,
    right = 0,
    top = 0,
    bottom = 0;
  const x = (timestamp: number) =>
    left + ((timestamp - start) / (end - start)) * (width - left - right);
  const y = (value: number) =>
    height - bottom - (value / max) * (height - top - bottom);
  let path = "";
  let move = true;
  for (const point of samples) {
    if (point.ups === null || point.state !== "active") {
      move = true;
      continue;
    }
    path += `${move ? "M" : "L"}${x(point.timestamp).toFixed(2)},${y(point.ups).toFixed(2)} `;
    move = false;
  }
  let targetPath = "";
  let previousTarget: number | null = null;
  const targetMarkers: Telemetry["history"] = [];
  for (const point of targetSamples) {
    if (point.target_ups === null || point.state === "unavailable") {
      previousTarget = null;
      continue;
    }
    const position = x(point.timestamp).toFixed(2);
    const level = y(point.target_ups).toFixed(2);
    targetPath +=
      previousTarget === null
        ? `M${position},${level} `
        : `H${position} V${level} `;
    if (previousTarget !== point.target_ups) targetMarkers.push(point);
    previousTarget = point.target_ups;
  }
  const values = samples.flatMap((point) =>
    point.state === "active" && point.ups !== null ? [point.ups] : [],
  );
  const average =
    values.length > 0
      ? values.reduce((sum, value) => sum + value, 0) / values.length
      : null;
  return (
    <section className="panel ups-panel">
      <div className="section-heading">
        <h2>
          <Activity size={20} />
          UPS tracker
        </h2>
        <div className="chart-ranges" aria-label="UPS time range">
          {[5, 15, 60].map((value) => (
            <button
              key={value}
              className={minutes === value ? "range active" : "range"}
              aria-pressed={minutes === value}
              onClick={() => {
                setMinutes(value);
              }}
            >
              {value < 60 ? `${String(value)}m` : "1h"}
            </button>
          ))}
        </div>
      </div>
      <div className="ups-readings">
        <div>
          <span>Current</span>
          <strong>
            {telemetry.ups === null ? "—" : telemetry.ups.toFixed(1)}
            <small>UPS</small>
          </strong>
        </div>
        <div>
          <span>Target</span>
          <strong>
            {telemetry.target_ups === null
              ? "—"
              : telemetry.target_ups.toFixed(0)}
          </strong>
        </div>
        <div>
          <span>Average</span>
          <strong>{average === null ? "—" : average.toFixed(1)}</strong>
        </div>
        <div>
          <span>Minimum</span>
          <strong>
            {values.length === 0 ? "—" : Math.min(...values).toFixed(1)}
          </strong>
        </div>
      </div>
      <div className="chart-legend" aria-label="UPS chart legend">
        <span>
          <i className="legend-measured" aria-hidden="true" />
          Measured UPS
        </span>
        <span>
          <i className="legend-target" aria-hidden="true" />
          Target UPS
        </span>
      </div>
      <div className="ups-chart">
        <div className="chart-axis" aria-hidden="true">
          <span>{max.toFixed(0)}</span>
          <span>{(max / 2).toFixed(0)}</span>
          <span>0</span>
        </div>
        <svg
          viewBox={`0 0 ${String(width)} ${String(height)}`}
          preserveAspectRatio="none"
          role="img"
          aria-label={`UPS over the last ${String(minutes)} minutes. Current ${telemetry.ups === null ? "unavailable" : telemetry.ups.toFixed(1)} UPS.`}
        >
          {[0, max / 2, max].map((value) => (
            <g key={value}>
              <line
                x1={left}
                x2={width - right}
                y1={y(value)}
                y2={y(value)}
                className="chart-grid"
              />
            </g>
          ))}
          <path d={targetPath} className="chart-target" />
          {targetMarkers.map((point) => (
            <circle
              key={point.timestamp}
              cx={x(point.timestamp)}
              cy={y(point.target_ups ?? 0)}
              r={2.5}
              className="chart-target-point"
            >
              <title>
                {formatDate(point.timestamp)} · Target{" "}
                {point.target_ups?.toFixed(1)} UPS
              </title>
            </circle>
          ))}
          <path d={path} className="chart-line" />
          {samples
            .filter((point) => point.state === "active" && point.ups !== null)
            .map((point) => (
              <circle
                key={point.timestamp}
                cx={x(point.timestamp)}
                cy={y(point.ups ?? 0)}
                r={2}
                className="chart-point"
              >
                <title>
                  {formatDate(point.timestamp)} · {point.ups?.toFixed(1)} UPS
                </title>
              </circle>
            ))}
        </svg>
        <div className="chart-times">
          <span>{minutes} minutes ago</span>
          <span>Now</span>
        </div>
        {values.length === 0 && targetMarkers.length === 0 && (
          <div className="chart-empty">
            {!telemetry.enabled
              ? "Scripting telemetry is disabled"
              : telemetry.available
                ? "Waiting for active tick samples"
                : "Waiting for server telemetry"}
          </div>
        )}
      </div>
    </section>
  );
}
export default function Overview({
  status,
  interval,
}: {
  status: Status;
  interval: number;
}) {
  const { canAdmin } = useAccess();
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  useEffect(() => {
    const timer = window.setInterval(() => {
      setNow(Math.floor(Date.now() / 1000));
    }, 1000);
    return () => {
      window.clearInterval(timer);
    };
  }, []);
  const telemetry = useQuery({
    queryKey: ["telemetry"],
    queryFn: api.telemetry,
    refetchInterval: interval,
  });
  const selection = useQuery({
    queryKey: ["selection"],
    queryFn: api.selection,
  });
  const data = telemetry.data;
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Overview</h1>
        </div>
        <ServerControls status={status} />
      </div>
      {status.error && <Notice>{status.error}</Notice>}
      <section className="panel live-status">
        <div className="live-status-main">
          <StatusBadge state={status.state} />
          <strong>{duration(status.since, now)}</strong>
          <span className="muted">since {formatDate(status.since)}</span>
        </div>
        <span className="muted">
          {status.version
            ? `Factorio ${status.version}`
            : status.binary_available
              ? "Factorio ready"
              : "Factorio not configured"}
        </span>
      </section>
      {!selection.isPending &&
        !selection.isError &&
        !selection.data.save &&
        !selection.data.load_latest &&
        status.state === "stopped" && (
          <div className="inline-info">
            Choose a startup save on the <a href="#saves">Saves page</a> to
            enable Start.
          </div>
        )}
      {telemetry.isError && <Notice>{errorMessage(telemetry.error)}</Notice>}
      {data?.error && <Notice>{data.error}</Notice>}
      {data && !data.enabled && (
        <div className="inline-info">
          Live UPS and research require scripting telemetry.{" "}
          {canAdmin ? (
            <>
              Enable it in{" "}
              <a
                href="#settings"
                onClick={() => {
                  window.dispatchEvent(new Event("open-manager-settings"));
                }}
              >
                Manager settings
              </a>{" "}
              after reviewing the achievement implications.
            </>
          ) : (
            "Ask an administrator to enable it after reviewing the achievement implications."
          )}
        </div>
      )}
      <div className="live-grid">
        <section className="panel">
          <div className="section-heading">
            <h2>
              <Users size={20} />
              Online players{" "}
              <span className="count-badge">
                {data?.players?.length ?? "—"}
              </span>
            </h2>
          </div>
          <div className="live-content">
            {data?.players === null || !data ? (
              <p className="muted">Player list unavailable.</p>
            ) : data.players.length === 0 ? (
              <p className="muted">No players online.</p>
            ) : (
              <ul className="online-list">
                {data.players.map((name) => (
                  <li key={name}>
                    <span className="tiny-dot" />
                    {name}
                  </li>
                ))}
              </ul>
            )}
          </div>
        </section>
        <section className="panel">
          <div className="section-heading">
            <h2>
              <FlaskConical size={20} />
              Research
            </h2>
          </div>
          <div className="live-content">
            {data?.research === null || !data ? (
              <p className="muted">Research information unavailable.</p>
            ) : data.research.length === 0 ? (
              <p className="muted">No research in progress.</p>
            ) : (
              data.research.map((item) => (
                <div className="research-item" key={item.force}>
                  <div>
                    <strong>{item.name}</strong>
                    <span>{(item.progress * 100).toFixed(1)}%</span>
                  </div>
                  <progress
                    value={item.progress}
                    max={1}
                    aria-label={`${item.name} research progress`}
                  />
                  <small className="muted">{item.force}</small>
                </div>
              ))
            )}
          </div>
        </section>
      </div>
      {data && <UpsChart telemetry={data} now={now} />}
      <section className="panel activity-panel">
        <div className="section-heading">
          <h2>Recent activity</h2>
          <a className="text-link" href="#console">
            Full console →
          </a>
        </div>
        <div className="activity-list">
          {!data || data.activity.length === 0 ? (
            <p className="muted activity-empty">No recent activity.</p>
          ) : (
            [...data.activity]
              .sort((a, b) => b.timestamp - a.timestamp || b.id - a.id)
              .map((item) => (
                <div className="activity-item" key={item.id}>
                  <time
                    dateTime={new Date(item.timestamp * 1000).toISOString()}
                  >
                    {formatDate(item.timestamp)}
                  </time>
                  <span className={`event-kind ${item.kind}`}>
                    {item.kind.replaceAll("_", " ")}
                  </span>
                  <p>{item.message}</p>
                </div>
              ))
          )}
        </div>
      </section>
    </>
  );
}
