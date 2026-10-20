import { useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Box,
  Download,
  FileArchive,
  LoaderCircle,
  Search,
  Upload,
} from "lucide-react";
import { api, errorMessage, formatBytes, formatDate } from "../api";
import type { Status } from "../api";
import { EmptyState, Loading, Notice } from "../components/ui";

export default function Saves({ status }: { status: Status }) {
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
    if (file.size > 64 * 1024 * 1024) {
      setFileError("This save is larger than the 64 MB upload limit.");
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
        <span className="muted">
          Selecting a save takes effect on the next start.
        </span>
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
            Upload a Factorio save to continue your factory. ZIP files up to 64
            MB; existing saves are kept intact.
          </EmptyState>
        </section>
      ) : (
        <section className="panel saves-panel">
          <div className="section-heading">
            <h2>
              Saves <span className="count-badge">{saves.data.length}</span>
            </h2>
            <span className="small-label">STORED LOCALLY</span>
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
                          <strong>{save.name.replace(/\.zip$/i, "")}</strong>
                          <small>
                            {save.name}
                            {status.active_save === save.name
                              ? " · Loaded session"
                              : ""}
                          </small>
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
          <div className="table-footer">
            <span>
              {filtered.length} {filtered.length === 1 ? "save" : "saves"}
            </span>
            <span>Your original save is never overwritten by an upload.</span>
          </div>
        </section>
      )}
      <div className="subtle-note">
        <FileArchive size={16} />
        <p>
          Bring a save from your single-player game or another server. Choose
          “Use on next start” to promote a save to the startup save.
        </p>
      </div>
    </>
  );
}
