import { useState } from "react";
import { useUnsavedChanges } from "../useUnsavedChanges";
import type { SyntheticEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Check,
  Globe2,
  LoaderCircle,
  Save,
  Settings2,
  Timer,
} from "lucide-react";
import { api, ApiError, errorMessage, settingsSchema } from "../api";
import type { Settings, SettingsDocument } from "../api";
import { Loading, Notice } from "../components/ui";

function SettingsForm({ initial }: { initial: SettingsDocument }) {
  const [baseline, setBaseline] = useState(initial);
  const [draft, setDraft] = useState(initial.settings);
  const [validation, setValidation] = useState<string | null>(null);
  const client = useQueryClient();
  const mutation = useMutation({
    mutationFn: api.saveSettings,
    onSuccess: (result) => {
      setBaseline(result);
      setDraft(result.settings);
      client.setQueryData(["settings"], result);
    },
  });
  const reload = useMutation({
    mutationFn: api.settings,
    onSuccess: (result) => {
      setBaseline(result);
      setDraft(result.settings);
      mutation.reset();
      setValidation(null);
      client.setQueryData(["settings"], result);
    },
  });
  const dirty = JSON.stringify(draft) !== JSON.stringify(baseline.settings);
  useUnsavedChanges(dirty);
  function update<K extends keyof Settings>(key: K, value: Settings[K]) {
    setDraft((previous) => ({ ...previous, [key]: value }));
    mutation.reset();
  }
  function submit(event: SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    setValidation(null);
    const parsed = settingsSchema.safeParse(draft);
    if (!parsed.success) {
      setValidation(
        parsed.error.issues[0]?.message ?? "Check the highlighted settings.",
      );
      return;
    }
    mutation.mutate({ revision: baseline.revision, settings: parsed.data });
  }
  const conflict =
    mutation.error instanceof ApiError && mutation.error.status === 409;
  return (
    <form onSubmit={submit} className="settings-form">
      <div className="settings-grid">
        <div className="settings-sections">
          <section className="panel form-panel">
            <div className="section-heading">
              <div className="section-title-icon">
                <Settings2 size={19} />
                <div>
                  <h2>Server identity</h2>
                  <p>A familiar name for your corner of the universe.</p>
                </div>
              </div>
            </div>
            <div className="form-fields">
              <label className="field">
                Server name
                <input
                  disabled={mutation.isPending || reload.isPending}
                  required
                  maxLength={100}
                  value={draft.name}
                  onChange={(event) => {
                    update("name", event.target.value);
                  }}
                />
                <small>Shown to players in the server browser.</small>
              </label>
              <label className="field">
                Description
                <textarea
                  disabled={mutation.isPending || reload.isPending}
                  rows={3}
                  maxLength={2000}
                  placeholder="What are we building?"
                  value={draft.description}
                  onChange={(event) => {
                    update("description", event.target.value);
                  }}
                />
              </label>
            </div>
          </section>
          <section className="panel form-panel">
            <div className="section-heading">
              <div className="section-title-icon">
                <Globe2 size={19} />
                <div>
                  <h2>Joining your world</h2>
                  <p>Choose who can find your server.</p>
                </div>
              </div>
            </div>
            <div className="form-fields">
              <label className="field compact-field">
                Player limit
                <input
                  disabled={mutation.isPending || reload.isPending}
                  type="number"
                  min={0}
                  max={65535}
                  required
                  value={draft.max_players}
                  onChange={(event) => {
                    update("max_players", event.target.valueAsNumber);
                  }}
                />
                <small>Use 0 for unlimited players.</small>
              </label>
              <label className="toggle-row">
                <span>
                  <strong>Visible on your local network</strong>
                  <small>Let nearby players find this server on LAN.</small>
                </span>
                <input
                  disabled={mutation.isPending || reload.isPending}
                  type="checkbox"
                  role="switch"
                  checked={draft.visibility.lan}
                  onChange={(event) => {
                    update("visibility", {
                      ...draft.visibility,
                      lan: event.target.checked,
                    });
                  }}
                />
              </label>
              <label className="toggle-row">
                <span>
                  <strong>Listed in the public server browser</strong>
                  <small>
                    Requires Factorio credentials in your server configuration.
                  </small>
                </span>
                <input
                  disabled={mutation.isPending || reload.isPending}
                  type="checkbox"
                  role="switch"
                  checked={draft.visibility.public}
                  onChange={(event) => {
                    update("visibility", {
                      ...draft.visibility,
                      public: event.target.checked,
                    });
                  }}
                />
              </label>
            </div>
          </section>
          <section className="panel form-panel">
            <div className="section-heading">
              <div className="section-title-icon">
                <Timer size={19} />
                <div>
                  <h2>Keep things ticking</h2>
                  <p>Small settings for a smoother session.</p>
                </div>
              </div>
            </div>
            <div className="form-fields">
              <label className="field compact-field">
                Autosave interval (minutes)
                <input
                  disabled={mutation.isPending || reload.isPending}
                  type="number"
                  min={0}
                  max={1440}
                  required
                  value={draft.autosave_interval}
                  onChange={(event) => {
                    update("autosave_interval", event.target.valueAsNumber);
                  }}
                />
                <small>Use 0 to disable automatic saves.</small>
              </label>
              <label className="toggle-row">
                <span>
                  <strong>Pause when the server is empty</strong>
                  <small>Your factory waits until someone comes back.</small>
                </span>
                <input
                  disabled={mutation.isPending || reload.isPending}
                  type="checkbox"
                  role="switch"
                  checked={draft.auto_pause}
                  onChange={(event) => {
                    update("auto_pause", event.target.checked);
                  }}
                />
              </label>
            </div>
          </section>
        </div>
        <aside className="settings-aside">
          <span className="aside-icon">
            <Check size={20} />
          </span>
          <h3>Saved means saved.</h3>
          <p>Changes are confirmed only after they’re written to disk.</p>
          <p>Restart the Factorio server to apply updated settings.</p>
          <div className="aside-divider" />
          <span className="small-label">ADMINISTRATORS</span>
          <p>
            Manage trusted Factorio players from the{" "}
            <a href="#players">
              Players page <span aria-hidden="true">↗</span>
            </a>
            .
          </p>
        </aside>
      </div>
      {validation && <Notice>{validation}</Notice>}
      {mutation.isError && (
        <Notice>
          {errorMessage(mutation.error)}
          {conflict && (
            <button
              type="button"
              className="text-button"
              onClick={() => {
                reload.mutate();
              }}
              disabled={reload.isPending}
            >
              Discard edits & reload
            </button>
          )}
        </Notice>
      )}
      {reload.isError && <Notice>{errorMessage(reload.error)}</Notice>}
      {mutation.isSuccess && !dirty && (
        <Notice success>
          Settings saved. Restart the server to apply them.
        </Notice>
      )}
      <div className="save-bar">
        <span>
          <span className={`tiny-dot ${dirty ? "amber" : ""}`} />
          {dirty ? "You have unsaved changes" : "All changes saved"}
        </span>
        <div>
          <button
            type="button"
            className="button secondary"
            disabled={!dirty || mutation.isPending}
            onClick={() => {
              setDraft(baseline.settings);
              mutation.reset();
              setValidation(null);
            }}
          >
            Discard
          </button>
          <button
            className="button primary"
            disabled={!dirty || mutation.isPending}
          >
            {mutation.isPending ? (
              <LoaderCircle size={16} className="spin" />
            ) : (
              <Save size={16} />
            )}
            {mutation.isPending ? "Saving…" : "Save changes"}
          </button>
        </div>
      </div>
    </form>
  );
}
export default function SettingsPage() {
  const query = useQuery({
    queryKey: ["settings"],
    queryFn: api.settings,
    refetchOnWindowFocus: false,
  });
  if (query.isPending) return <Loading />;
  if (query.isError) return <Notice>{errorMessage(query.error)}</Notice>;
  return <SettingsForm initial={query.data} />;
}
