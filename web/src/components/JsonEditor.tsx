export function EditorMode({
  mode,
  onChange,
  busy,
  invalid = false,
}: {
  mode: "form" | "raw";
  onChange: (mode: "form" | "raw") => void;
  busy: boolean;
  invalid?: boolean;
}) {
  return (
    <div className="tab-buttons" aria-label="Settings editor">
      {(["form", "raw"] as const).map((value) => (
        <button
          key={value}
          type="button"
          className={mode === value ? "button primary" : "button secondary"}
          aria-pressed={mode === value}
          disabled={busy || (value === "form" && invalid)}
          onClick={() => {
            onChange(value);
          }}
        >
          {value === "form" ? "Form" : "Raw JSON"}
        </button>
      ))}
    </div>
  );
}

export function JsonEditor({
  filename,
  label,
  help,
  value,
  onChange,
  disabled,
  rows = 12,
}: {
  filename: string;
  label: string;
  help: string;
  value: string;
  onChange: (value: string) => void;
  disabled: boolean;
  rows?: number;
}) {
  return (
    <section className="panel">
      <div className="section-heading">
        <div>
          <h2>{filename}</h2>
          <p>{help}</p>
        </div>
      </div>
      <div className="form-fields">
        <label className="field">
          {label}
          <textarea
            className="raw-editor"
            spellCheck={false}
            autoComplete="off"
            rows={rows}
            value={value}
            disabled={disabled}
            onChange={(event) => {
              onChange(event.target.value);
            }}
          />
        </label>
      </div>
    </section>
  );
}
