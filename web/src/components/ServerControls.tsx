import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { LoaderCircle, Play, Square } from "lucide-react";
import { api, errorMessage } from "../api";
import type { Status } from "../api";
import { Modal, Notice } from "./ui";

export default function ServerControls({ status }: { status: Status }) {
  const [dialog, setDialog] = useState<"start" | "stop" | null>(null);
  const selection = useQuery({
    queryKey: ["selection"],
    queryFn: api.selection,
  });
  const client = useQueryClient();
  const mutation = useMutation({
    mutationFn: (action: { kind: "start"; save: string } | { kind: "stop" }) =>
      action.kind === "start" ? api.start(action.save) : api.stop(),
    onSuccess: async (result) => {
      client.setQueryData(["status"], result);
      setDialog(null);
      await client.invalidateQueries({ queryKey: ["telemetry"] });
    },
  });
  const active =
    status.state === "running" ||
    status.state === "paused" ||
    status.state === "starting";
  const canStart =
    status.binary_available &&
    selection.data?.save != null &&
    status.state !== "stopping";
  return (
    <>
      {active ? (
        <button
          className="button secondary"
          onClick={() => {
            mutation.reset();
            setDialog("stop");
          }}
        >
          <Square size={14} />
          Stop server
        </button>
      ) : (
        <button
          className="button primary"
          disabled={!canStart}
          title={
            !status.binary_available
              ? "Factorio is not configured"
              : !selection.data?.save
                ? "Select a save on the Saves page"
                : undefined
          }
          onClick={() => {
            mutation.reset();
            setDialog("start");
          }}
        >
          {status.state === "stopping" ? (
            <LoaderCircle size={16} className="spin" />
          ) : (
            <Play size={15} fill="currentColor" />
          )}
          {status.state === "stopping" ? "Stopping…" : "Start server"}
        </button>
      )}
      {dialog && (
        <Modal
          title={dialog === "start" ? "Start server?" : "Stop server?"}
          close={() => {
            if (!mutation.isPending) setDialog(null);
          }}
        >
          <p className="modal-description">
            {dialog === "start" ? (
              <>
                Load <strong>{selection.data?.save}</strong> with the saved
                server configuration and enabled mods. Change the startup save
                on the{" "}
                <a
                  href="#saves"
                  onClick={() => {
                    setDialog(null);
                  }}
                >
                  Saves page
                </a>
                .
              </>
            ) : (
              "Connected players will be disconnected. Factorio will save and shut down gracefully."
            )}
          </p>
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
                mutation.isPending ||
                (dialog === "start" && !selection.data?.save)
              }
              onClick={() => {
                if (dialog === "stop") mutation.mutate({ kind: "stop" });
                else if (selection.data?.save)
                  mutation.mutate({ kind: "start", save: selection.data.save });
              }}
            >
              {mutation.isPending
                ? "Working…"
                : dialog === "start"
                  ? "Start server"
                  : "Save & stop"}
            </button>
          </div>
        </Modal>
      )}
      {selection.isError && <Notice>{errorMessage(selection.error)}</Notice>}
    </>
  );
}
