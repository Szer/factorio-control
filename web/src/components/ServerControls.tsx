import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { LoaderCircle, Play, Square } from "lucide-react";
import { api, errorMessage } from "../api";
import type { Status } from "../api";
import { Modal, Notice } from "./ui";

export default function ServerControls({ status }: { status: Status }) {
  const [dialog, setDialog] = useState<"start" | "stop" | null>(null);
  const [selected, setSelected] = useState("");
  const saves = useQuery({ queryKey: ["saves"], queryFn: api.saves });
  const client = useQueryClient();
  const mutation = useMutation({
    mutationFn: (action: { kind: "start"; save: string } | { kind: "stop" }) =>
      action.kind === "start" ? api.start(action.save) : api.stop(),
    onSuccess: async (result) => {
      client.setQueryData(["status"], result);
      setDialog(null);
      await client.invalidateQueries({ queryKey: ["logs"] });
    },
  });
  const canStart = status.binary_available && (saves.data?.length ?? 0) > 0;
  const transitional =
    status.state === "starting" || status.state === "stopping";
  function open(kind: "start" | "stop") {
    mutation.reset();
    setSelected(saves.data?.[0]?.name ?? "");
    setDialog(kind);
  }
  return (
    <>
      {status.state === "running" || status.state === "starting" ? (
        <button
          className="button secondary"
          onClick={() => {
            open("stop");
          }}
        >
          <Square size={14} />
          Stop server
        </button>
      ) : (
        <button
          className="button primary"
          disabled={!canStart || transitional}
          onClick={() => {
            open("start");
          }}
          title={
            !status.binary_available
              ? "Connect the Factorio binary to start your server."
              : undefined
          }
        >
          {transitional ? (
            <LoaderCircle size={16} className="spin" />
          ) : (
            <Play size={15} fill="currentColor" />
          )}
          {status.state === "stopping" ? "Stopping…" : "Start server"}
        </button>
      )}
      {dialog && (
        <Modal
          title={dialog === "start" ? "Start your factory" : "Stop the server?"}
          close={() => {
            if (!mutation.isPending) setDialog(null);
          }}
        >
          <p className="modal-description">
            {dialog === "start"
              ? "Choose the world to load. The server will use your saved settings."
              : "Connected players will be disconnected. Factorio will be asked to save and shut down gracefully."}
          </p>
          {dialog === "start" && (
            <label className="field">
              World
              <select
                value={selected}
                onChange={(event) => {
                  setSelected(event.target.value);
                }}
              >
                {saves.data?.map((save) => (
                  <option key={save.name} value={save.name}>
                    {save.name}
                  </option>
                ))}
              </select>
            </label>
          )}
          {mutation.isError && <Notice>{errorMessage(mutation.error)}</Notice>}
          <div className="modal-actions">
            <button
              className="button secondary"
              disabled={mutation.isPending}
              onClick={() => {
                setDialog(null);
              }}
            >
              Cancel
            </button>
            <button
              className={`button ${dialog === "stop" ? "danger" : "primary"}`}
              disabled={
                mutation.isPending || (dialog === "start" && selected === "")
              }
              onClick={() => {
                mutation.mutate(
                  dialog === "start"
                    ? { kind: "start", save: selected }
                    : { kind: "stop" },
                );
              }}
            >
              {mutation.isPending ? (
                <LoaderCircle size={16} className="spin" />
              ) : (
                <Play size={14} />
              )}
              {dialog === "start" ? "Start server" : "Save & stop"}
            </button>
          </div>
        </Modal>
      )}
    </>
  );
}
