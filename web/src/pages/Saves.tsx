import { useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Box,
  Download,
  FileArchive,
  LoaderCircle,
  Search,
  RefreshCw,
  Upload,
} from "lucide-react";
import { api, errorMessage, formatBytes, formatDate } from "../api";
import type { Status, SaveSyncPlan } from "../api";
import { EmptyState, Loading, Modal, Notice } from "../components/ui";

export default function Saves({ status }: { status: Status }) {
  const [syncPlan, setSyncPlan] = useState<SaveSyncPlan | null>(null);
  const [operationId, setOperationId] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [fileError, setFileError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const saves = useQuery({ queryKey: ["saves"], queryFn: api.saves });
  const client = useQueryClient();
  const selection = useQuery({
    queryKey: ["selection"],
    queryFn: api.selection,
  });
  const promote = useMutation({
    mutationFn: api.selectSave,
    onSuccess: (result) => {
      client.setQueryData(["selection"], result);
    },
  });
  const upload = useMutation({
    mutationFn: api.upload,
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: ["saves"] });
    },
  });
  const sync = useMutation({
    mutationFn: api.syncSaveMods,
    onSuccess: (result) => {
      client.setQueryData(["mods"], result);
      setSyncPlan(null);
    },
  });
  const review = useMutation({
    mutationFn: api.saveModPlan,
    onSuccess: (result) => {
      setSyncPlan(result);
      sync.reset();
    },
  });
  const progress = useQuery({
    queryKey: ["save-sync-progress", syncPlan?.save, operationId],
    queryFn: () => api.saveSyncProgress(syncPlan?.save ?? ""),
    enabled: sync.isPending && syncPlan !== null && operationId !== null,
    refetchInterval: 250,
    retry: false,
  });
  const currentProgress =
    progress.data?.operation_id === operationId ? progress.data : null;
  const progressLabel = currentProgress
    ? {
        preparing: "Preparing sync",
        resolving: "Finding release",
        downloading: "Downloading",
        validating: "Validating mod dependencies",
        applying: "Activating mod set",
        complete: "Sync complete",
        failed: "Sync failed",
      }[currentProgress.phase]
    : "Preparing sync";
  const stopped = status.state === "stopped" || status.state === "failed";
  const filtered =
    saves.data?.filter((save) =>
      save.name.toLocaleLowerCase().includes(search.toLocaleLowerCase()),
    ) ?? [];
  function uploadFile(file: File | undefined) {
    if (!file) return;
    setFileError(null);
    upload.reset();
    if (!file.name.toLocaleLowerCase().endsWith(".zip")) {
      setFileError("Choose a Factorio save in ZIP format.");
      return;
    }
    if (file.size > 1024 * 1024 * 1024) {
      setFileError("This save is larger than the 1 GiB upload limit.");
      return;
    }
    upload.mutate(file);
  }
  const uploadButton = (
    <button
      className="button primary"
      disabled={upload.isPending}
      onClick={() => {
        input.current?.click();
      }}
    >
      {upload.isPending ? (
        <LoaderCircle size={16} className="spin" />
      ) : (
        <Upload size={16} />
      )}
      {upload.isPending ? "Uploading…" : "Upload save"}
    </button>
  );
  return (
    <>
      <input
        className="visually-hidden"
        aria-label="Upload Factorio save"
        ref={input}
        type="file"
        accept=".zip,application/zip"
        onChange={(event) => {
          uploadFile(event.target.files?.[0]);
          event.target.value = "";
        }}
      />
      <div className="toolbar">
        <label className="search-box">
          <Search size={17} />
          <input
            aria-label="Search saves"
            placeholder="Find a save…"
            value={search}
            onChange={(event) => {
              setSearch(event.target.value);
            }}
          />
        </label>
        <div className="toolbar-actions">{uploadButton}</div>
      </div>
      <div className="selection-banner">
        <span>Next server start</span>
        <strong>{selection.data?.save ?? "No save selected"}</strong>
      </div>
      {selection.isError && <Notice>{errorMessage(selection.error)}</Notice>}
      {promote.isError && (
        <Notice>
          {errorMessage(promote.error)}{" "}
          <button
            className="text-button"
            disabled={selection.isFetching}
            onClick={() => {
              client
                .invalidateQueries({ queryKey: ["selection"] })
                .catch(() => undefined);
              promote.reset();
            }}
          >
            Refresh selection
          </button>
        </Notice>
      )}
      {promote.isSuccess && (
        <Notice success>
          {promote.data.save} will be used on the next start.
        </Notice>
      )}
      {fileError && <Notice>{fileError}</Notice>}
      {upload.isError && <Notice>{errorMessage(upload.error)}</Notice>}
      {upload.isSuccess && (
        <Notice success>{upload.data.name} is ready to load.</Notice>
      )}
      {review.isPending && <Notice>Reading the save's mod list…</Notice>}
      {review.isError && !syncPlan && (
        <Notice>{errorMessage(review.error)}</Notice>
      )}
      {sync.isSuccess && (
        <Notice success>
          Mods synced. Select this save for the next server start when you are
          ready.
        </Notice>
      )}
      {syncPlan && (
        <Modal
          title={`Sync mods with ${syncPlan.save}`}
          className="sync-dialog"
          busy={sync.isPending || review.isPending}
          close={() => {
            setSyncPlan(null);
            sync.reset();
          }}
        >
          <div className="sync-summary">
            <p>
              Factorio {syncPlan.factorio_version} → {syncPlan.engine_version}
            </p>
            <p>
              {syncPlan.mods.length} mods ·{" "}
              {
                syncPlan.mods.filter((item) => item.action === "download")
                  .length
              }{" "}
              downloads · {syncPlan.disable.length} to disable
            </p>
          </div>
          <div className="sync-requirements">
            {syncPlan.blockers.length > 0 && (
              <Notice>
                {syncPlan.blockers[0]}
                {syncPlan.blockers.length > 1 && (
                  <details>
                    <summary>
                      {syncPlan.blockers.length - 1} more requirements
                    </summary>
                    <ul>
                      {syncPlan.blockers.slice(1).map((message) => (
                        <li key={message}>{message}</li>
                      ))}
                    </ul>
                  </details>
                )}
                <a
                  className="text-button"
                  href="#settings"
                  onClick={() => {
                    setSyncPlan(null);
                  }}
                >
                  Open server settings
                </a>
              </Notice>
            )}
            {!stopped && <Notice>Stop the server before syncing mods.</Notice>}
            {sync.isError && <Notice>{errorMessage(sync.error)}</Notice>}
            {review.isError && <Notice>{errorMessage(review.error)}</Notice>}
          </div>
          {sync.isPending && (
            <div className="sync-progress" role="status" aria-live="polite">
              <div>
                <strong>
                  {progressLabel}
                  {currentProgress?.current_mod
                    ? `: ${currentProgress.current_mod}`
                    : "…"}
                </strong>
                {currentProgress && currentProgress.total > 0 && (
                  <span>
                    {currentProgress.completed} / {currentProgress.total}{" "}
                    downloaded
                  </span>
                )}
              </div>
              <progress
                aria-label="Mod download progress"
                max={currentProgress?.total || 1}
                value={
                  currentProgress?.total ? currentProgress.completed : undefined
                }
              />
              {progress.isError && (
                <small>
                  Progress temporarily unavailable. Sync is still running.
                </small>
              )}
            </div>
          )}
          <div
            className="sync-mod-list"
            role="region"
            aria-label="Mod changes"
            tabIndex={0}
          >
            <table>
              <thead>
                <tr>
                  <th>Mod</th>
                  <th>Version in save</th>
                  <th>Action</th>
                </tr>
              </thead>
              <tbody>
                {syncPlan.mods.map((item) => (
                  <tr
                    key={item.name}
                    className={
                      sync.isPending &&
                      currentProgress?.current_mod === item.name
                        ? "sync-current-mod"
                        : undefined
                    }
                  >
                    <td>{item.name}</td>
                    <td>{item.version}</td>
                    <td>
                      {
                        {
                          keep: "Already matched",
                          enable: "Enable",
                          switch: "Switch installed version",
                          download: "Download exact version",
                          bundled: "Use bundled mod",
                        }[item.action]
                      }
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {syncPlan.disable.length > 0 && (
              <div className="sync-details">
                <strong>Will disable:</strong> {syncPlan.disable.join(", ")}
              </div>
            )}
          </div>
          <details className="sync-explanation">
            <summary>How sync works</summary>
            <p>
              Matches exact mod versions, downloads missing ZIPs, and disables
              extra mods. Existing ZIPs and the save are kept. Bundled mods use
              the installed game version. Startup settings can be imported
              separately from mod-settings.dat.
            </p>
          </details>
          <div className="sync-actions">
            <button
              className="button secondary"
              disabled={sync.isPending || review.isPending}
              onClick={() => {
                setSyncPlan(null);
                sync.reset();
              }}
            >
              Cancel
            </button>
            <button
              className="button secondary"
              disabled={sync.isPending || review.isPending}
              onClick={() => {
                review.mutate(syncPlan.save);
              }}
            >
              Refresh plan
            </button>
            <button
              className="button primary"
              disabled={
                !stopped ||
                sync.isPending ||
                review.isPending ||
                sync.isError ||
                syncPlan.blockers.length > 0
              }
              onClick={() => {
                const id = crypto.randomUUID();
                setOperationId(id);
                sync.mutate({
                  name: syncPlan.save,
                  revision: syncPlan.revision,
                  operation_id: id,
                });
              }}
            >
              {sync.isPending ? "Syncing mods…" : "Apply mod sync"}
            </button>
          </div>
        </Modal>
      )}
      {saves.isPending ? (
        <Loading />
      ) : saves.isError ? (
        <Notice>{errorMessage(saves.error)}</Notice>
      ) : saves.data.length === 0 ? (
        <section className="panel">
          <EmptyState
            icon={<Box size={32} />}
            title="No saves uploaded"
            action={uploadButton}
          >
            Upload a Factorio save to continue your factory. ZIP files up to 1
            GiB; existing saves are kept intact.
          </EmptyState>
        </section>
      ) : (
        <section className="panel saves-panel">
          <div className="section-heading">
            <h2>
              Saves <span className="count-badge">{saves.data.length}</span>
            </h2>
          </div>
          <div className="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>SAVE</th>
                  <th>LAST MODIFIED</th>
                  <th>SIZE</th>
                  <th>
                    <span className="visually-hidden">Actions</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {filtered.map((save) => (
                  <tr key={save.name}>
                    <td>
                      <div className="save-name">
                        <span className="file-icon">
                          <FileArchive size={19} />
                        </span>
                        <div>
                          <strong>{save.name}</strong>
                          {status.active_save === save.name && (
                            <small>Loaded session</small>
                          )}
                        </div>
                      </div>
                    </td>
                    <td>{formatDate(save.modified_at)}</td>
                    <td className="mono">{formatBytes(save.size_bytes)}</td>
                    <td>
                      <div className="row-actions">
                        <button
                          className={
                            selection.data?.save === save.name
                              ? "button selected-save"
                              : "button secondary"
                          }
                          disabled={
                            promote.isPending ||
                            !selection.data ||
                            selection.data.save === save.name
                          }
                          onClick={() => {
                            if (selection.data)
                              promote.mutate({
                                revision: selection.data.revision,
                                save: save.name,
                              });
                          }}
                        >
                          {selection.data?.save === save.name
                            ? "Selected"
                            : "Use on next start"}
                        </button>
                        <button
                          className="button secondary"
                          disabled={review.isPending || sync.isPending}
                          onClick={() => {
                            setSyncPlan(null);
                            review.mutate(save.name);
                          }}
                        >
                          <RefreshCw size={16} />
                          Sync mods
                        </button>
                        <a
                          className="icon-button"
                          href={`/api/saves/${encodeURIComponent(save.name)}`}
                          download={save.name}
                          aria-label={`Download ${save.name}`}
                        >
                          <Download size={17} />
                        </a>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {filtered.length === 0 && (
            <div className="search-empty">No saves match “{search}”.</div>
          )}
        </section>
      )}
    </>
  );
}
