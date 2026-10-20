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
import ServerControls from "../components/ServerControls";

export default function Worlds({ status }: { status: Status }) {
  const [search, setSearch] = useState("");
  const [fileError, setFileError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const saves = useQuery({ queryKey: ["saves"], queryFn: api.saves });
  const client = useQueryClient();
  const upload = useMutation({
    mutationFn: api.upload,
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: ["saves"] });
    },
  });
  const worlds =
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
      {upload.isPending ? "Uploading…" : "Upload world"}
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
            aria-label="Search worlds"
            placeholder="Find a world…"
            value={search}
            onChange={(event) => {
              setSearch(event.target.value);
            }}
          />
        </label>
        <div className="toolbar-actions">
          <ServerControls status={status} />
          {uploadButton}
        </div>
      </div>
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
            title="Your next world starts here."
            action={uploadButton}
          >
            Upload a Factorio save to continue your factory. ZIP files up to 64
            MB; existing worlds are kept intact.
          </EmptyState>
        </section>
      ) : (
        <section className="panel worlds-panel">
          <div className="section-heading">
            <h2>
              Your worlds{" "}
              <span className="count-badge">{saves.data.length}</span>
            </h2>
            <span className="small-label">STORED LOCALLY</span>
          </div>
          <div className="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>WORLD</th>
                  <th>LAST MODIFIED</th>
                  <th>SIZE</th>
                  <th>
                    <span className="visually-hidden">Actions</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {worlds.map((save) => (
                  <tr key={save.name}>
                    <td>
                      <div className="world-name">
                        <span className="file-icon">
                          <FileArchive size={19} />
                        </span>
                        <div>
                          <strong>{save.name.replace(/\.zip$/i, "")}</strong>
                          <small>
                            {save.name}
                            {status.active_save === save.name
                              ? " · Current world"
                              : ""}
                          </small>
                        </div>
                      </div>
                    </td>
                    <td>{formatDate(save.modified_at)}</td>
                    <td className="mono">{formatBytes(save.size_bytes)}</td>
                    <td>
                      <a
                        className="icon-button"
                        href={`/api/saves/${encodeURIComponent(save.name)}`}
                        download={save.name}
                        aria-label={`Download ${save.name}`}
                      >
                        <Download size={17} />
                      </a>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {worlds.length === 0 && (
            <div className="search-empty">No worlds match “{search}”.</div>
          )}
          <div className="table-footer">
            <span>
              {worlds.length} {worlds.length === 1 ? "world" : "worlds"}
            </span>
            <span>Your original save is never overwritten by an upload.</span>
          </div>
        </section>
      )}
      <div className="subtle-note">
        <FileArchive size={16} />
        <p>
          Bring a save from your single-player game or another server. Select
          the world you want when starting the server.
        </p>
      </div>
    </>
  );
}
