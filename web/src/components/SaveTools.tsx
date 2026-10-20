import { useSessionMutation } from "../useSessionMutation";
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Archive, Plus, RotateCcw } from "lucide-react";
import { errorMessage, formatBytes, formatDate } from "../api";
import { operations } from "../operationsApi";
import { Loading, Modal, Notice } from "./ui";

export function SaveTools({ stopped }: { stopped: boolean }) {
  const [dialog, setDialog] = useState<"create" | "archive" | null>(null);
  const [name, setName] = useState("");
  const create = useSessionMutation({
    mutationFn: operations.createSave,
    onSuccess: async (_result, _variables, session) => {
      await session.refresh([["saves"]]);
      session.commit(() => {
        setDialog(null);
        setName("");
      });
    },
  });
  return (
    <>
      <button
        className="button secondary"
        disabled={!stopped}
        title={!stopped ? "Stop the server to create a save" : undefined}
        onClick={() => {
          create.reset();
          setDialog("create");
        }}
      >
        <Plus size={16} />
        New save
      </button>
      <button
        className="button secondary"
        onClick={() => {
          setDialog("archive");
        }}
      >
        <Archive size={16} />
        Archived
      </button>
      {dialog === "create" && (
        <Modal
          title="Create save"
          busy={create.isPending}
          close={() => {
            setDialog(null);
          }}
        >
          <form
            onSubmit={(event) => {
              event.preventDefault();
              const trimmed = name.trim();
              create.mutate(
                /\.zip$/iu.test(trimmed) ? trimmed : `${trimmed}.zip`,
              );
            }}
          >
            <div className="modal-body">
              <label className="field">
                Name
                <input
                  value={name}
                  maxLength={100}
                  required
                  autoFocus
                  disabled={create.isPending}
                  onChange={(event) => {
                    setName(event.target.value);
                  }}
                  placeholder="new-factory"
                />
              </label>
              <p className="muted">
                Uses Factorio’s default map generation and your enabled mods.
              </p>
              {create.isError && <Notice>{errorMessage(create.error)}</Notice>}
            </div>
            <div className="modal-actions">
              <button
                type="button"
                className="button secondary"
                disabled={create.isPending}
                onClick={() => {
                  setDialog(null);
                }}
              >
                Cancel
              </button>
              <button
                className="button primary"
                disabled={!stopped || create.isPending || !name.trim()}
              >
                {create.isPending ? "Generating…" : "Create save"}
              </button>
            </div>
          </form>
        </Modal>
      )}
      {dialog === "archive" && (
        <ArchivedSaves
          stopped={stopped}
          close={() => {
            setDialog(null);
          }}
        />
      )}
    </>
  );
}
function ArchivedSaves({
  stopped,
  close,
}: {
  stopped: boolean;
  close: () => void;
}) {
  const query = useQuery({
    queryKey: ["archived-saves"],
    queryFn: operations.archivedSaves,
  });
  const restore = useSessionMutation({
    mutationFn: operations.restoreSave,
    onSuccess: async (_result, _variables, session) => {
      await session.refresh([["saves"], ["archived-saves"]]);
    },
  });
  return (
    <Modal
      title="Archived saves"
      close={close}
      busy={restore.isPending}
      className="save-archive-modal"
    >
      {query.isPending ? (
        <Loading />
      ) : query.isError ? (
        <Notice>{errorMessage(query.error)}</Notice>
      ) : query.data.length === 0 ? (
        <p className="modal-body muted">No archived saves.</p>
      ) : (
        <div className="table-scroll">
          <table>
            <thead>
              <tr>
                <th>Name</th>
                <th>Modified</th>
                <th>Size</th>
                <th>Actions</th>
              </tr>
            </thead>
            <tbody>
              {query.data.map((save) => (
                <tr key={save.id}>
                  <td>{save.name}</td>
                  <td>{formatDate(save.modified_at)}</td>
                  <td>{formatBytes(save.size_bytes)}</td>
                  <td>
                    <button
                      className="button secondary"
                      disabled={!stopped || restore.isPending}
                      onClick={() => {
                        restore.mutate({
                          id: save.id,
                          revision: save.revision,
                        });
                      }}
                    >
                      <RotateCcw size={16} />
                      Restore
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {restore.isError && <Notice>{errorMessage(restore.error)}</Notice>}
    </Modal>
  );
}
export function ArchiveSaveButton({
  name,
  revision,
  disabled,
  reason,
}: {
  name: string;
  revision: string;
  disabled: boolean;
  reason: string | undefined;
}) {
  const [open, setOpen] = useState(false);
  const mutation = useSessionMutation({
    mutationFn: operations.archiveSave,
    onSuccess: async (_result, _variables, session) => {
      await session.refresh([["saves"], ["archived-saves"]]);
      session.commit(() => {
        setOpen(false);
      });
    },
  });
  return (
    <>
      <button
        className="icon-button"
        aria-label={`Archive ${name}`}
        title={reason ?? "Archive save"}
        disabled={disabled || mutation.isPending}
        onClick={() => {
          mutation.reset();
          setOpen(true);
        }}
      >
        <Archive size={17} />
      </button>
      {open && (
        <Modal
          title="Archive save?"
          close={() => {
            setOpen(false);
          }}
          busy={mutation.isPending}
        >
          <p className="modal-description">
            <strong>{name}</strong> will move to Archived saves. You can restore
            it later.
          </p>
          {mutation.isError && <Notice>{errorMessage(mutation.error)}</Notice>}
          <div className="modal-actions">
            <button
              className="button secondary"
              disabled={mutation.isPending}
              onClick={() => {
                setOpen(false);
              }}
            >
              Cancel
            </button>
            <button
              className="button primary"
              disabled={disabled || mutation.isPending}
              onClick={() => {
                mutation.mutate({ name, revision });
              }}
            >
              {mutation.isPending ? "Archiving…" : "Archive save"}
            </button>
          </div>
        </Modal>
      )}
    </>
  );
}
