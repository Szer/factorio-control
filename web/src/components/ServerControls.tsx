import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { LoaderCircle, Play, Square, OctagonX } from "lucide-react";
import { useAccess } from "../authApi";
import { operations } from "../operationsApi";
import { api, errorMessage } from "../api";
import type { Status } from "../api";
import { Modal, Notice } from "./ui";

export default function ServerControls({ status }: { status: Status }) {
  const { canOperate } = useAccess();
  const [dialog, setDialog] = useState<"start" | "stop" | "force" | null>(null);
  const selection = useQuery({
    queryKey: ["selection"],
    queryFn: api.selection,
  });
  const saves = useQuery({
    queryKey: ["saves"],
    queryFn: api.saves,
    enabled: selection.data?.load_latest === true,
  });
  const client = useQueryClient();
  const mutation = useMutation({
    mutationFn: (
      action:
        { kind: "start"; save?: string } | { kind: "stop" } | { kind: "force" },
    ) =>
      action.kind === "start"
        ? api.start(action.save)
        : action.kind === "force"
          ? operations.forceStop()
          : api.stop(),
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
    (selection.data?.load_latest
      ? (saves.data?.length ?? 0) > 0
      : selection.data?.save != null) &&
    status.state !== "stopping";
  if (!canOperate) return null;
  return (
    <>
      <div className="server-actions">
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
                : !canStart
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
        {(active || status.state === "stopping") && (
          <button
            className="icon-button danger-text"
            aria-label="Force stop server"
            title="Force stop server"
            onClick={() => {
              mutation.reset();
              setDialog("force");
            }}
          >
            <OctagonX size={19} />
          </button>
        )}
      </div>
      {dialog && (
        <Modal
          busy={mutation.isPending}
          title={
            dialog === "start"
              ? "Start server?"
              : dialog === "force"
                ? "Force stop server?"
                : "Stop server?"
          }
          close={() => {
            if (!mutation.isPending) setDialog(null);
          }}
        >
          <p className="modal-description">
            {dialog === "start" ? (
              <>
                Load{" "}
                <strong>
                  {selection.data?.load_latest
                    ? "the newest save, including autosaves"
                    : selection.data?.save}
                </strong>{" "}
                with the saved server configuration and enabled mods. Change the
                startup save on the{" "}
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
            ) : dialog === "force" ? (
              "Stops the game immediately without saving. Progress since the last save may be lost. Use only if graceful shutdown is stuck."
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
              className={`button ${dialog !== "start" ? "danger" : "primary"}`}
              disabled={mutation.isPending || (dialog === "start" && !canStart)}
              onClick={() => {
                if (dialog === "stop") mutation.mutate({ kind: "stop" });
                else if (dialog === "force") mutation.mutate({ kind: "force" });
                else if (selection.data?.load_latest)
                  mutation.mutate({ kind: "start" });
                else if (selection.data?.save)
                  mutation.mutate({ kind: "start", save: selection.data.save });
              }}
            >
              {mutation.isPending
                ? "Working…"
                : dialog === "start"
                  ? "Start server"
                  : dialog === "force"
                    ? "Force stop without saving"
                    : "Save & stop"}
            </button>
          </div>
        </Modal>
      )}
      {selection.isError && <Notice>{errorMessage(selection.error)}</Notice>}
    </>
  );
}
