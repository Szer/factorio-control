import { useSessionMutation } from "../useSessionMutation";
import { operations } from "../operationsApi";
import { useEffect, useState } from "react";
import type { SyntheticEvent } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  api,
  ApiError,
  errorMessage,
  managerSchema,
  rawSettingsSchema,
} from "../api";
import type { Settings, RawSettingsDocument, ManagerDocument } from "../api";
import { Loading, Notice } from "../components/ui";
import { AccountPanel } from "../components/Auth";
import Administrators from "../components/Administrators";
import { EditorMode, JsonEditor } from "../components/JsonEditor";
import { useUnsavedChanges } from "../useUnsavedChanges";

type Field = {
  key: keyof Settings;
  label: string;
  kind:
    | "text"
    | "password"
    | "number"
    | "boolean"
    | "textarea"
    | "tags"
    | "commands";
  help?: string;
  max?: number;
};
const groups: { title: string; fields: Field[] }[] = [
  {
    title: "Server identity",
    fields: [
      { key: "name", label: "Server name", kind: "text", max: 100 },
      { key: "description", label: "Description", kind: "textarea", max: 2000 },
      { key: "tags", label: "Tags", kind: "tags", help: "One tag per line." },
    ],
  },
  {
    title: "Access and players",
    fields: [
      {
        key: "max_players",
        label: "Player limit",
        kind: "number",
        max: 65535,
        help: "0 allows unlimited players.",
      },
      {
        key: "game_password",
        label: "Game password",
        kind: "password",
        help: "Leave empty to allow joining without a password.",
      },
      {
        key: "require_user_verification",
        label: "Verify Factorio accounts",
        kind: "boolean",
      },
      {
        key: "ignore_player_limit_for_returning_players",
        label: "Allow returning players past the player limit",
        kind: "boolean",
      },
      {
        key: "allow_commands",
        label: "Lua console commands",
        kind: "commands",
      },
      {
        key: "afk_autokick_interval",
        label: "AFK kick interval (minutes)",
        kind: "number",
        help: "0 disables automatic kicking.",
      },
    ],
  },
  {
    title: "Factorio account",
    fields: [
      {
        key: "username",
        label: "Factorio username",
        kind: "text",
        help: "Your Factorio.com account username. Mod downloads require both username and token.",
      },
      {
        key: "token",
        label: "Factorio token",
        kind: "password",
        help: "Your Factorio.com account token, used with the username for public game registration and mod downloads. This is not a control-panel login token.",
      },
      {
        key: "password",
        label: "Factorio password",
        kind: "password",
        help: "Factorio supports a token instead of an account password.",
      },
    ],
  },
  {
    title: "Saving and pausing",
    fields: [
      {
        key: "autosave_interval",
        label: "Autosave interval (minutes)",
        kind: "number",
        max: 1440,
        help: "0 disables autosaves.",
      },
      { key: "autosave_slots", label: "Autosave slots", kind: "number" },
      {
        key: "autosave_only_on_server",
        label: "Autosave only on the server",
        kind: "boolean",
      },
      {
        key: "non_blocking_saving",
        label: "Non-blocking saves",
        kind: "boolean",
        help: "Factorio's background saving support on Linux.",
      },
      {
        key: "auto_pause",
        label: "Pause when the server is empty",
        kind: "boolean",
      },
      {
        key: "auto_pause_when_players_connect",
        label: "Pause while players connect",
        kind: "boolean",
      },
      {
        key: "only_admins_can_pause_the_game",
        label: "Only administrators can pause",
        kind: "boolean",
      },
    ],
  },
  {
    title: "Network",
    fields: [
      {
        key: "max_upload_in_kilobytes_per_second",
        label: "Upload limit (KB/s)",
        kind: "number",
        help: "0 means unlimited.",
      },
      {
        key: "max_upload_slots",
        label: "Concurrent upload slots",
        kind: "number",
        help: "0 means unlimited.",
      },
      {
        key: "minimum_latency_in_ticks",
        label: "Minimum latency (ticks)",
        kind: "number",
      },
      {
        key: "max_heartbeats_per_second",
        label: "Maximum heartbeats per second",
        kind: "number",
      },
      {
        key: "minimum_segment_size",
        label: "Minimum segment size",
        kind: "number",
      },
      {
        key: "minimum_segment_size_peer_count",
        label: "Minimum segment size peer count",
        kind: "number",
      },
      {
        key: "maximum_segment_size",
        label: "Maximum segment size",
        kind: "number",
      },
      {
        key: "maximum_segment_size_peer_count",
        label: "Maximum segment size peer count",
        kind: "number",
      },
    ],
  },
];
function parseRaw(text: string): Record<string, unknown> {
  const parsed: unknown = JSON.parse(text);
  return rawSettingsSchema.shape.settings.parse(parsed);
}
function SaveBar({
  dirty,
  busy,
  discard,
  label = "Save changes",
}: {
  dirty: boolean;
  busy: boolean;
  discard: () => void;
  label?: string;
}) {
  return (
    <div className="save-bar">
      <span>{dirty ? "You have unsaved changes" : "All changes saved"}</span>
      <div>
        <button
          type="button"
          className="button secondary"
          disabled={!dirty || busy}
          onClick={discard}
        >
          Discard
        </button>
        <button className="button primary" disabled={!dirty || busy}>
          {busy ? "Saving…" : label}
        </button>
      </div>
    </div>
  );
}
function ServerForm({ initial }: { initial: RawSettingsDocument }) {
  const [baseline, setBaseline] = useState(initial);
  const [text, setText] = useState(JSON.stringify(initial.settings, null, 2));
  const [mode, setMode] = useState<"form" | "raw">("form");
  const [validation, setValidation] = useState<string | null>(null);
  const client = useQueryClient();
  const dirty = text !== JSON.stringify(baseline.settings, null, 2);
  useUnsavedChanges(dirty);
  async function receive(result: RawSettingsDocument) {
    setBaseline(result);
    setText(JSON.stringify(result.settings, null, 2));
    await client.cancelQueries({ queryKey: ["raw-settings"] });
    client.setQueryData(["raw-settings"], result);
    client.invalidateQueries({ queryKey: ["settings"] }).catch(() => undefined);
  }
  const save = useSessionMutation({
    mutationFn: api.saveRawSettings,
    onSuccess: receive,
  });
  const reload = useSessionMutation({
    mutationFn: api.rawSettings,
    onSuccess: async (result) => {
      await receive(result);
      save.reset();
      setValidation(null);
    },
  });
  const busy = save.isPending || reload.isPending;
  let draft: Record<string, unknown> = {};
  let parseError = false;
  try {
    draft = parseRaw(text);
  } catch {
    parseError = true;
  }
  function setField(key: string, value: unknown) {
    setText(JSON.stringify({ ...draft, [key]: value }, null, 2));
    save.reset();
    setValidation(null);
  }
  function submit(event: SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    setValidation(null);
    try {
      save.mutate({ revision: baseline.revision, settings: parseRaw(text) });
    } catch {
      setValidation("Enter a valid JSON object. Your draft has been kept.");
    }
  }
  const visibility = rawSettingsSchema.shape.settings.safeParse(
    draft.visibility,
  );
  return (
    <form onSubmit={submit}>
      <div className="toolbar">
        <EditorMode
          mode={mode}
          onChange={setMode}
          busy={busy}
          invalid={parseError}
        />
        <span className="muted">Applied on the next server start</span>
      </div>
      {mode === "raw" ? (
        <JsonEditor
          filename="server-settings.json"
          label="Raw server settings"
          help="All fields are editable. Unknown keys are preserved when switching back to the form. This file can contain credentials."
          rows={24}
          value={text}
          disabled={busy}
          onChange={(value) => {
            setText(value);
            save.reset();
            setValidation(null);
          }}
        />
      ) : (
        <div className="settings-sections">
          {parseError && (
            <Notice>
              Invalid JSON. Open Raw JSON to correct the document.
            </Notice>
          )}
          {groups.map((group) => (
            <section className="panel form-panel" key={group.title}>
              <div className="section-heading">
                <h2>{group.title}</h2>
              </div>
              <div className="form-fields settings-fields">
                {group.fields.map((field) => {
                  const value = draft[field.key];
                  if (field.kind === "boolean")
                    return (
                      <label key={field.key} className="toggle-row">
                        <span>
                          <strong>{field.label}</strong>
                          {field.help && <small>{field.help}</small>}
                        </span>
                        <input
                          type="checkbox"
                          checked={value === true}
                          disabled={busy || parseError}
                          onChange={(event) => {
                            setField(field.key, event.target.checked);
                          }}
                        />
                      </label>
                    );
                  return (
                    <label
                      className={`field field-${field.kind} ${field.kind === "textarea" || field.kind === "tags" ? "wide-field" : ""}`}
                      key={field.key}
                    >
                      {field.label}
                      {field.kind === "commands" ? (
                        <select
                          value={String(value)}
                          disabled={busy || parseError}
                          onChange={(event) => {
                            setField(
                              field.key,
                              event.target.value === "admins-only"
                                ? "admins-only"
                                : event.target.value === "true",
                            );
                          }}
                        >
                          <option value="admins-only">
                            Administrators only
                          </option>
                          <option value="true">Everyone</option>
                          <option value="false">Disabled</option>
                        </select>
                      ) : field.kind === "textarea" || field.kind === "tags" ? (
                        <textarea
                          rows={2}
                          maxLength={field.max}
                          value={
                            field.kind === "tags"
                              ? Array.isArray(value)
                                ? value
                                    .filter(
                                      (item): item is string =>
                                        typeof item === "string",
                                    )
                                    .join("\n")
                                : ""
                              : typeof value === "string"
                                ? value
                                : ""
                          }
                          disabled={busy || parseError}
                          onChange={(event) => {
                            setField(
                              field.key,
                              field.kind === "tags"
                                ? event.target.value.split("\n")
                                : event.target.value,
                            );
                          }}
                        />
                      ) : (
                        <input
                          type={field.kind}
                          value={
                            typeof value === "string" ||
                            typeof value === "number"
                              ? value
                              : ""
                          }
                          disabled={busy || parseError}
                          autoComplete={
                            field.kind === "password" ? "new-password" : "off"
                          }
                          min={field.kind === "number" ? 0 : undefined}
                          max={field.kind === "number" ? field.max : undefined}
                          step={field.kind === "number" ? 1 : undefined}
                          maxLength={
                            field.kind === "text" ? field.max : undefined
                          }
                          required={
                            field.kind === "number" || field.key === "name"
                          }
                          onChange={(event) => {
                            setField(
                              field.key,
                              field.kind === "number"
                                ? event.target.value === ""
                                  ? ""
                                  : event.target.valueAsNumber
                                : event.target.value,
                            );
                          }}
                        />
                      )}
                      {field.help && <small>{field.help}</small>}
                    </label>
                  );
                })}
              </div>
            </section>
          ))}
          <section className="panel form-panel">
            <div className="section-heading">
              <h2>Server visibility</h2>
            </div>
            <div className="form-fields">
              {(
                [
                  ["lan", "Visible on your local network"],
                  ["public", "Listed in the public server browser"],
                ] as const
              ).map(([key, label]) => (
                <label className="toggle-row" key={key}>
                  <strong>{label}</strong>
                  <input
                    type="checkbox"
                    checked={
                      visibility.success && visibility.data[key] === true
                    }
                    disabled={busy || parseError}
                    onChange={(event) => {
                      setField("visibility", {
                        ...(visibility.success ? visibility.data : {}),
                        [key]: event.target.checked,
                      });
                    }}
                  />
                </label>
              ))}
            </div>
          </section>
        </div>
      )}
      {validation && <Notice>{validation}</Notice>}
      {save.isError && (
        <Notice>
          {errorMessage(save.error)}
          {save.error instanceof ApiError && save.error.status === 409 && (
            <button
              type="button"
              className="text-button"
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
      {reload.isError && <Notice>{errorMessage(reload.error)}</Notice>}
      {save.isSuccess && !dirty && (
        <Notice success>
          Settings saved. Restart the server to apply them.
        </Notice>
      )}
      <SaveBar
        dirty={dirty}
        busy={busy}
        discard={() => {
          setText(JSON.stringify(baseline.settings, null, 2));
          save.reset();
          setValidation(null);
        }}
      />
    </form>
  );
}
function ServerSettings() {
  const query = useQuery({
    queryKey: ["raw-settings"],
    queryFn: api.rawSettings,
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
      <ServerForm initial={query.data} />
    </>
  );
}
function ManagerForm({ initial }: { initial: ManagerDocument }) {
  const [baseline, setBaseline] = useState(initial);
  const [text, setText] = useState(JSON.stringify(initial.settings, null, 2));
  const [mode, setMode] = useState<"form" | "raw">("form");
  const [draft, setFormDraft] = useState(initial.settings);
  let invalid = false;
  try {
    managerSchema.shape.settings.strict().parse(JSON.parse(text) as unknown);
  } catch {
    invalid = true;
  }
  function setDraft(value: ManagerDocument["settings"]) {
    setFormDraft(value);
    setText(JSON.stringify(value, null, 2));
    setValidation(null);
  }
  const [validation, setValidation] = useState<string | null>(null);
  const client = useQueryClient();
  const dirty = text !== JSON.stringify(baseline.settings, null, 2);
  useUnsavedChanges(dirty);
  async function receive(result: ManagerDocument) {
    setBaseline(result);
    setDraft(result.settings);
    await client.cancelQueries({ queryKey: ["manager"] });
    client.setQueryData(["manager"], result);
  }
  const save = useSessionMutation({
    mutationFn: api.saveManager,
    onSuccess: receive,
  });
  const reload = useSessionMutation({
    mutationFn: api.manager,
    onSuccess: async (result) => {
      await receive(result);
      save.reset();
      setValidation(null);
    },
  });
  const busy = save.isPending || reload.isPending;
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        try {
          const settings = managerSchema.shape.settings
            .strict()
            .parse(JSON.parse(text) as unknown);
          setValidation(null);
          save.mutate({ revision: baseline.revision, settings });
        } catch {
          setValidation(
            "Enter a JSON object with a refresh interval of 1–60, history of 1–1440, and a boolean telemetry setting. Unknown keys are not supported. Your draft has been kept.",
          );
        }
      }}
    >
      <div className="toolbar">
        <EditorMode
          mode={mode}
          onChange={setMode}
          busy={busy}
          invalid={invalid}
        />
        <span className="muted">manager-settings.json</span>
      </div>
      {mode === "raw" ? (
        <JsonEditor
          filename="manager-settings.json"
          label="Raw manager settings"
          help="Controls the manager only. Enabling scripting telemetry disables achievements for the loaded save."
          value={text}
          disabled={busy}
          onChange={(value) => {
            setText(value);
            save.reset();
            setValidation(null);
            try {
              setFormDraft(
                managerSchema.shape.settings
                  .strict()
                  .parse(JSON.parse(value) as unknown),
              );
            } catch {
              /* Invalid JSON stays editable in the raw editor. */
            }
          }}
        />
      ) : (
        <section className="panel form-panel">
          <div className="section-heading">
            <div>
              <h2>Manager settings</h2>
            </div>
          </div>
          <div className="form-fields">
            <label className="field compact-field">
              Refresh interval (seconds)
              <input
                type="number"
                min={1}
                max={60}
                required
                disabled={busy}
                value={
                  Number.isNaN(draft.poll_interval_seconds)
                    ? ""
                    : draft.poll_interval_seconds
                }
                onChange={(event) => {
                  setDraft({
                    ...draft,
                    poll_interval_seconds: event.target.valueAsNumber,
                  });
                  save.reset();
                }}
              />
              <small>UI refresh and server sampling interval.</small>
            </label>
            <label className="field compact-field">
              UPS history (minutes)
              <input
                type="number"
                min={1}
                max={1440}
                required
                disabled={busy}
                value={
                  Number.isNaN(draft.ups_history_minutes)
                    ? ""
                    : draft.ups_history_minutes
                }
                onChange={(event) => {
                  setDraft({
                    ...draft,
                    ups_history_minutes: event.target.valueAsNumber,
                  });
                  save.reset();
                }}
              />
              <small>History is kept in memory while the manager runs.</small>
            </label>
            <label className="field compact-field">
              Game bind address
              <input
                required
                value={draft.game_bind_address}
                disabled={busy}
                onChange={(event) => {
                  setDraft({ ...draft, game_bind_address: event.target.value });
                  save.reset();
                }}
              />
              <small>Applied on the next server start.</small>
            </label>
            <label className="field compact-field">
              Game UDP port
              <input
                type="number"
                min={1}
                max={65535}
                required
                value={Number.isNaN(draft.game_port) ? "" : draft.game_port}
                disabled={busy}
                onChange={(event) => {
                  setDraft({ ...draft, game_port: event.target.valueAsNumber });
                  save.reset();
                }}
              />
            </label>
            <label className="toggle-row">
              <span>
                <strong>Enable server whitelist</strong>
                <small>
                  Only players in server-whitelist.json may join. Applied on the
                  next server start.
                </small>
              </span>
              <input
                type="checkbox"
                checked={draft.use_server_whitelist}
                disabled={busy}
                onChange={(event) => {
                  setDraft({
                    ...draft,
                    use_server_whitelist: event.target.checked,
                  });
                  save.reset();
                }}
              />
            </label>
            <label className="toggle-row">
              <span>
                <strong>Enable scripting telemetry</strong>
                <small>
                  Read live UPS, pause state, players, research, and completed
                  research through Factorio's scripting console.
                </small>
              </span>
              <input
                type="checkbox"
                checked={draft.enable_script_telemetry}
                disabled={busy}
                onChange={(event) => {
                  setDraft({
                    ...draft,
                    enable_script_telemetry: event.target.checked,
                  });
                  save.reset();
                }}
              />
            </label>
            <div className="notice">
              <span>
                Enabling scripting telemetry disables achievements for the
                loaded save. Turning it off later does not restore achievements.
                Enable it only for saves where you accept this.
              </span>
            </div>
          </div>
        </section>
      )}
      {validation && <Notice>{validation}</Notice>}
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
      {reload.isError && <Notice>{errorMessage(reload.error)}</Notice>}
      {save.isSuccess && !dirty && (
        <Notice success>Manager settings saved.</Notice>
      )}
      <SaveBar
        dirty={dirty}
        busy={busy}
        discard={() => {
          setDraft(baseline.settings);
          save.reset();
          setValidation(null);
        }}
      />
    </form>
  );
}
function RuntimePathsDetails() {
  const [open, setOpen] = useState(false);
  const paths = useQuery({
    queryKey: ["runtime-paths"],
    queryFn: operations.runtimePaths,
    enabled: open,
  });
  return (
    <details
      className="runtime-paths"
      onToggle={(event) => {
        setOpen(event.currentTarget.open);
      }}
    >
      <summary>Runtime paths</summary>
      {open &&
        (paths.isPending ? (
          <Loading />
        ) : paths.isError ? (
          <Notice>{errorMessage(paths.error)}</Notice>
        ) : (
          <dl>
            {[
              ["Manager data", paths.data.manager_data],
              ["Game executable", paths.data.game_binary ?? "Not configured"],
              ["Game configuration", paths.data.config_file],
              ["Read data", paths.data.read_data],
              ["Write data", paths.data.write_data],
            ].map(([label, value]) => (
              <div key={label}>
                <dt>{label}</dt>
                <dd>{value}</dd>
              </div>
            ))}
          </dl>
        ))}
    </details>
  );
}
function ManagerSettings() {
  const query = useQuery({ queryKey: ["manager"], queryFn: api.manager });
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
      <ManagerForm initial={query.data} />
      <RuntimePathsDetails />
    </>
  );
}
export default function SettingsPage() {
  const [tab, setTab] = useState<"server" | "manager" | "admins" | "accounts">(
    "server",
  );
  useEffect(() => {
    const show = () => {
      setTab("manager");
    };
    window.addEventListener("open-manager-settings", show);
    return () => {
      window.removeEventListener("open-manager-settings", show);
    };
  }, []);
  return (
    <>
      <div className="tab-buttons settings-tabs" aria-label="Settings section">
        {(
          [
            ["server", "Server settings"],
            ["admins", "Administrators"],
            ["manager", "Manager settings"],
            ["accounts", "Panel accounts"],
          ] as const
        ).map(([id, label]) => (
          <button
            key={id}
            className={tab === id ? "button primary" : "button secondary"}
            aria-pressed={tab === id}
            onClick={() => {
              setTab(id);
            }}
          >
            {label}
          </button>
        ))}
      </div>
      <div hidden={tab !== "server"}>
        <ServerSettings />
      </div>
      <div hidden={tab !== "admins"}>
        <Administrators />
      </div>
      <div hidden={tab !== "manager"}>
        <ManagerSettings />
      </div>
      <div hidden={tab !== "accounts"}>
        <AccountPanel />
      </div>
    </>
  );
}
