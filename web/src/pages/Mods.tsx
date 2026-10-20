import { Fragment, useRef, useState } from "react";
import {
  useIsFetching,
  useIsMutating,
  useMutation,
  useQueries,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import type { UseQueryResult } from "@tanstack/react-query";
import { ArrowUp, Download, RefreshCw, Upload } from "lucide-react";
import { api, ApiError, errorMessage } from "../api";
import type {
  ModSettingsChange,
  ModSettingsDocument,
  ModsDocument,
  ModUpdates,
  Status,
} from "../api";
import { Loading, Notice } from "../components/ui";
import { useUnsavedChanges } from "../useUnsavedChanges";

function releaseDate(value: string | null | undefined): string {
  if (!value) return "—";
  const timestamp = Date.parse(value);
  return Number.isNaN(timestamp)
    ? "—"
    : new Date(timestamp).toISOString().slice(0, 10);
}

function ModRow({
  mod,
  document,
  stopped,
  updates,
}: {
  mod: ModsDocument["mods"][number];
  document: ModsDocument;
  stopped: boolean;
  updates: UseQueryResult<ModUpdates>;
}) {
  const client = useQueryClient();
  const busy = useIsMutating({ mutationKey: ["mod-change"] }) > 0;
  const enable = useMutation({
    mutationKey: ["mod-change"],
    mutationFn: api.enableMod,
    onSuccess: (result) => {
      client.setQueryData(["mods"], result);
    },
  });
  const update = useMutation({
    mutationKey: ["mod-change"],
    mutationFn: api.updateMod,
    onSuccess: async (result) => {
      client.setQueryData(["mods"], result);
      await client.invalidateQueries({ queryKey: ["mod-settings"] });
    },
  });
  const incompatible = updates.data?.compatible === false;
  const outdated = updates.data?.update_available === true;
  const failure = enable.error ?? update.error;
  return (
    <Fragment>
      <tr
        className={
          incompatible
            ? "mod-incompatible"
            : outdated
              ? "mod-outdated"
              : undefined
        }
      >
        <td>
          <div className="mod-name-cell">
            <input
              type="checkbox"
              aria-label={`Enable ${mod.title || mod.name}`}
              checked={mod.enabled}
              disabled={!stopped || busy}
              onChange={(event) => {
                enable.mutate({
                  name: mod.name,
                  revision: document.revision,
                  enabled: event.target.checked,
                });
              }}
            />
            <span
              className={mod.enabled ? "mod-name" : "mod-name mod-disabled"}
              title={mod.name}
            >
              {mod.title || mod.name}
            </span>
            {incompatible && <span className="mod-state">Incompatible</span>}
            <a
              className="mod-download"
              href={`/api/mods/${encodeURIComponent(mod.name)}/download`}
              download={mod.file_name}
              aria-label={`Download ${mod.title || mod.name}`}
              title="Download installed version"
            >
              <Download size={16} />
            </a>
          </div>
        </td>
        <td>{mod.version}</td>
        <td>
          {updates.data ? (
            outdated && updates.data.latest_version ? (
              <button
                className="mod-upgrade"
                disabled={!stopped || busy || !updates.data.authenticated}
                title={
                  !updates.data.authenticated
                    ? "Add Factorio credentials in Server settings to update"
                    : "Install this version"
                }
                aria-label={`Update ${mod.title || mod.name} to ${updates.data.latest_version}`}
                onClick={() => {
                  const version = updates.data.latest_version;
                  if (version)
                    update.mutate({
                      name: mod.name,
                      version,
                      revision: document.revision,
                    });
                }}
              >
                <ArrowUp size={15} />
                {update.isPending ? "Updating…" : updates.data.latest_version}
              </button>
            ) : (
              (updates.data.latest_version ?? "Unavailable")
            )
          ) : updates.isError ? (
            <button
              className="text-button"
              title={errorMessage(updates.error)}
              onClick={() => {
                updates.refetch().catch(() => undefined);
              }}
            >
              Retry check
            </button>
          ) : (
            <span className="muted">Checking…</span>
          )}
        </td>
        <td>{updates.data?.factorio_min_version ?? mod.factorio_version}</td>
        <td title={updates.data?.latest_released_at ?? undefined}>
          {releaseDate(updates.data?.latest_released_at)}
        </td>
      </tr>
      {failure && (
        <tr>
          <td colSpan={5}>
            <Notice>
              {errorMessage(failure)}{" "}
              <button
                className="text-button"
                onClick={() => {
                  client
                    .invalidateQueries({ queryKey: ["mods"] })
                    .catch(() => undefined);
                  enable.reset();
                  update.reset();
                }}
              >
                Refresh mod list
              </button>
            </Notice>
          </td>
        </tr>
      )}
    </Fragment>
  );
}

function InstalledModsTable({
  document,
  stopped,
  version,
}: {
  document: ModsDocument;
  stopped: boolean;
  version: string | null;
}) {
  const checks = useQueries({
    queries: document.mods.map((mod) => ({
      queryKey: ["mod-updates", mod.name, mod.version, version],
      queryFn: () => api.modUpdates(mod.name),
      staleTime: 5 * 60 * 1000,
      retry: false,
      refetchOnWindowFocus: false,
    })),
  });
  const outdated = checks.filter(
    (check) => check.data?.update_available,
  ).length;
  const incompatible = checks.filter(
    (check) => check.data?.compatible === false,
  ).length;
  const pending = checks.filter((check) => check.isFetching).length;
  const failed = checks.filter((check) => check.isError).length;
  return (
    <>
      <div className="mod-summary" aria-live="polite">
        <span>
          Latest compatible releases{version ? ` · Factorio ${version}` : ""}
        </span>
        <span className="mod-legend outdated">{outdated} upgrades</span>
        <span className="mod-legend incompatible">
          {incompatible} incompatible
        </span>
        {pending > 0 && <span>{pending} checking…</span>}
        {failed > 0 && <span>{failed} checks failed · retry below</span>}
      </div>
      <div className="table-scroll mods-table-scroll">
        <table className="mods-table" aria-label="Installed mod versions">
          <thead>
            <tr>
              <th scope="col">Name</th>
              <th scope="col">Current</th>
              <th scope="col">Latest</th>
              <th
                scope="col"
                title="Minimum Factorio version required by the installed mod"
              >
                Factorio min.
              </th>
              <th scope="col">Released</th>
            </tr>
          </thead>
          <tbody>
            {document.mods.map((mod, index) => {
              const check = checks[index];
              return check ? (
                <ModRow
                  key={`${mod.name}:${mod.version}`}
                  mod={mod}
                  document={document}
                  stopped={stopped}
                  updates={check}
                />
              ) : null;
            })}
          </tbody>
        </table>
      </div>
    </>
  );
}

function StartupEditor({
  initial,
  stopped,
}: {
  initial: ModSettingsDocument;
  stopped: boolean;
}) {
  const [baseline, setBaseline] = useState(initial);
  const [draft, setDraft] = useState(initial.settings);
  const [seenRevision, setSeenRevision] = useState(initial.revision);
  const fileInput = useRef<HTMLInputElement>(null);
  const client = useQueryClient();
  const dirty = JSON.stringify(draft) !== JSON.stringify(baseline.settings);
  useUnsavedChanges(dirty);
  function receive(result: ModSettingsDocument) {
    setBaseline(result);
    setDraft(result.settings);
    client.setQueryData(["mod-settings"], result);
  }
  const save = useMutation({
    mutationFn: api.saveModSettings,
    onSuccess: receive,
  });
  const upload = useMutation({
    mutationFn: api.uploadModSettings,
    onSuccess: receive,
  });
  const reload = useMutation({
    mutationFn: api.modSettings,
    onSuccess: (result) => {
      receive(result);
      save.reset();
      upload.reset();
    },
  });
  const busy = save.isPending || upload.isPending || reload.isPending;
  if (initial.revision !== seenRevision) {
    setSeenRevision(initial.revision);
    if (!dirty && !busy) {
      setBaseline(initial);
      setDraft(initial.settings);
    }
  }
  function change(name: string, value: string) {
    setDraft((previous) =>
      previous.map((setting) =>
        setting.name === name ? { ...setting, value } : setting,
      ),
    );
    save.reset();
  }
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        const changes: Record<string, ModSettingsChange> = {};
        for (const setting of draft) {
          const old = baseline.settings.find(
            (item) => item.name === setting.name,
          );
          if (JSON.stringify(old) !== JSON.stringify(setting)) {
            changes[setting.name] =
              setting.kind === "color"
                ? {
                    components: Object.fromEntries(
                      setting.components.map((component) => [
                        component.name,
                        component.value,
                      ]),
                    ),
                  }
                : { value: setting.value };
          }
        }
        save.mutate({ revision: baseline.revision, changes });
      }}
    >
      <div className="toolbar">
        <p className="muted">
          Startup changes take effect when Factorio next starts.
        </p>
        <div className="toolbar-actions">
          <button
            type="button"
            className="button secondary"
            disabled={busy || dirty}
            onClick={() => {
              reload.mutate();
            }}
          >
            Reload settings
          </button>
          <input
            ref={fileInput}
            className="visually-hidden"
            type="file"
            accept=".dat"
            aria-label="Import mod-settings.dat"
            disabled={!stopped || busy || dirty}
            onChange={(event) => {
              const file = event.target.files?.[0];
              if (file) upload.mutate({ file, revision: baseline.revision });
              event.target.value = "";
            }}
          />
          <button
            type="button"
            className="button secondary"
            disabled={!stopped || busy || dirty}
            onClick={() => {
              fileInput.current?.click();
            }}
          >
            <Upload size={16} />
            Import settings
          </button>
          {baseline.available && (
            <a
              className="button secondary"
              href="/api/mods/settings/file"
              download="mod-settings.dat"
            >
              <Download size={16} />
              Download settings
            </a>
          )}
        </div>
      </div>
      <section className="panel">
        <div className="section-heading">
          <div>
            <h2>Startup settings</h2>
            <p>Values from mod-settings.dat. Runtime settings are preserved.</p>
          </div>
        </div>
        <div className="form-fields">
          {!baseline.available ? (
            <p className="muted">
              No mod-settings.dat exists yet. Import this file from a Factorio
              installation configured with your mods to edit its startup values
              here.
            </p>
          ) : draft.length === 0 ? (
            <p className="muted">This file contains no startup settings.</p>
          ) : (
            draft.map((setting) => (
              <div key={setting.name}>
                {setting.kind === "boolean" ? (
                  <label className="toggle-row">
                    <strong>{setting.name}</strong>
                    <input
                      type="checkbox"
                      checked={setting.value === "true"}
                      disabled={!stopped || busy}
                      onChange={(event) => {
                        change(setting.name, String(event.target.checked));
                      }}
                    />
                  </label>
                ) : setting.kind === "color" ? (
                  <fieldset
                    className="color-setting"
                    disabled={!stopped || busy}
                  >
                    <legend>{setting.name}</legend>
                    {setting.components.map((component) => (
                      <label className="field" key={component.name}>
                        {component.name}
                        <input
                          type="number"
                          step="any"
                          min={0}
                          max={1}
                          required
                          value={component.value}
                          onChange={(event) => {
                            setDraft((previous) =>
                              previous.map((item) =>
                                item.name === setting.name
                                  ? {
                                      ...item,
                                      components: item.components.map((part) =>
                                        part.name === component.name
                                          ? {
                                              ...part,
                                              value: event.target.value,
                                            }
                                          : part,
                                      ),
                                    }
                                  : item,
                              ),
                            );
                            save.reset();
                          }}
                        />
                      </label>
                    ))}
                  </fieldset>
                ) : (
                  <label className="field">
                    {setting.name}
                    <input
                      type={
                        setting.kind === "string" ||
                        setting.kind === "unsupported"
                          ? "text"
                          : "number"
                      }
                      step={setting.kind === "number" ? "any" : 1}
                      min={setting.kind === "unsigned_integer" ? 0 : undefined}
                      required={setting.kind !== "string"}
                      value={setting.value}
                      disabled={
                        !stopped || busy || setting.kind === "unsupported"
                      }
                      onChange={(event) => {
                        change(setting.name, event.target.value);
                      }}
                    />
                    {setting.kind === "unsupported" && (
                      <small>
                        This value is preserved but cannot be edited in the
                        form.
                      </small>
                    )}
                  </label>
                )}
              </div>
            ))
          )}
        </div>
      </section>
      {save.isError && (
        <Notice>
          {errorMessage(save.error)}
          {save.error instanceof ApiError && save.error.status === 409 && (
            <button
              className="text-button"
              type="button"
              disabled={busy}
              onClick={() => {
                reload.mutate();
              }}
            >
              Discard edits & reload
            </button>
          )}
        </Notice>
      )}
      {upload.isError && <Notice>{errorMessage(upload.error)}</Notice>}
      {reload.isError && <Notice>{errorMessage(reload.error)}</Notice>}
      {save.isSuccess && !dirty && (
        <Notice success>Startup settings saved.</Notice>
      )}
      {upload.isSuccess && !dirty && (
        <Notice success>Mod settings imported.</Notice>
      )}
      <div className="save-bar">
        <span>
          {dirty
            ? "You have unsaved changes"
            : "Startup settings are up to date"}
        </span>
        <div>
          <button
            type="button"
            className="button secondary"
            disabled={!dirty || busy}
            onClick={() => {
              setDraft(baseline.settings);
              save.reset();
            }}
          >
            Discard
          </button>
          <button
            className="button primary"
            disabled={!dirty || busy || !stopped}
          >
            {save.isPending ? "Saving…" : "Save startup settings"}
          </button>
        </div>
      </div>
    </form>
  );
}
function StartupSettings({ stopped }: { stopped: boolean }) {
  const query = useQuery({
    queryKey: ["mod-settings"],
    queryFn: api.modSettings,
  });
  if (query.data === undefined)
    return query.isPending ? (
      <Loading />
    ) : (
      <Notice>{errorMessage(query.error)}</Notice>
    );
  return (
    <>
      {query.isError && (
        <Notice>{errorMessage(query.error)} Your draft is kept below.</Notice>
      )}
      <StartupEditor initial={query.data} stopped={stopped} />
    </>
  );
}
export default function Mods({
  status,
  active = true,
}: {
  status: Status;
  active?: boolean;
}) {
  const [tab, setTab] = useState<"mods" | "startup">("mods");
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const client = useQueryClient();
  const query = useQuery({ queryKey: ["mods"], queryFn: api.mods });
  const checking = useIsFetching({ queryKey: ["mod-updates"] }) > 0;
  const changing = useIsMutating({ mutationKey: ["mod-change"] }) > 0;
  const upload = useMutation({
    mutationKey: ["mod-change"],
    mutationFn: api.uploadMod,
    onSuccess: (result) => {
      client.setQueryData(["mods"], result);
      client
        .invalidateQueries({ queryKey: ["mod-settings"] })
        .catch(() => undefined);
    },
  });
  const stopped = status.state === "stopped" || status.state === "failed";
  return (
    <>
      <div className="tab-buttons settings-tabs">
        <button
          className={tab === "mods" ? "button primary" : "button secondary"}
          onClick={() => {
            setTab("mods");
          }}
        >
          Installed mods
        </button>
        <button
          className={tab === "startup" ? "button primary" : "button secondary"}
          onClick={() => {
            setTab("startup");
          }}
        >
          Startup settings
        </button>
      </div>
      {!stopped && (
        <div className="inline-info">
          Stop the server before changing mods or startup settings. Downloads
          remain available.
        </div>
      )}
      <div hidden={tab !== "mods"}>
        <div className="toolbar">
          <button
            className="button secondary"
            disabled={checking || !query.data?.mods.length}
            onClick={() => {
              client
                .invalidateQueries({
                  queryKey: ["mod-updates"],
                  refetchType: "active",
                })
                .catch(() => undefined);
            }}
          >
            <RefreshCw size={16} />
            {checking ? "Checking updates…" : "Check all updates"}
          </button>
          <input
            ref={input}
            type="file"
            className="visually-hidden"
            accept=".zip,application/zip"
            aria-label="Upload mod ZIP"
            disabled={!stopped || changing}
            onChange={(event) => {
              const file = event.target.files?.[0];
              setError(null);
              if (file && query.data) {
                if (file.size > 64 * 1024 * 1024)
                  setError("Mod ZIP uploads are limited to 64 MB.");
                else upload.mutate({ file, revision: query.data.revision });
              }
              event.target.value = "";
            }}
          />
          <button
            className="button primary"
            disabled={!stopped || changing || !query.data}
            onClick={() => {
              input.current?.click();
            }}
          >
            <Upload size={16} />
            {upload.isPending ? "Uploading…" : "Upload mod"}
          </button>
        </div>
        {error && <Notice>{error}</Notice>}
        {upload.isError && (
          <Notice>
            {errorMessage(upload.error)}{" "}
            <button
              className="text-button"
              onClick={() => {
                client
                  .invalidateQueries({ queryKey: ["mods"] })
                  .catch(() => undefined);
                upload.reset();
              }}
            >
              Refresh mod list
            </button>
          </Notice>
        )}
        {upload.isSuccess && (
          <Notice success>
            Mod installed. Changes apply on the next server start.
          </Notice>
        )}
        {query.isPending ? (
          <Loading />
        ) : query.isError ? (
          <Notice>{errorMessage(query.error)}</Notice>
        ) : (
          <>
            {!query.data.portal_authenticated && (
              <div className="inline-info">
                Installing updates requires a Factorio username and token in{" "}
                <a href="#settings">Server settings</a>. Version checks work
                without an account.
              </div>
            )}
            <section className="panel">
              <div className="section-heading">
                <h2>
                  Installed mods{" "}
                  <span className="count-badge">{query.data.mods.length}</span>
                </h2>
              </div>
              {query.data.mods.length === 0 ? (
                <p className="live-content muted">
                  No uploaded mods. Factorio's bundled content is managed by the
                  game.
                </p>
              ) : (
                active &&
                tab === "mods" && (
                  <InstalledModsTable
                    document={query.data}
                    stopped={stopped}
                    version={status.version}
                  />
                )
              )}
            </section>
          </>
        )}
      </div>
      <div hidden={tab !== "startup"}>
        <StartupSettings stopped={stopped} />
      </div>
    </>
  );
}
