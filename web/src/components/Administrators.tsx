import { useSessionMutation } from "../useSessionMutation";
import { useState } from "react";
import { useUnsavedChanges } from "../useUnsavedChanges";
import type { SyntheticEvent } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  LoaderCircle,
  Plus,
  Save,
  ShieldCheck,
  UserRound,
  X,
} from "lucide-react";
import { api, ApiError, errorMessage, adminsSchema } from "../api";
import type { AdminsDocument } from "../api";
import { EmptyState, Loading, Notice } from "./ui";
import { EditorMode, JsonEditor } from "./JsonEditor";

function AdminsForm({ initial }: { initial: AdminsDocument }) {
  const [baseline, setBaseline] = useState(initial);
  const [text, setText] = useState(JSON.stringify(initial.admins, null, 2));
  const [mode, setMode] = useState<"form" | "raw">("form");
  let admins: string[] = [];
  let invalid = false;
  try {
    admins = adminsSchema.shape.admins.parse(JSON.parse(text) as unknown);
  } catch {
    invalid = true;
  }
  function setAdmins(value: string[]) {
    setText(JSON.stringify(value, null, 2));
  }
  const [name, setName] = useState("");
  const [validation, setValidation] = useState<string | null>(null);
  const client = useQueryClient();
  const mutation = useSessionMutation({
    mutationFn: api.saveAdmins,
    onSuccess: async (result) => {
      setBaseline(result);
      setAdmins(result.admins);
      await client.cancelQueries({ queryKey: ["admins"] });
      client.setQueryData(["admins"], result);
    },
  });
  const reload = useSessionMutation({
    mutationFn: api.admins,
    onSuccess: async (result) => {
      setBaseline(result);
      setAdmins(result.admins);
      mutation.reset();
      setValidation(null);
      await client.cancelQueries({ queryKey: ["admins"] });
      client.setQueryData(["admins"], result);
    },
  });
  function add(event: SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    const value = name.trim();
    if (!value || value.length > 64 || /[\s,]/u.test(value)) {
      setValidation(
        "Enter one Factorio username, without spaces or commas (up to 64 characters).",
      );
      return;
    }
    if (
      admins.some(
        (admin) => admin.toLocaleLowerCase() === value.toLocaleLowerCase(),
      )
    ) {
      setValidation("That player is already on the list.");
      return;
    }
    setAdmins([...admins, value]);
    setName("");
    setValidation(null);
    mutation.reset();
  }
  const dirty = text !== JSON.stringify(baseline.admins, null, 2);
  const busy = mutation.isPending || reload.isPending;
  useUnsavedChanges(dirty);
  return (
    <>
      <div className="toolbar">
        <EditorMode
          mode={mode}
          onChange={setMode}
          busy={busy}
          invalid={invalid}
        />
        <span className="muted">server-adminlist.json</span>
      </div>
      {mode === "raw" ? (
        <JsonEditor
          filename="server-adminlist.json"
          label="Raw administrator list"
          help='A JSON array of Factorio usernames, for example ["Szer", "paragith"]. Applied on the next server start.'
          value={text}
          disabled={busy}
          onChange={(value) => {
            setText(value);
            mutation.reset();
            setValidation(null);
          }}
        />
      ) : (
        <div className="settings-grid">
          <section className="panel admins-panel">
            <div className="section-heading">
              <div>
                <h2>
                  Game administrators{" "}
                  <span className="count-badge">{admins.length}</span>
                </h2>
              </div>
              <ShieldCheck size={21} />
            </div>
            <form className="add-admin-form" onSubmit={add}>
              <label className="field">
                <span className="visually-hidden">Factorio username</span>
                <input
                  placeholder="Factorio username"
                  aria-label="Factorio username"
                  value={name}
                  onChange={(event) => {
                    setName(event.target.value);
                  }}
                  maxLength={64}
                  autoComplete="off"
                />
              </label>
              <button
                className="button secondary"
                disabled={
                  mutation.isPending || reload.isPending || !name.trim()
                }
              >
                <Plus size={16} />
                Add player
              </button>
            </form>
            {validation && <Notice>{validation}</Notice>}
            {admins.length === 0 ? (
              <EmptyState
                icon={<ShieldCheck size={29} />}
                title="No administrators"
              >
                Add your Factorio username to become a game administrator.
              </EmptyState>
            ) : (
              <ul className="admin-list">
                {admins.map((admin, index) => (
                  <li key={`${admin}:${String(index)}`}>
                    <span className="player-avatar">
                      <UserRound size={19} />
                    </span>
                    <span>
                      <strong>{admin}</strong>
                    </span>
                    <button
                      className="icon-button"
                      aria-label={`Remove ${admin}`}
                      disabled={mutation.isPending || reload.isPending}
                      onClick={() => {
                        setAdmins(
                          admins.filter((_, position) => position !== index),
                        );
                        mutation.reset();
                      }}
                    >
                      <X size={17} />
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>
          <aside className="settings-aside">
            <span className="aside-icon">
              <ShieldCheck size={20} />
            </span>
            <p>
              Use their exact Factorio username. These are game permissions,
              separate from access to this control room.
            </p>
            <div className="aside-divider" />
            <p>Changes take effect when the Factorio server starts again.</p>
          </aside>
        </div>
      )}
      {mode === "raw" && validation && <Notice>{validation}</Notice>}
      {mutation.isError && (
        <Notice>
          {errorMessage(mutation.error)}
          {mutation.error instanceof ApiError &&
            mutation.error.status === 409 && (
              <button
                className="text-button"
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
      {mutation.isSuccess && !dirty && (
        <Notice success>
          Administrators saved. Restart the server to apply the list.
        </Notice>
      )}
      <div className="save-bar">
        <span>
          <span className={`tiny-dot ${dirty ? "amber" : ""}`} />
          {dirty
            ? "You have unsaved changes"
            : "Administrator list is up to date"}
        </span>
        <div>
          <button
            className="button secondary"
            disabled={!dirty || busy}
            onClick={() => {
              setAdmins(baseline.admins);
              mutation.reset();
              setValidation(null);
            }}
          >
            Discard
          </button>
          <button
            className="button primary"
            disabled={!dirty || busy}
            onClick={() => {
              if (invalid) {
                setValidation(
                  "Enter a JSON array of usernames. Your draft has been kept.",
                );
                return;
              }
              setValidation(null);
              mutation.mutate({ revision: baseline.revision, admins });
            }}
          >
            {mutation.isPending ? (
              <LoaderCircle size={16} className="spin" />
            ) : (
              <Save size={16} />
            )}
            Save administrators
          </button>
        </div>
      </div>
    </>
  );
}
export default function Administrators() {
  const query = useQuery({
    queryKey: ["admins"],
    queryFn: api.admins,
    refetchOnWindowFocus: false,
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
      <AdminsForm initial={query.data} />
    </>
  );
}
