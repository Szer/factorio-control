import { useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Download, RefreshCw, Upload } from "lucide-react";
import { api, ApiError, errorMessage, formatBytes } from "../api";
import type {
  ModSettingsChange,
  ModSettingsDocument,
  ModsDocument,
  Status,
} from "../api";
import { Loading, Notice } from "../components/ui";
import { useUnsavedChanges } from "../useUnsavedChanges";

function ModRow({
  mod,
  document,
  stopped,
}: {
  mod: ModsDocument["mods"][number];
  document: ModsDocument;
  stopped: boolean;
}) {
  const client = useQueryClient();
  const updates = useQuery({
    queryKey: ["mod-updates", mod.name, mod.version],
    queryFn: () => api.modUpdates(mod.name),
    enabled: false,
    retry: false,
  });
  const enable = useMutation({
    mutationFn: api.enableMod,
    onSuccess: (result) => {
      client.setQueryData(["mods"], result);
    },
  });
  const update = useMutation({
    mutationFn: api.updateMod,
    onSuccess: async (result) => {
      client.setQueryData(["mods"], result);
      await client.invalidateQueries({ queryKey: ["mod-settings"] });
    },
  });
  const busy = enable.isPending || update.isPending;
  return (
    <div className="mod-row">
      <div className="mod-identity">
        <strong>{mod.title || mod.name}</strong>
        <span>
          {mod.name} · {mod.version} · {formatBytes(mod.size_bytes)}
        </span>
        <p>{mod.description}</p>
        <small>
          Factorio {mod.factorio_version} · {mod.author}
        </small>
        {mod.dependencies.length > 0 && (
          <details>
            <summary>Dependencies</summary>
            <ul>
              {mod.dependencies.map((dependency) => (
                <li key={dependency}>{dependency}</li>
              ))}
            </ul>
          </details>
        )}
      </div>
      <div className="mod-actions">
        <label className="check-label">
          <input
            type="checkbox"
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
          Enabled
        </label>
        <a
          className="button secondary"
          href={`/api/mods/${encodeURIComponent(mod.name)}/download`}
          download={mod.file_name}
        >
          <Download size={16} />
          Download
        </a>
        <button
          className="button secondary"
          disabled={busy || updates.isFetching}
          onClick={() => {
            updates.refetch().catch(() => undefined);
          }}
        >
          <RefreshCw size={16} />
          {updates.isFetching ? "Checking…" : "Check update"}
        </button>
        {updates.data?.latest_version &&
        updates.data.latest_version !== mod.version ? (
          <button
            className="button primary"
            disabled={!stopped || busy || !updates.data.authenticated}
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
            {update.isPending
              ? "Updating…"
              : `Update to ${updates.data.latest_version}`}
          </button>
        ) : (
          updates.isSuccess && (
            <span className="muted">No compatible update found</span>
          )
        )}
      </div>
      {updates.data?.message && (
        <div className="inline-info">{updates.data.message}</div>
      )}
      {updates.isError && <Notice>{errorMessage(updates.error)}</Notice>}
      {enable.isError && (
        <Notice>
          {errorMessage(enable.error)}{" "}
          <button
            className="text-button"
            onClick={() => {
              client
                .invalidateQueries({ queryKey: ["mods"] })
                .catch(() => undefined);
              enable.reset();
            }}
          >
            Refresh mod list
          </button>
        </Notice>
      )}
      {update.isError && (
        <Notice>
          {errorMessage(update.error)}{" "}
          <button
            className="text-button"
            onClick={() => {
              client
                .invalidateQueries({ queryKey: ["mods"] })
                .catch(() => undefined);
              update.reset();
            }}
          >
            Refresh mod list
          </button>
        </Notice>
      )}
    </div>
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
export default function Mods({ status }: { status: Status }) {
  const [tab, setTab] = useState<"mods" | "startup">("mods");
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const client = useQueryClient();
  const query = useQuery({ queryKey: ["mods"], queryFn: api.mods });
  const upload = useMutation({
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
          <p className="muted">
            Upload a mod ZIP to install it or replace it with a newer version.
          </p>
          <input
            ref={input}
            type="file"
            className="visually-hidden"
            accept=".zip,application/zip"
            aria-label="Upload mod ZIP"
            disabled={!stopped || upload.isPending}
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
            disabled={!stopped || upload.isPending || !query.data}
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
                Mod portal updates need your Factorio username and token in{" "}
                <a href="#settings">Server settings</a>, or FACTORIO_USERNAME
                and FACTORIO_TOKEN in the manager environment. ZIP upload and
                download work without an account.
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
                query.data.mods.map((mod) => (
                  <ModRow
                    key={`${mod.name}:${mod.version}`}
                    mod={mod}
                    document={query.data}
                    stopped={stopped}
                  />
                ))
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
