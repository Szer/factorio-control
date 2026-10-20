import { useState } from "react";
import { useUnsavedChanges } from "../useUnsavedChanges";
import type { SyntheticEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  LoaderCircle,
  Plus,
  Save,
  ShieldCheck,
  UserRound,
  X,
} from "lucide-react";
import { api, ApiError, errorMessage } from "../api";
import type { AdminsDocument } from "../api";
import { EmptyState, Loading, Notice } from "../components/ui";

function AdminsForm({ initial }: { initial: AdminsDocument }) {
  const [baseline, setBaseline] = useState(initial);
  const [admins, setAdmins] = useState(initial.admins);
  const [name, setName] = useState("");
  const [validation, setValidation] = useState<string | null>(null);
  const client = useQueryClient();
  const mutation = useMutation({
    mutationFn: api.saveAdmins,
    onSuccess: (result) => {
      setBaseline(result);
      setAdmins(result.admins);
      client.setQueryData(["admins"], result);
    },
  });
  const reload = useMutation({
    mutationFn: api.admins,
    onSuccess: (result) => {
      setBaseline(result);
      setAdmins(result.admins);
      mutation.reset();
      setValidation(null);
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
    setAdmins((previous) => [...previous, value]);
    setName("");
    setValidation(null);
    mutation.reset();
  }
  const dirty = JSON.stringify(admins) !== JSON.stringify(baseline.admins);
  useUnsavedChanges(dirty);
  return (
    <>
      <div className="settings-grid">
        <section className="panel admins-panel">
          <div className="section-heading">
            <div>
              <h2>
                Game administrators{" "}
                <span className="count-badge">{admins.length}</span>
              </h2>
              <p>Trusted players with permission to manage the game.</p>
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
              disabled={mutation.isPending || reload.isPending || !name.trim()}
            >
              <Plus size={16} />
              Add player
            </button>
          </form>
          {validation && <Notice>{validation}</Notice>}
          {admins.length === 0 ? (
            <EmptyState
              icon={<ShieldCheck size={29} />}
              title="A trusted crew starts with you."
            >
              Add your Factorio username to become a game administrator.
            </EmptyState>
          ) : (
            <ul className="admin-list">
              {admins.map((admin) => (
                <li key={admin}>
                  <span className="player-avatar">
                    <UserRound size={19} />
                  </span>
                  <span>
                    <strong>{admin}</strong>
                    <small>Game administrator</small>
                  </span>
                  <span className="admin-role">Admin</span>
                  <button
                    className="icon-button"
                    aria-label={`Remove ${admin}`}
                    disabled={mutation.isPending || reload.isPending}
                    onClick={() => {
                      setAdmins((previous) =>
                        previous.filter((value) => value !== admin),
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
          <h3>The keys to the factory.</h3>
          <p>
            Administrators can run game commands and manage other players. Only
            add people you trust.
          </p>
          <p>
            Use their exact Factorio username. These are game permissions,
            separate from access to this control room.
          </p>
          <div className="aside-divider" />
          <p>Changes take effect when the Factorio server starts again.</p>
        </aside>
      </div>
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
            disabled={!dirty || mutation.isPending}
            onClick={() => {
              setAdmins(baseline.admins);
              mutation.reset();
            }}
          >
            Discard
          </button>
          <button
            className="button primary"
            disabled={!dirty || mutation.isPending}
            onClick={() => {
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
export default function Players() {
  const query = useQuery({
    queryKey: ["admins"],
    queryFn: api.admins,
    refetchOnWindowFocus: false,
  });
  if (query.isPending) return <Loading />;
  if (query.isError) return <Notice>{errorMessage(query.error)}</Notice>;
  return <AdminsForm initial={query.data} />;
}
