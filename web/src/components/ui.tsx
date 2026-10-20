import { AlertCircle, Check, LoaderCircle, X } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useRef } from "react";
import type { Status } from "../api";

export function Notice({
  children,
  success = false,
}: {
  children: ReactNode;
  success?: boolean;
}) {
  return (
    <div
      className={`notice ${success ? "success" : "error"}`}
      role={success ? "status" : "alert"}
    >
      {success ? <Check size={17} /> : <AlertCircle size={17} />}
      <span>{children}</span>
    </div>
  );
}
export function Loading() {
  return (
    <div className="loading" role="status">
      <LoaderCircle className="spin" size={22} /> Connecting to your control
      room…
    </div>
  );
}
export function StatusBadge({ state }: { state: Status["state"] }) {
  const labels: Record<Status["state"], string> = {
    stopped: "Stopped",
    starting: "Starting",
    running: "Running",
    stopping: "Stopping",
    failed: "Needs attention",
  };
  return (
    <span className={`status-badge ${state}`}>
      <span className="status-dot" />
      {labels[state]}
    </span>
  );
}
export function EmptyState({
  icon,
  title,
  children,
  action,
}: {
  icon: ReactNode;
  title: string;
  children: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="empty-state">
      <div className="empty-icon">{icon}</div>
      <h3>{title}</h3>
      <p>{children}</p>
      {action}
    </div>
  );
}
export function Modal({
  title,
  children,
  close,
}: {
  title: string;
  children: ReactNode;
  close: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const element = dialog.current;
    element?.showModal();
    return () => {
      element?.close();
    };
  }, []);
  return (
    <dialog
      ref={dialog}
      className="modal"
      onCancel={(event) => {
        event.preventDefault();
        close();
      }}
      aria-labelledby="dialog-title"
    >
      <header>
        <h2 id="dialog-title">{title}</h2>
        <button
          className="icon-button"
          aria-label="Close dialog"
          onClick={close}
        >
          <X size={20} />
        </button>
      </header>
      {children}
    </dialog>
  );
}
